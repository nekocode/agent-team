use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::{self as acp, Agent};
use anyhow::{Context, Result};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(not(unix))]
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::acp_client::team_client::PermissionDecision;
use crate::config::TeamConfig;
use crate::session::agent::{spawn_agent, AgentHandle, AgentStatus};
use crate::protocol::messages::{OutputEntry, OutputType, SessionRequest, SessionResponse};
use crate::protocol::transport::{JsonLineReader, JsonLineWriter};

const SHUTDOWN_TIMEOUT_SECS: u64 = 3;

#[cfg(unix)]
type SessionStream = tokio::net::UnixStream;
#[cfg(not(unix))]
type SessionStream = tokio::net::TcpStream;

// ==================== stdout 事件 ====================

pub(crate) enum Event {
    /// AI 输出（来自 ACP 回调）
    Output(OutputEntry),
    /// 系统生命周期事件
    Info { tag: &'static str, message: String },
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

// ==================== session 入口 ====================

pub async fn run(
    name: String,
    agent_type: String,
    config: TeamConfig,
    extra_args: Vec<String>,
    cwd: PathBuf,
) -> Result<()> {
    let sock_path = config.session_socket(&name);
    config.ensure_socket_dir()?;
    cleanup_socket(&sock_path);

    // 先 bind listener，让 socket 文件尽早可见
    #[cfg(unix)]
    let listener = UnixListener::bind(&sock_path)
        .with_context(|| format!("Failed to bind: {}", sock_path.display()))?;

    #[cfg(not(unix))]
    let listener = {
        let l = TcpListener::bind("127.0.0.1:0")
            .await
            .context("Failed to bind TCP")?;
        let port = l.local_addr()?.port();
        std::fs::write(&sock_path, port.to_string())
            .with_context(|| format!("Failed to write port file: {}", sock_path.display()))?;
        l
    };

    // 事件通道
    let (event_tx, event_rx) = mpsc::unbounded_channel::<Event>();
    let (output_tx, output_rx) = mpsc::unbounded_channel::<OutputEntry>();

    // 桥接：TeamClient output → event 流
    let bridge_tx = event_tx.clone();
    tokio::task::spawn_local(bridge_output(output_rx, bridge_tx));

    // stdout 打印
    tokio::task::spawn_local(print_events(event_rx));

    event_tx
        .send(Event::Info {
            tag: "started",
            message: format!(
                "Listening on {} (type: {})",
                sock_path.display(),
                agent_type,
            ),
        })
        .ok();

    // spawn agent
    let tc = config
        .agent_types
        .get(&agent_type)
        .with_context(|| format!("Unknown agent type: {}", agent_type))?
        .clone();

    event_tx
        .send(Event::Info {
            tag: "spawned",
            message: "Agent process started".into(),
        })
        .ok();

    let handle = spawn_agent(
        name.clone(),
        agent_type,
        tc,
        cwd,
        extra_args,
        config.output_buffer_size,
        config.auto_approve.clone(),
        Some(output_tx),
    )
    .await?;

    event_tx
        .send(Event::Info {
            tag: "initialized",
            message: "ACP protocol ready".into(),
        })
        .ok();
    event_tx
        .send(Event::Info {
            tag: "idle",
            message: "Ready".into(),
        })
        .ok();

    let handle = Rc::new(RefCell::new(handle));
    let config = Rc::new(config);
    let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<()>();

    // 主循环
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result.context("Accept failed")?;
                let h = Rc::clone(&handle);
                let c = Rc::clone(&config);
                let etx = event_tx.clone();
                let stx = shutdown_tx.clone();
                tokio::task::spawn_local(async move {
                    handle_connection(stream, h, c, etx, stx).await;
                });
            }
            _ = shutdown_rx.recv() => {
                event_tx.send(Event::Info {
                    tag: "shutdown",
                    message: "Remote request".into(),
                }).ok();
                break;
            }
            _ = signal_shutdown() => {
                event_tx.send(Event::Info {
                    tag: "shutdown",
                    message: "Signal received".into(),
                }).ok();
                break;
            }
        }
    }

    // 优雅关闭（take 销毁连接）
    let (conn, sid, mut child) = {
        let mut h = handle.borrow_mut();
        h.set_status(AgentStatus::Stopping);
        (h.acp_conn.take(), h.session_id.take(), h.child.take())
    };
    if let (Some(conn), Some(sid)) = (conn, sid) {
        let _ = conn.cancel(acp::CancelNotification::new(sid)).await;
    }
    if let Some(ref mut child) = child {
        shutdown_child(child, &event_tx).await;
    }

    cleanup_socket(&sock_path);
    event_tx
        .send(Event::Info {
            tag: "stopped",
            message: "Socket cleaned".into(),
        })
        .ok();
    Ok(())
}

// ==================== 连接处理 ====================

async fn handle_connection(
    stream: SessionStream,
    handle: Rc<RefCell<AgentHandle>>,
    config: Rc<TeamConfig>,
    event_tx: mpsc::UnboundedSender<Event>,
    shutdown_tx: mpsc::UnboundedSender<()>,
) {
    let (read, write) = stream.into_split();
    let mut reader = JsonLineReader::new(read);
    let mut writer = JsonLineWriter::new(write);

    loop {
        let req = match reader.read::<SessionRequest>().await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(e) => {
                event_tx
                    .send(Event::Info {
                        tag: "error",
                        message: format!("Read error: {}", e),
                    })
                    .ok();
                break;
            }
        };

        let is_shutdown = matches!(req, SessionRequest::Shutdown);
        // GetStatus 是轮询心跳；Prompt 由 UserPrompt 事件覆盖
        if !matches!(req, SessionRequest::GetStatus | SessionRequest::GetOutput { .. } | SessionRequest::Prompt { .. }) {
            event_tx
                .send(Event::Info {
                    tag: "request",
                    message: req.label().to_string(),
                })
                .ok();
        }

        let resp = handle_request(&handle, &config, req, &event_tx).await;

        if writer.write(&resp).await.is_err() {
            event_tx
                .send(Event::Info {
                    tag: "disconnected",
                    message: "Client disconnected".into(),
                })
                .ok();
            break;
        }

        if is_shutdown {
            shutdown_tx.send(()).ok();
            break;
        }
    }
}

// ==================== 请求分发 ====================

pub(crate) async fn handle_request(
    handle: &Rc<RefCell<AgentHandle>>,
    config: &TeamConfig,
    req: SessionRequest,
    event_tx: &mpsc::UnboundedSender<Event>,
) -> SessionResponse {
    match req {
        SessionRequest::GetStatus => {
            let h = handle.borrow();
            SessionResponse::Status {
                summary: h.to_summary(),
            }
        }

        SessionRequest::Prompt { text, files } => {
            // 忙碌时自动取消当前任务
            if let Err(resp) = cancel_if_busy(handle, event_tx).await {
                return resp;
            }
            // 前置校验
            let h = handle.borrow();
            if h.get_status() == AgentStatus::Running {
                return SessionResponse::Error { message: "Agent is already running".into() };
            }
            if h.acp_conn.is_none() || h.session_id.is_none() {
                return no_session();
            }
            drop(h);
            // 提交 prompt
            submit_prompt(handle, event_tx, text, files).await
        }

        SessionRequest::GetOutput { last, agent_only } => {
            let name = handle.borrow().name.clone();
            let buf = handle.borrow().output_buffer.clone();
            let mut entries = buf.lock().await.last_msgs(last);
            if agent_only {
                entries.retain(|e| !matches!(e.update_type, OutputType::UserPrompt));
            }
            SessionResponse::Output { agent_name: name, entries }
        }

        SessionRequest::Cancel => {
            let (conn, sid) = clone_conn(handle);
            let Some((conn, sid)) = conn.zip(sid) else {
                return no_session();
            };
            let _ = conn.cancel(acp::CancelNotification::new(sid)).await;
            event_tx.send(Event::Info { tag: "cancelled", message: "Cancel sent".into() }).ok();
            SessionResponse::Ok { message: "Cancel sent".into() }
        }

        SessionRequest::ApprovePermission => {
            handle_permission(handle, event_tx, true).await
        }

        SessionRequest::DenyPermission => {
            handle_permission(handle, event_tx, false).await
        }

        SessionRequest::Restart => {
            // 1. 关闭旧 agent
            let (old_conn, old_sid, old_child, agent_type, cwd, extra_args) = {
                let mut h = handle.borrow_mut();
                h.set_status(AgentStatus::Stopping);
                (
                    h.acp_conn.take(),
                    h.session_id.take(),
                    h.child.take(),
                    h.agent_type.clone(),
                    h.cwd.clone(),
                    h.extra_args.clone(),
                )
            };

            if let (Some(conn), Some(sid)) = (old_conn, old_sid) {
                let _ = conn.cancel(acp::CancelNotification::new(sid)).await;
            }
            if let Some(mut child) = old_child {
                shutdown_child(&mut child, event_tx).await;
            }

            // 2. 新 output 桥接
            let (new_output_tx, new_output_rx) =
                mpsc::unbounded_channel::<OutputEntry>();
            let bridge_tx = event_tx.clone();
            tokio::task::spawn_local(bridge_output(new_output_rx, bridge_tx));

            // 3. 重新 spawn
            let name = handle.borrow().name.clone();
            let tc = match config.agent_types.get(&agent_type) {
                Some(tc) => tc.clone(),
                None => {
                    handle.borrow().set_status(AgentStatus::Error(
                        format!("Unknown agent type: {}", agent_type),
                    ));
                    return SessionResponse::Error {
                        message: format!("Unknown agent type: {}", agent_type),
                    };
                }
            };

            match spawn_agent(
                name,
                agent_type,
                tc,
                cwd,
                extra_args,
                config.output_buffer_size,
                config.auto_approve.clone(),
                Some(new_output_tx),
            )
            .await
            {
                Ok(new_handle) => {
                    *handle.borrow_mut() = new_handle;
                    event_tx
                        .send(Event::Info {
                            tag: "restarted",
                            message: "Agent restarted, idle".into(),
                        })
                        .ok();
                    SessionResponse::Ok {
                        message: "Agent restarted".into(),
                    }
                }
                Err(e) => {
                    // S2: Restart 失败 → 状态标记为 Error，而非停留在 Stopping
                    handle.borrow().set_status(AgentStatus::Error(format!("{:#}", e)));
                    SessionResponse::Error {
                        message: format!("Restart failed: {:#}", e),
                    }
                }
            }
        }

        SessionRequest::Shutdown => SessionResponse::Ok {
            message: "Session shutting down".into(),
        },

        SessionRequest::SetMode { mode } => {
            let msg = format!("Mode: {}", mode);
            acp_call(handle, event_tx, "mode", &msg, |conn, sid| {
                Box::pin(async move {
                    conn.set_session_mode(acp::SetSessionModeRequest::new(sid, mode)).await
                })
            }).await
        }

        SessionRequest::SetConfig { key, value } => {
            let msg = format!("Config: {} = {}", key, value);
            acp_call(handle, event_tx, "config", &msg, |conn, sid| {
                Box::pin(async move {
                    conn.set_session_config_option(
                        acp::SetSessionConfigOptionRequest::new(sid, key, value),
                    ).await
                })
            }).await
        }
    }
}

// ==================== prompt 辅助 ====================

/// 忙碌时取消当前任务，等待 settle（5s 超时）
async fn cancel_if_busy(
    handle: &Rc<RefCell<AgentHandle>>,
    event_tx: &mpsc::UnboundedSender<Event>,
) -> Result<(), SessionResponse> {
    let cur_status = handle.borrow().get_status();
    if !matches!(cur_status, AgentStatus::Running | AgentStatus::WaitingPermission) {
        return Ok(());
    }

    let (conn, sid) = clone_conn(handle);
    if let (Some(conn), Some(sid)) = (conn, sid) {
        let _ = conn.cancel(acp::CancelNotification::new(sid)).await;
    }

    let queue = handle.borrow().pending_permissions.clone();
    drain_permissions(&queue).await;
    event_tx.send(Event::Info { tag: "cancelled", message: "Auto-cancelled for new prompt".into() }).ok();

    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drain_permissions(&queue).await;
        let s = handle.borrow().get_status();
        if matches!(s, AgentStatus::Idle | AgentStatus::Error(_)) {
            return Ok(());
        }
    }
    Err(SessionResponse::Error { message: "Agent still busy after cancel".into() })
}

async fn drain_permissions(
    queue: &Arc<tokio::sync::Mutex<std::collections::VecDeque<crate::acp_client::team_client::PendingPermission>>>,
) {
    let mut q: tokio::sync::MutexGuard<'_, _> = queue.lock().await;
    while let Some(perm) = q.pop_front() {
        let _ = perm.response_tx.send(PermissionDecision::Deny);
    }
}

/// 记录 prompt + spawn 后台 do_prompt
async fn submit_prompt(
    handle: &Rc<RefCell<AgentHandle>>,
    event_tx: &mpsc::UnboundedSender<Event>,
    text: String,
    files: Vec<crate::protocol::messages::FileAttachment>,
) -> SessionResponse {
    let user_entry = OutputEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        update_type: OutputType::UserPrompt,
        content: text.clone(),
    };
    let buf = handle.borrow().output_buffer.clone();
    buf.lock().await.push(user_entry.clone());
    event_tx.send(Event::Output(user_entry)).ok();

    let mut blocks: Vec<acp::ContentBlock> = vec![text.into()];
    for f in &files {
        blocks.push(format!("--- {} ---\n{}", f.path.display(), f.content).into());
    }
    let h = Rc::clone(handle);
    let etx = event_tx.clone();
    tokio::task::spawn_local(async move { do_prompt(&h, blocks, &etx).await; });
    SessionResponse::Ok { message: "Prompt submitted".into() }
}

/// S6: 通用 ACP 调用（SetMode / SetConfig 共享骨架）
async fn acp_call<F, T>(
    handle: &Rc<RefCell<AgentHandle>>,
    event_tx: &mpsc::UnboundedSender<Event>,
    tag: &'static str,
    success_msg: &str,
    call: F,
) -> SessionResponse
where
    F: FnOnce(Rc<acp::ClientSideConnection>, acp::SessionId) -> std::pin::Pin<Box<dyn std::future::Future<Output = acp::Result<T>>>>,
{
    let (conn, sid) = clone_conn(handle);
    let Some((conn, sid)) = conn.zip(sid) else {
        return no_session();
    };
    match call(conn, sid).await {
        Ok(_) => {
            event_tx.send(Event::Info { tag, message: success_msg.to_string() }).ok();
            SessionResponse::Ok { message: success_msg.to_string() }
        }
        Err(e) => SessionResponse::Error { message: format!("{}", e) },
    }
}

// ==================== prompt 核心 ====================

async fn do_prompt(
    handle: &Rc<RefCell<AgentHandle>>,
    prompt_blocks: Vec<acp::ContentBlock>,
    event_tx: &mpsc::UnboundedSender<Event>,
) {
    let (conn, sid, buf) = {
        let mut h = handle.borrow_mut();
        // S3: 优雅检查，避免与 Restart 交错时 panic
        let Some(conn) = h.acp_conn.as_ref().map(Rc::clone) else {
            h.set_status(AgentStatus::Error("No ACP connection".into()));
            event_tx.send(Event::Info { tag: "error", message: "No ACP connection in do_prompt".into() }).ok();
            return;
        };
        let Some(sid) = h.session_id.clone() else {
            h.set_status(AgentStatus::Error("No session ID".into()));
            event_tx.send(Event::Info { tag: "error", message: "No session ID in do_prompt".into() }).ok();
            return;
        };
        h.set_status(AgentStatus::Running);
        h.prompt_count += 1;
        (conn, sid, Arc::clone(&h.output_buffer))
    };
    event_tx.send(Event::Info { tag: "running", message: "Processing".into() }).ok();

    let result = conn.prompt(acp::PromptRequest::new(sid, prompt_blocks)).await;
    match result {
        Ok(resp) => {
            buf.lock().await.push(OutputEntry {
                timestamp: chrono::Utc::now().to_rfc3339(),
                update_type: OutputType::PromptResponse,
                content: format!("{:?}", resp.stop_reason),
            });
            let msg = format!("{:?}", resp.stop_reason);
            event_tx.send(Event::Info { tag: "done", message: msg }).ok();
            handle.borrow().set_status(AgentStatus::Idle);
        }
        Err(e) => {
            handle.borrow().set_status(AgentStatus::Error(format!("{}", e)));
            event_tx.send(Event::Info { tag: "error", message: format!("Prompt failed: {}", e) }).ok();
            return;
        }
    }
    event_tx.send(Event::Info { tag: "idle", message: "Ready".into() }).ok();
}

// ==================== 连接辅助 ====================

async fn handle_permission(
    handle: &Rc<RefCell<AgentHandle>>,
    event_tx: &mpsc::UnboundedSender<Event>,
    approve: bool,
) -> SessionResponse {
    let queue = handle.borrow().pending_permissions.clone();
    let mut q = queue.lock().await;
    let Some(perm) = q.pop_front() else {
        return SessionResponse::Error {
            message: "No pending permissions".into(),
        };
    };
    let info = perm.tool_info.clone();
    let (decision, tag) = if approve {
        (PermissionDecision::Approve, "approved")
    } else {
        (PermissionDecision::Deny, "denied")
    };
    let _ = perm.response_tx.send(decision);
    event_tx.send(Event::Info { tag, message: info.clone() }).ok();
    SessionResponse::Ok {
        message: format!("{}: {}", if approve { "Approved" } else { "Denied" }, info),
    }
}

fn clone_conn(
    handle: &Rc<RefCell<AgentHandle>>,
) -> (Option<Rc<acp::ClientSideConnection>>, Option<acp::SessionId>) {
    let h = handle.borrow();
    (h.acp_conn.as_ref().map(Rc::clone), h.session_id.clone())
}

pub(crate) fn no_session() -> SessionResponse {
    SessionResponse::Error {
        message: "No active session".into(),
    }
}

// ==================== stdout 打印 ====================

async fn bridge_output(
    mut rx: mpsc::UnboundedReceiver<OutputEntry>,
    tx: mpsc::UnboundedSender<Event>,
) {
    while let Some(entry) = rx.recv().await {
        tx.send(Event::Output(entry)).ok();
    }
}

async fn print_events(mut rx: mpsc::UnboundedReceiver<Event>) {
    use std::io::Write;
    let mut needs_newline = false;
    let mut in_message = false;

    while let Some(event) = rx.recv().await {
        match event {
            Event::Output(entry) => match entry.update_type {
                OutputType::UserPrompt => {
                    in_message = false;
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    println!("{} [request] Prompt:\n{}", now(), entry.content.trim());
                }
                OutputType::AgentMessage | OutputType::AgentThought => {
                    // 新消息段的第一个 chunk，去掉前导空白
                    let text = if !in_message {
                        entry.content.trim_start()
                    } else {
                        &entry.content
                    };
                    if !text.is_empty() {
                        print!("{}", text);
                        std::io::stdout().flush().ok();
                        needs_newline = !text.ends_with('\n');
                        in_message = true;
                    }
                }
                _ => {
                    in_message = false;
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    println!(
                        "{} [{}] {}",
                        now(),
                        entry.update_type.label(),
                        entry.content,
                    );
                }
            },
            Event::Info { tag, message } => {
                in_message = false;
                if needs_newline {
                    println!();
                    needs_newline = false;
                }
                println!("{} [{}] {}", now(), tag, message);
            }
        }
    }
}

// ==================== 关闭 & 工具 ====================

async fn signal_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("Failed to register SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn shutdown_child(
    child: &mut tokio::process::Child,
    event_tx: &mpsc::UnboundedSender<Event>,
) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.start_kill();
    }

    match tokio::time::timeout(
        Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => {
            event_tx
                .send(Event::Info {
                    tag: "exited",
                    message: format!("Code: {}", status),
                })
                .ok();
        }
        Ok(Err(e)) => {
            event_tx
                .send(Event::Info {
                    tag: "error",
                    message: format!("Wait error: {}", e),
                })
                .ok();
        }
        Err(_) => {
            event_tx
                .send(Event::Info {
                    tag: "exited",
                    message: "Timeout, SIGKILL sent".into(),
                })
                .ok();
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

pub(crate) fn cleanup_socket(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

