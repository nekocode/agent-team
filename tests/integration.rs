// ==================== 集成测试 ====================
// 每个测试启动独立 session + mock agent

use std::collections::HashMap;
use std::time::Duration;

use agent_team::config::{AgentTypeConfig, AutoApprovePolicy, TeamConfig};
use agent_team::protocol::messages::{SessionRequest, SessionResponse};
use agent_team::protocol::transport::{JsonLineReader, JsonLineWriter};
use tokio::net::UnixStream;

fn test_config(socket_dir: std::path::PathBuf) -> TeamConfig {
    let mock_agent_bin = env!("CARGO_BIN_EXE_mock-agent");

    let mut agent_types = HashMap::new();
    agent_types.insert(
        "mock".to_string(),
        AgentTypeConfig {
            command: mock_agent_bin.to_string(),
            default_args: vec![],
        },
    );

    TeamConfig {
        auto_approve: AutoApprovePolicy::Never,
        output_buffer_size: 100,
        agent_types,
        default_cwd: std::env::temp_dir(),
        socket_dir,
    }
}

async fn send_recv(
    sock_path: &std::path::Path,
    req: SessionRequest,
) -> SessionResponse {
    let stream = UnixStream::connect(sock_path).await.unwrap();
    let (read, write) = stream.into_split();
    let mut writer = JsonLineWriter::new(write);
    let mut reader = JsonLineReader::new(read);
    writer.write(&req).await.unwrap();
    reader.read::<SessionResponse>().await.unwrap().unwrap()
}

/// 发送 Prompt 并等待 agent 回到 idle（fire-and-forget 语义）
async fn send_prompt_and_wait(
    sock_path: &std::path::Path,
    text: &str,
    min_prompts: u64,
) {
    let resp = send_recv(
        sock_path,
        SessionRequest::Prompt {
            text: text.into(),
            files: vec![],
        },
    )
    .await;
    assert!(matches!(resp, SessionResponse::Ok { .. }));

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let resp = send_recv(sock_path, SessionRequest::GetStatus).await;
        if let SessionResponse::Status { summary } = resp {
            if summary.status == "idle" && summary.prompt_count >= min_prompts {
                return;
            }
        }
    }
    panic!("timed out waiting for prompt completion");
}

// ==================== session 启动 + 状态查询 ====================

#[tokio::test]
async fn session_status() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().to_path_buf());
    let sock_path = config.session_socket("test-1");

    let local = tokio::task::LocalSet::new();
    let session_config = config.clone();
    let session_handle = local.spawn_local(async move {
        agent_team::session::server::run(
            "test-1".into(),
            "mock".into(),
            session_config,
            vec![],
            std::env::temp_dir(),
        )
        .await
    });

    local
        .run_until(async {
            tokio::time::sleep(Duration::from_millis(200)).await;

            let resp = send_recv(&sock_path, SessionRequest::GetStatus).await;
            match &resp {
                SessionResponse::Status { summary } => {
                    assert_eq!(summary.name, "test-1");
                    assert_eq!(summary.agent_type, "mock");
                    assert_eq!(summary.status, "idle");
                    assert_eq!(summary.prompt_count, 0);
                }
                other => panic!("expected Status, got: {:?}", other),
            }

            let resp = send_recv(&sock_path, SessionRequest::Shutdown).await;
            assert!(matches!(resp, SessionResponse::Ok { .. }));

            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(session_handle.is_finished());
        })
        .await;
}

// ==================== prompt → output ====================

#[tokio::test]
async fn prompt_and_output() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().to_path_buf());
    let sock_path = config.session_socket("prompter");

    let local = tokio::task::LocalSet::new();
    let session_config = config.clone();
    let _handle = local.spawn_local(async move {
        agent_team::session::server::run(
            "prompter".into(),
            "mock".into(),
            session_config,
            vec![],
            std::env::temp_dir(),
        )
        .await
    });

    local
        .run_until(async {
            tokio::time::sleep(Duration::from_millis(200)).await;

            // 1. prompt → fire-and-forget，等待完成
            send_prompt_and_wait(&sock_path, "hello from test", 1).await;

            // 2. status — prompt_count=1, idle
            let resp =
                send_recv(&sock_path, SessionRequest::GetStatus).await;
            match &resp {
                SessionResponse::Status { summary } => {
                    assert_eq!(summary.status, "idle");
                    assert_eq!(summary.prompt_count, 1);
                }
                other => panic!("expected Status, got: {:?}", other),
            }

            // 3. output（最近一条消息）— 应有 PromptResponse
            let resp = send_recv(
                &sock_path,
                SessionRequest::GetOutput { last: 1, agent_only: false },
            )
            .await;
            match &resp {
                SessionResponse::Output { entries, .. } => {
                    assert!(!entries.is_empty(), "expected output");
                    let has_pr = entries.iter().any(|e| {
                        matches!(
                            e.update_type,
                            agent_team::protocol::messages::OutputType::PromptResponse
                        )
                    });
                    assert!(has_pr, "expected PromptResponse entry");
                }
                other => panic!("expected Output, got: {:?}", other),
            }

            // 4. 再 prompt → prompt_count=2
            send_prompt_and_wait(&sock_path, "second prompt", 2).await;

            let resp =
                send_recv(&sock_path, SessionRequest::GetStatus).await;
            match &resp {
                SessionResponse::Status { summary } => {
                    assert_eq!(summary.prompt_count, 2);
                }
                other => panic!("expected Status, got: {:?}", other),
            }

            let resp =
                send_recv(&sock_path, SessionRequest::Shutdown).await;
            assert!(matches!(resp, SessionResponse::Ok { .. }));
        })
        .await;
}

// ==================== cancel（无活跃任务）====================

#[tokio::test]
async fn cancel_no_active_task() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().to_path_buf());
    let sock_path = config.session_socket("canceller");

    let local = tokio::task::LocalSet::new();
    let session_config = config.clone();
    let _handle = local.spawn_local(async move {
        agent_team::session::server::run(
            "canceller".into(),
            "mock".into(),
            session_config,
            vec![],
            std::env::temp_dir(),
        )
        .await
    });

    local
        .run_until(async {
            tokio::time::sleep(Duration::from_millis(200)).await;

            // idle 状态 cancel → Ok（连接存在，发 cancel 通知是 no-op）
            let resp =
                send_recv(&sock_path, SessionRequest::Cancel).await;
            assert!(matches!(resp, SessionResponse::Ok { .. }));

            let resp =
                send_recv(&sock_path, SessionRequest::Shutdown).await;
            assert!(matches!(resp, SessionResponse::Ok { .. }));
        })
        .await;
}

// ==================== restart ====================

#[tokio::test]
async fn restart_agent() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().to_path_buf());
    let sock_path = config.session_socket("worker");

    let local = tokio::task::LocalSet::new();
    let session_config = config.clone();
    let _handle = local.spawn_local(async move {
        agent_team::session::server::run(
            "worker".into(),
            "mock".into(),
            session_config,
            vec![],
            std::env::temp_dir(),
        )
        .await
    });

    local
        .run_until(async {
            tokio::time::sleep(Duration::from_millis(200)).await;

            // prompt 一次
            send_prompt_and_wait(&sock_path, "hello", 1).await;

            // restart
            let resp =
                send_recv(&sock_path, SessionRequest::Restart).await;
            match &resp {
                SessionResponse::Ok { message } => {
                    assert!(message.contains("restarted"));
                }
                other => panic!("expected Ok, got: {:?}", other),
            }

            // restart 后：idle, prompt_count=0
            let resp =
                send_recv(&sock_path, SessionRequest::GetStatus).await;
            match &resp {
                SessionResponse::Status { summary } => {
                    assert_eq!(summary.name, "worker");
                    assert_eq!(summary.status, "idle");
                    assert_eq!(summary.prompt_count, 0);
                }
                other => panic!("expected Status, got: {:?}", other),
            }

            let resp =
                send_recv(&sock_path, SessionRequest::Shutdown).await;
            assert!(matches!(resp, SessionResponse::Ok { .. }));
        })
        .await;
}

// ==================== graceful shutdown ====================

#[tokio::test]
async fn graceful_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().to_path_buf());
    let sock_path = config.session_socket("alpha");

    let local = tokio::task::LocalSet::new();
    let session_config = config.clone();
    let session_handle = local.spawn_local(async move {
        agent_team::session::server::run(
            "alpha".into(),
            "mock".into(),
            session_config,
            vec![],
            std::env::temp_dir(),
        )
        .await
    });

    local
        .run_until(async {
            tokio::time::sleep(Duration::from_millis(200)).await;

            // prompt 确认 session 活跃
            send_prompt_and_wait(&sock_path, "hello", 1).await;

            // shutdown
            let resp =
                send_recv(&sock_path, SessionRequest::Shutdown).await;
            match &resp {
                SessionResponse::Ok { message } => {
                    assert!(message.contains("shutting down"));
                }
                other => panic!("expected Ok, got: {:?}", other),
            }

            // 等待 session 退出
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(
                session_handle.is_finished(),
                "session should have exited"
            );

            // socket 文件已清理
            assert!(
                !sock_path.exists(),
                "socket file should be cleaned up"
            );
        })
        .await;
}

// ==================== output last round ====================

#[tokio::test]
async fn output_last_round() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path().to_path_buf());
    let sock_path = config.session_socket("laster");

    let local = tokio::task::LocalSet::new();
    let session_config = config.clone();
    let _handle = local.spawn_local(async move {
        agent_team::session::server::run(
            "laster".into(),
            "mock".into(),
            session_config,
            vec![],
            std::env::temp_dir(),
        )
        .await
    });

    local
        .run_until(async {
            tokio::time::sleep(Duration::from_millis(200)).await;

            // 无 prompt → 空
            let resp = send_recv(
                &sock_path,
                SessionRequest::GetOutput { last: 1, agent_only: false },
            )
            .await;
            match &resp {
                SessionResponse::Output { entries, .. } => {
                    assert!(entries.is_empty());
                }
                other => panic!("expected Output, got: {:?}", other),
            }

            // prompt
            send_prompt_and_wait(&sock_path, "test", 1).await;

            // last=1 → 最后一条消息（agent 回复），不含 UserPrompt
            let resp = send_recv(
                &sock_path,
                SessionRequest::GetOutput { last: 1, agent_only: false },
            )
            .await;
            match &resp {
                SessionResponse::Output { entries, .. } => {
                    assert!(!entries.is_empty());
                    // last=1 只取最后一条消息块（agent 回复）
                    assert!(!matches!(
                        entries[0].update_type,
                        agent_team::protocol::messages::OutputType::UserPrompt
                    ));
                }
                other => panic!("expected Output, got: {:?}", other),
            }

            // last=2 → 包含 UserPrompt + agent 回复
            let resp = send_recv(
                &sock_path,
                SessionRequest::GetOutput { last: 2, agent_only: false },
            )
            .await;
            match &resp {
                SessionResponse::Output { entries, .. } => {
                    assert!(entries.iter().any(|e| matches!(
                        e.update_type,
                        agent_team::protocol::messages::OutputType::UserPrompt
                    )));
                }
                other => panic!("expected Output, got: {:?}", other),
            }

            let resp =
                send_recv(&sock_path, SessionRequest::Shutdown).await;
            assert!(matches!(resp, SessionResponse::Ok { .. }));
        })
        .await;
}
