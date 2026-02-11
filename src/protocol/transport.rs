use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

// ==================== JSON Lines 读取端 ====================

pub struct JsonLineReader {
    reader: BufReader<OwnedReadHalf>,
}

impl JsonLineReader {
    pub fn new(read_half: OwnedReadHalf) -> Self {
        Self {
            reader: BufReader::new(read_half),
        }
    }

    /// 读取下一条 JSON 消息，EOF 返回 None
    pub async fn read<T: for<'de> Deserialize<'de>>(&mut self) -> Result<Option<T>> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .await
            .context("Failed to read from socket")?;
        if n == 0 {
            return Ok(None);
        }
        let msg = serde_json::from_str(line.trim())
            .context("Failed to deserialize message")?;
        Ok(Some(msg))
    }
}

// ==================== JSON Lines 写入端 ====================

pub struct JsonLineWriter {
    writer: OwnedWriteHalf,
}

impl JsonLineWriter {
    pub fn new(write_half: OwnedWriteHalf) -> Self {
        Self {
            writer: write_half,
        }
    }

    /// 写入一条 JSON 消息（自动追加换行符）
    pub async fn write<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        let mut json = serde_json::to_string(msg)
            .context("Failed to serialize message")?;
        json.push('\n');
        self.writer
            .write_all(json.as_bytes())
            .await
            .context("Failed to write to socket")?;
        self.writer
            .flush()
            .await
            .context("Failed to flush socket")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::messages::{SessionRequest, SessionResponse, AgentSummary};
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn roundtrip_over_uds() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("test.sock");
        let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

        let sock_path_clone = sock_path.clone();
        let handle = tokio::spawn(async move {
            let stream = UnixStream::connect(&sock_path_clone).await.unwrap();
            let (read, write) = stream.into_split();
            let mut writer = JsonLineWriter::new(write);
            let mut reader = JsonLineReader::new(read);

            // 发送请求
            writer.write(&SessionRequest::GetStatus).await.unwrap();

            // 接收响应
            let resp: SessionResponse = reader.read().await.unwrap().unwrap();
            resp
        });

        // server 端
        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = JsonLineReader::new(read);
        let mut writer = JsonLineWriter::new(write);

        // 接收请求
        let req: SessionRequest = reader.read().await.unwrap().unwrap();
        assert!(matches!(req, SessionRequest::GetStatus));

        // 发送响应
        writer
            .write(&SessionResponse::Status {
                summary: AgentSummary {
                    name: "test-1".into(),
                    agent_type: "mock".into(),
                    cwd: "/tmp".into(),
                    status: "idle".into(),
                    uptime: "0m 0s".into(),
                    prompt_count: 0,
                    pending_permissions: 0,
                    agent_info_name: None,
                    agent_info_version: None,
                },
            })
            .await
            .unwrap();

        let resp = handle.await.unwrap();
        match resp {
            SessionResponse::Status { summary } => {
                assert_eq!(summary.name, "test-1");
            }
            _ => panic!("expected Status"),
        }
    }

    #[tokio::test]
    async fn multiple_messages() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("multi.sock");
        let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

        let sock_path_clone = sock_path.clone();
        let handle = tokio::spawn(async move {
            let stream = UnixStream::connect(&sock_path_clone).await.unwrap();
            let (read, write) = stream.into_split();
            let mut writer = JsonLineWriter::new(write);
            let mut reader = JsonLineReader::new(read);

            // 发送 3 条请求
            writer.write(&SessionRequest::GetStatus).await.unwrap();
            writer
                .write(&SessionRequest::Prompt {
                    text: "hello".into(),
                    files: vec![],
                })
                .await
                .unwrap();
            writer.write(&SessionRequest::Shutdown).await.unwrap();

            // 接收 3 条响应
            let mut results = vec![];
            for _ in 0..3 {
                let resp: SessionResponse = reader.read().await.unwrap().unwrap();
                results.push(resp);
            }
            results
        });

        let (stream, _) = listener.accept().await.unwrap();
        let (read, write) = stream.into_split();
        let mut reader = JsonLineReader::new(read);
        let mut writer = JsonLineWriter::new(write);

        for _ in 0..3 {
            let _req: SessionRequest = reader.read().await.unwrap().unwrap();
            writer
                .write(&SessionResponse::Ok {
                    message: "done".into(),
                })
                .await
                .unwrap();
        }

        let results = handle.await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn eof_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("eof.sock");
        let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

        let sock_path_clone = sock_path.clone();
        tokio::spawn(async move {
            let stream = UnixStream::connect(&sock_path_clone).await.unwrap();
            drop(stream); // 立即关闭
        });

        let (stream, _) = listener.accept().await.unwrap();
        let (read, _write) = stream.into_split();
        let mut reader = JsonLineReader::new(read);

        let result: Option<SessionRequest> = reader.read().await.unwrap();
        assert!(result.is_none());
    }
}
