use anyhow::{Context, Result};

use crate::config::TeamConfig;
use crate::protocol::messages::{SessionRequest, SessionResponse};
use crate::protocol::transport::{JsonLineReader, JsonLineWriter};

// ==================== 平台类型别名 ====================

#[cfg(unix)]
type ReadHalf = tokio::net::unix::OwnedReadHalf;
#[cfg(unix)]
type WriteHalf = tokio::net::unix::OwnedWriteHalf;

#[cfg(not(unix))]
type ReadHalf = tokio::net::tcp::OwnedReadHalf;
#[cfg(not(unix))]
type WriteHalf = tokio::net::tcp::OwnedWriteHalf;

// ==================== SessionClient ====================

/// 复用连接的 session 客户端
pub struct SessionClient {
    reader: JsonLineReader<ReadHalf>,
    writer: JsonLineWriter<WriteHalf>,
}

impl SessionClient {
    /// 连接到指定 agent 的 session
    pub async fn connect(config: &TeamConfig, name: &str) -> Result<Self> {
        let sock_path = config.session_socket(name);

        #[cfg(unix)]
        let stream = match tokio::net::UnixStream::connect(&sock_path).await {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_file(&sock_path);
                return Err(e).with_context(|| {
                    format!("Cannot connect to agent '{}'. Is it running?", name)
                });
            }
        };

        #[cfg(not(unix))]
        let stream = {
            let port_str = std::fs::read_to_string(&sock_path)
                .with_context(|| format!("Cannot read port file for '{}'", name))?;
            let port: u16 = port_str.trim().parse()
                .with_context(|| format!("Invalid port in {}", sock_path.display()))?;
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = std::fs::remove_file(&sock_path);
                    return Err(e).with_context(|| {
                        format!("Cannot connect to agent '{}'. Is it running?", name)
                    });
                }
            }
        };

        let (read, write) = stream.into_split();
        Ok(Self {
            reader: JsonLineReader::new(read),
            writer: JsonLineWriter::new(write),
        })
    }

    /// 发送请求并读取响应
    pub async fn send(&mut self, req: SessionRequest) -> Result<SessionResponse> {
        self.writer.write(&req).await?;
        self.reader
            .read()
            .await?
            .context("Session closed connection unexpectedly")
    }
}

// ==================== 便捷函数 ====================

/// 单次 connect + send + drop
pub async fn send(
    config: &TeamConfig,
    name: &str,
    req: SessionRequest,
) -> Result<SessionResponse> {
    let mut client = SessionClient::connect(config, name).await?;
    client.send(req).await
}

// ==================== 测试 ====================

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::protocol::messages::AgentSummary;
    use tokio::net::UnixListener;

    /// 启动 mock server，处理 N 个请求后关闭
    async fn mock_server(
        listener: UnixListener,
        responses: Vec<SessionResponse>,
    ) {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = JsonLineReader::new(read);
        let mut writer = JsonLineWriter::new(write);

        for resp in responses {
            let _req: SessionRequest = reader.read().await.unwrap().unwrap();
            writer.write(&resp).await.unwrap();
        }
    }

    fn test_summary(name: &str) -> AgentSummary {
        AgentSummary {
            name: name.into(),
            agent_type: "mock".into(),
            cwd: "/tmp".into(),
            status: "idle".into(),
            uptime: "0m 0s".into(),
            prompt_count: 0,
            pending_permissions: 0,
            agent_info_name: None,
            agent_info_version: None,
        }
    }

    #[tokio::test]
    async fn client_single_send() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let resp = SessionResponse::Status {
            summary: test_summary("a-1"),
        };
        let server = tokio::spawn(mock_server(listener, vec![resp]));

        // 用底层构造（不走 config 路径）
        let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let (read, write) = stream.into_split();
        let mut client = SessionClient {
            reader: JsonLineReader::new(read),
            writer: JsonLineWriter::new(write),
        };

        let result = client.send(SessionRequest::GetStatus).await.unwrap();
        match result {
            SessionResponse::Status { summary } => {
                assert_eq!(summary.name, "a-1");
            }
            _ => panic!("expected Status"),
        }

        server.await.unwrap();
    }

    #[tokio::test]
    async fn client_multiple_sends() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("multi.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let responses = vec![
            SessionResponse::Ok { message: "ok".into() },
            SessionResponse::Status { summary: test_summary("b-1") },
            SessionResponse::Ok { message: "done".into() },
        ];
        let server = tokio::spawn(mock_server(listener, responses));

        let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let (read, write) = stream.into_split();
        let mut client = SessionClient {
            reader: JsonLineReader::new(read),
            writer: JsonLineWriter::new(write),
        };

        // 同一连接发 3 次
        let r1 = client
            .send(SessionRequest::Prompt {
                text: "hi".into(),
                files: vec![],
            })
            .await
            .unwrap();
        assert!(matches!(r1, SessionResponse::Ok { .. }));

        let r2 = client.send(SessionRequest::GetStatus).await.unwrap();
        assert!(matches!(r2, SessionResponse::Status { .. }));

        let r3 = client.send(SessionRequest::Shutdown).await.unwrap();
        assert!(matches!(r3, SessionResponse::Ok { .. }));

        server.await.unwrap();
    }
}
