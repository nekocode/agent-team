use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use agent_client_protocol::{self as acp, Agent};
use anyhow::{Context, Result};
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::acp_client::team_client::{PendingPermission, TeamClient};
use crate::config::{AgentTypeConfig, AutoApprovePolicy};
use crate::protocol::messages::{AgentSummary, OutputEntry, OutputType};

// ==================== Agent 状态机 ====================

#[derive(Clone, Debug, PartialEq)]
pub enum AgentStatus {
    Starting,
    Idle,
    Running,
    WaitingPermission,
    Error(String),
    Stopping,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => f.write_str("starting"),
            Self::Idle => f.write_str("idle"),
            Self::Running => f.write_str("running"),
            Self::WaitingPermission => f.write_str("waiting_permission"),
            Self::Error(_) => f.write_str("error"),
            Self::Stopping => f.write_str("stopping"),
        }
    }
}

// ==================== 输出环形缓冲区 ====================

pub struct OutputRingBuffer {
    entries: VecDeque<OutputEntry>,
    capacity: usize,
}

impl OutputRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, entry: OutputEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// 最近 n 条消息，0 = 全部
    /// 分隔点：角色切换（UserPrompt ↔ 非 UserPrompt）+ 交互点（PermissionRequest）
    pub fn last_msgs(&self, n: usize) -> Vec<OutputEntry> {
        if n == 0 {
            return self.entries.iter().cloned().collect();
        }
        let mut msg_starts: Vec<usize> = vec![];
        let mut prev_is_user: Option<bool> = None;
        let mut after_interaction = false;
        for (i, e) in self.entries.iter().enumerate() {
            let is_user = matches!(e.update_type, OutputType::UserPrompt);
            if after_interaction || prev_is_user != Some(is_user) {
                msg_starts.push(i);
            }
            prev_is_user = Some(is_user);
            after_interaction = matches!(e.update_type, OutputType::PermissionRequest);
        }
        if msg_starts.is_empty() {
            return self.entries.iter().cloned().collect();
        }
        let start = if n >= msg_starts.len() {
            0
        } else {
            msg_starts[msg_starts.len() - n]
        };
        self.entries.iter().skip(start).cloned().collect()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ==================== Agent 句柄 ====================

pub struct AgentHandle {
    pub name: String,
    pub agent_type: String,
    pub cwd: PathBuf,
    pub extra_args: Vec<String>,
    pub status: Arc<std::sync::Mutex<AgentStatus>>,
    pub started_at: Instant,
    pub output_buffer: Arc<Mutex<OutputRingBuffer>>,
    pub pending_permissions: Arc<Mutex<VecDeque<PendingPermission>>>,
    pub prompt_count: u64,
    pub session_id: Option<acp::SessionId>,
    pub acp_conn: Option<Rc<acp::ClientSideConnection>>,
    pub child: Option<Child>,
    /// agent 自报名称+版本（来自 InitializeResponse）
    pub agent_info: Option<(String, String)>,
}

impl AgentHandle {
    pub fn set_status(&self, s: AgentStatus) {
        *self.status.lock().unwrap() = s;
    }

    pub fn get_status(&self) -> AgentStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn to_summary(&self) -> AgentSummary {
        let uptime = self.started_at.elapsed();
        let mins = uptime.as_secs() / 60;
        let secs = uptime.as_secs() % 60;

        let pending = self
            .pending_permissions
            .try_lock()
            .map(|q| q.len())
            .unwrap_or(0);

        let (info_name, info_ver) = match &self.agent_info {
            Some((n, v)) => (Some(n.clone()), Some(v.clone())),
            None => (None, None),
        };

        AgentSummary {
            name: self.name.clone(),
            agent_type: self.agent_type.clone(),
            cwd: self.cwd.display().to_string(),
            status: self.get_status().to_string(),
            uptime: format!("{}m {}s", mins, secs),
            prompt_count: self.prompt_count,
            pending_permissions: pending,
            agent_info_name: info_name,
            agent_info_version: info_ver,
        }
    }
}

// ==================== spawn + ACP 连接 ====================

pub async fn spawn_agent(
    name: String,
    agent_type: String,
    type_config: AgentTypeConfig,
    cwd: PathBuf,
    extra_args: Vec<String>,
    buf_size: usize,
    auto_approve: AutoApprovePolicy,
    output_tx: Option<tokio::sync::mpsc::UnboundedSender<OutputEntry>>,
) -> Result<AgentHandle> {
    let mut cmd = tokio::process::Command::new(&type_config.command);
    cmd.args(&type_config.default_args)
        .args(&extra_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(&cwd)
        .kill_on_drop(true);

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn '{}'. Is it installed?",
            type_config.command
        )
    })?;

    let stdin = child.stdin.take().unwrap().compat_write();
    let stdout = child.stdout.take().unwrap().compat();
    let stderr = child.stderr.take().unwrap();

    // stderr → 后台读取（64KB 上限，仅用于 init 失败诊断）
    const STDERR_LIMIT: usize = 65_536;
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf2 = Arc::clone(&stderr_buf);
    tokio::task::spawn_local(async move {
        use tokio::io::AsyncReadExt;
        let mut reader = stderr;
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = reader.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let s = String::from_utf8_lossy(&buf[..n]);
            let mut sb = stderr_buf2.lock().await;
            if sb.len() < STDERR_LIMIT {
                sb.push_str(&s);
            }
        }
    });

    let status = Arc::new(std::sync::Mutex::new(AgentStatus::Starting));
    let output_buffer = Arc::new(Mutex::new(OutputRingBuffer::new(buf_size)));
    let pending_permissions = Arc::new(Mutex::new(VecDeque::new()));
    let err_tx = output_tx.clone();
    let client = TeamClient::new(
        Arc::clone(&status),
        Arc::clone(&output_buffer),
        Arc::clone(&pending_permissions),
        auto_approve,
        output_tx,
    );

    let (conn, io_task) = acp::ClientSideConnection::new(
        client,
        stdin,
        stdout,
        |fut| {
            tokio::task::spawn_local(fut);
        },
    );
    tokio::task::spawn_local(async move {
        if let Err(e) = io_task.await {
            if let Some(tx) = &err_tx {
                tx.send(OutputEntry {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    update_type: OutputType::Error,
                    content: format!("ACP IO error: {}", e),
                })
                .ok();
            }
        }
    });

    // ACP initialize
    let init_resp = match conn
        .initialize(acp::InitializeRequest::new(acp::ProtocolVersion::V1))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let stderr_output = stderr_buf.lock().await;
            if stderr_output.is_empty() {
                return Err(e).context("ACP initialization failed");
            }
            return Err(e).context(format!(
                "ACP initialize failed. stderr: {}",
                stderr_output.trim()
            ));
        }
    };

    let agent_info = init_resp.agent_info.map(|info| {
        (info.name, info.version)
    });

    let session_resp = conn
        .new_session(acp::NewSessionRequest::new(&cwd))
        .await
        .context("ACP new_session() failed")?;

    *status.lock().unwrap() = AgentStatus::Idle;

    Ok(AgentHandle {
        name,
        agent_type,
        cwd,
        extra_args,
        status,
        started_at: Instant::now(),
        output_buffer,
        pending_permissions,
        prompt_count: 0,
        session_id: Some(session_resp.session_id),
        acp_conn: Some(Rc::new(conn)),
        child: Some(child),
        agent_info,
    })
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_capacity() {
        let mut buf = OutputRingBuffer::new(3);
        for i in 0..5 {
            buf.push(OutputEntry {
                timestamp: format!("t{}", i),
                update_type: OutputType::AgentMessage,
                content: format!("msg-{}", i),
            });
        }
        let all = buf.last_msgs(0);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].content, "msg-2");
        assert_eq!(all[2].content, "msg-4");
    }

    /// last=1 → 最后一条消息（agent 回复块）
    #[test]
    fn last_msgs_one() {
        let mut buf = OutputRingBuffer::new(100);
        // msg1: user
        buf.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::UserPrompt,
            content: "hello".into(),
        });
        // msg2: agent（包含 AgentMessage + PromptResponse）
        buf.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::AgentMessage,
            content: "reply".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t2".into(),
            update_type: OutputType::PromptResponse,
            content: "done".into(),
        });

        let last = buf.last_msgs(1);
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].content, "reply");
        assert_eq!(last[1].content, "done");
    }

    /// last=2 → 最后两条消息（user 提问 + agent 回复）
    #[test]
    fn last_msgs_two() {
        let mut buf = OutputRingBuffer::new(100);
        // 第1轮
        buf.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::UserPrompt,
            content: "q1".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::AgentMessage,
            content: "a1".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t2".into(),
            update_type: OutputType::PromptResponse,
            content: "done1".into(),
        });
        // 第2轮
        buf.push(OutputEntry {
            timestamp: "t3".into(),
            update_type: OutputType::UserPrompt,
            content: "q2".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t4".into(),
            update_type: OutputType::AgentMessage,
            content: "a2".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t5".into(),
            update_type: OutputType::PromptResponse,
            content: "done2".into(),
        });

        // last=2: user msg "q2" + agent msg "a2,done2"
        let last = buf.last_msgs(2);
        assert_eq!(last.len(), 3);
        assert_eq!(last[0].content, "q2");
        assert_eq!(last[1].content, "a2");
    }

    #[test]
    fn last_msgs_no_entries() {
        let buf = OutputRingBuffer::new(100);
        let last = buf.last_msgs(1);
        assert!(last.is_empty());
    }

    /// 纯 agent 消息（无 UserPrompt），整体算一条消息
    #[test]
    fn last_msgs_agent_only() {
        let mut buf = OutputRingBuffer::new(100);
        buf.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::AgentMessage,
            content: "partial".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::ToolCallStart,
            content: "tool".into(),
        });

        let last = buf.last_msgs(1);
        assert_eq!(last.len(), 2);
    }

    /// PermissionRequest 是交互点，之后开启新消息
    #[test]
    fn last_msgs_permission_splits() {
        let mut buf = OutputRingBuffer::new(100);
        // user prompt
        buf.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::UserPrompt,
            content: "edit file".into(),
        });
        // agent work + 交互点（同一条消息）
        buf.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::AgentMessage,
            content: "sure".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t2".into(),
            update_type: OutputType::ToolCallStart,
            content: "edit /tmp/a.txt".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t3".into(),
            update_type: OutputType::PermissionRequest,
            content: "allow edit?".into(),
        });
        // 审批后的后续输出（新消息）
        buf.push(OutputEntry {
            timestamp: "t4".into(),
            update_type: OutputType::ToolCallResult,
            content: "edited".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t5".into(),
            update_type: OutputType::AgentMessage,
            content: "done".into(),
        });

        // last=1 → 审批后输出（ToolCallResult + AgentMessage）
        let last = buf.last_msgs(1);
        assert_eq!(last.len(), 2);
        assert!(matches!(last[0].update_type, OutputType::ToolCallResult));

        // last=2 → agent work（含 PermissionRequest）+ 后续输出
        let last = buf.last_msgs(2);
        assert_eq!(last.len(), 5);
        assert!(matches!(last[0].update_type, OutputType::AgentMessage));
        assert!(matches!(last[2].update_type, OutputType::PermissionRequest));
        assert!(matches!(last[3].update_type, OutputType::ToolCallResult));
    }

    #[test]
    fn last_msgs_overflow() {
        let mut buf = OutputRingBuffer::new(100);
        buf.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::UserPrompt,
            content: "q".into(),
        });
        buf.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::AgentMessage,
            content: "a".into(),
        });

        // 请求 10 条但只有 2 条消息，返回全部
        let last = buf.last_msgs(10);
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].content, "q");
    }

    #[test]
    fn ring_buffer_is_empty() {
        let buf = OutputRingBuffer::new(10);
        assert!(buf.is_empty());
    }

    #[test]
    fn status_display_all_variants() {
        assert_eq!(AgentStatus::Starting.to_string(), "starting");
        assert_eq!(AgentStatus::Idle.to_string(), "idle");
        assert_eq!(AgentStatus::Running.to_string(), "running");
        assert_eq!(AgentStatus::WaitingPermission.to_string(), "waiting_permission");
        assert_eq!(AgentStatus::Error("oops".into()).to_string(), "error");
        assert_eq!(AgentStatus::Stopping.to_string(), "stopping");
    }

    #[test]
    fn to_summary_with_agent_info() {
        let handle = AgentHandle {
            name: "test".into(),
            agent_type: "mock".into(),
            cwd: PathBuf::from("/tmp"),
            extra_args: vec![],
            status: Arc::new(std::sync::Mutex::new(AgentStatus::Running)),
            started_at: Instant::now(),
            output_buffer: Arc::new(Mutex::new(OutputRingBuffer::new(10))),
            pending_permissions: Arc::new(Mutex::new(VecDeque::new())),
            prompt_count: 5,
            session_id: None,
            acp_conn: None,
            child: None,
            agent_info: Some(("Gemini".into(), "2.0".into())),
        };
        let s = handle.to_summary();
        assert_eq!(s.name, "test");
        assert_eq!(s.status, "running");
        assert_eq!(s.prompt_count, 5);
        assert_eq!(s.agent_info_name, Some("Gemini".into()));
        assert_eq!(s.agent_info_version, Some("2.0".into()));
    }

    #[test]
    fn to_summary_without_agent_info() {
        let handle = AgentHandle {
            name: "bob".into(),
            agent_type: "claude".into(),
            cwd: PathBuf::from("/home"),
            extra_args: vec!["--fast".into()],
            status: Arc::new(std::sync::Mutex::new(AgentStatus::Idle)),
            started_at: Instant::now(),
            output_buffer: Arc::new(Mutex::new(OutputRingBuffer::new(10))),
            pending_permissions: Arc::new(Mutex::new(VecDeque::new())),
            prompt_count: 0,
            session_id: None,
            acp_conn: None,
            child: None,
            agent_info: None,
        };
        let s = handle.to_summary();
        assert_eq!(s.agent_type, "claude");
        assert!(s.agent_info_name.is_none());
        assert!(s.agent_info_version.is_none());
    }
}
