use std::collections::VecDeque;
use std::sync::Arc;

use agent_client_protocol as acp;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::config::AutoApprovePolicy;
use crate::session::agent::{AgentStatus, OutputRingBuffer};
use crate::protocol::messages::{OutputEntry, OutputType};

// ==================== 权限请求队列 ====================

pub struct PendingPermission {
    pub tool_info: String,
    pub response_tx: oneshot::Sender<PermissionDecision>,
}

pub enum PermissionDecision {
    Approve,
    Deny,
}

// ==================== ACP Client 实现 ====================
// 每个 Agent 一个 TeamClient，处理回调（通知、权限等）

pub struct TeamClient {
    pub status: Arc<Mutex<AgentStatus>>,
    pub output_buffer: Arc<Mutex<OutputRingBuffer>>,
    pub pending_permissions: Arc<Mutex<VecDeque<PendingPermission>>>,
    pub auto_approve: AutoApprovePolicy,
    pub output_tx: Option<mpsc::UnboundedSender<OutputEntry>>,
}

impl TeamClient {
    pub fn new(
        status: Arc<Mutex<AgentStatus>>,
        buffer: Arc<Mutex<OutputRingBuffer>>,
        pending: Arc<Mutex<VecDeque<PendingPermission>>>,
        auto_approve: AutoApprovePolicy,
        output_tx: Option<mpsc::UnboundedSender<OutputEntry>>,
    ) -> Self {
        Self {
            status,
            output_buffer: buffer,
            pending_permissions: pending,
            auto_approve,
            output_tx,
        }
    }

    /// push 到 buffer + 通知 stdout
    async fn write_output(&self, update_type: OutputType, content: String) {
        let entry = OutputEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            update_type,
            content,
        };
        if let Some(tx) = &self.output_tx {
            tx.send(entry.clone()).ok();
        }
        self.output_buffer.lock().await.push(entry);
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for TeamClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        let tool_info = fmt_tool_info(&args.tool_call.fields);

        // auto-approve 策略
        if matches!(self.auto_approve, AutoApprovePolicy::Always) {
            self.write_output(
                OutputType::PermissionRequest,
                format!("Permission auto-approved: {}", tool_info),
            )
            .await;
            return Ok(permission_response(&args.options, true));
        }

        // 写入 output 让用户看到
        self.write_output(
            OutputType::PermissionRequest,
            format!("Permission requested: {} (Waiting for approval)", tool_info),
        )
        .await;

        // 创建 channel，放入 pending queue
        let (tx, rx) = oneshot::channel();
        {
            let mut queue = self.pending_permissions.lock().await;
            queue.push_back(PendingPermission {
                tool_info,
                response_tx: tx,
            });
        }

        // 状态 → WaitingPermission
        *self.status.lock().await = AgentStatus::WaitingPermission;

        // 等待用户回复
        let approved = matches!(rx.await, Ok(PermissionDecision::Approve));
        *self.status.lock().await = AgentStatus::Running;
        Ok(permission_response(&args.options, approved))
    }

    async fn session_notification(
        &self,
        args: acp::SessionNotification,
    ) -> acp::Result<()> {
        match &args.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                let text = extract_text(&chunk.content);
                if !text.is_empty() {
                    self.write_output(OutputType::AgentMessage, text).await;
                }
            }

            acp::SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = extract_text(&chunk.content);
                if !text.is_empty() {
                    self.write_output(OutputType::AgentThought, text).await;
                }
            }

            acp::SessionUpdate::ToolCall(tc) => {
                self.write_output(
                    OutputType::ToolCallStart,
                    tc.title.clone(),
                )
                .await;
            }

            acp::SessionUpdate::ToolCallUpdate(tcu) => {
                let mut parts = Vec::new();
                if let Some(title) = &tcu.fields.title {
                    parts.push(title.clone());
                }
                if let Some(status) = &tcu.fields.status {
                    parts.push(format!("{:?}", status));
                }
                let text = if parts.is_empty() {
                    "(No details)".to_string()
                } else {
                    parts.join(" ")
                };
                self.write_output(OutputType::ToolCallUpdate, text).await;
            }

            acp::SessionUpdate::Plan(plan) => {
                let entries_text: Vec<String> = plan
                    .entries
                    .iter()
                    .map(|e| {
                        format!("  [{:?}] {}", e.status, e.content)
                    })
                    .collect();
                self.write_output(
                    OutputType::PlanUpdate,
                    format!("Plan:\n{}", entries_text.join("\n")),
                )
                .await;
            }

            acp::SessionUpdate::CurrentModeUpdate(m) => {
                self.write_output(
                    OutputType::ModeUpdate,
                    format!("{}", m.current_mode_id.0),
                )
                .await;
            }

            acp::SessionUpdate::ConfigOptionUpdate(c) => {
                let items: Vec<String> = c
                    .config_options
                    .iter()
                    .map(|o| format!("{} ({})", o.name, o.id.0))
                    .collect();
                self.write_output(
                    OutputType::ConfigUpdate,
                    items.join(", "),
                )
                .await;
            }

            // AvailableCommandsUpdate 等信息性通知，静默忽略
            _ => {}
        }

        Ok(())
    }

}

// ==================== 辅助函数 ====================

fn permission_response(
    options: &[acp::PermissionOption],
    approved: bool,
) -> acp::RequestPermissionResponse {
    if approved {
        if let Some(opt) = options.first() {
            return acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Selected(
                    acp::SelectedPermissionOutcome::new(opt.option_id.clone()),
                ),
            );
        }
    }
    acp::RequestPermissionResponse::new(acp::RequestPermissionOutcome::Cancelled)
}

/// title 优先，fallback 到 kind，都没有才 "Unknown tool"
fn fmt_tool_info(fields: &acp::ToolCallUpdateFields) -> String {
    if let Some(title) = &fields.title {
        return title.clone();
    }
    if let Some(kind) = &fields.kind {
        return format!("{:?}", kind);
    }
    "Unknown tool".to_string()
}

fn extract_text(content: &acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_response_approve() {
        let opt = acp::PermissionOption::new(
            "allow-once",
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        );
        let resp = permission_response(&[opt], true);
        match resp.outcome {
            acp::RequestPermissionOutcome::Selected(s) => {
                assert_eq!(s.option_id.0.as_ref(), "allow-once");
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn permission_response_deny() {
        let opt = acp::PermissionOption::new(
            "allow-once",
            "Allow once",
            acp::PermissionOptionKind::AllowOnce,
        );
        let resp = permission_response(&[opt], false);
        assert!(matches!(
            resp.outcome,
            acp::RequestPermissionOutcome::Cancelled
        ));
    }

    #[test]
    fn permission_response_approve_empty_options() {
        let resp = permission_response(&[], true);
        assert!(matches!(
            resp.outcome,
            acp::RequestPermissionOutcome::Cancelled
        ));
    }

    #[test]
    fn fmt_tool_info_with_title() {
        let mut fields = acp::ToolCallUpdateFields::new();
        fields.title = Some("Edit /tmp/a.txt".into());
        assert_eq!(fmt_tool_info(&fields), "Edit /tmp/a.txt");
    }

    #[test]
    fn fmt_tool_info_with_kind() {
        let fields = acp::ToolCallUpdateFields::new().kind(acp::ToolKind::Execute);
        let result = fmt_tool_info(&fields);
        assert!(result.contains("Execute"));
    }

    #[test]
    fn fmt_tool_info_fallback() {
        let fields = acp::ToolCallUpdateFields::new();
        assert_eq!(fmt_tool_info(&fields), "Unknown tool");
    }

    #[test]
    fn extract_text_from_text_block() {
        let block = acp::ContentBlock::from("hello world");
        assert_eq!(extract_text(&block), "hello world");
    }

    #[test]
    fn extract_text_from_non_text() {
        let block = acp::ContentBlock::ResourceLink(
            acp::ResourceLink::new("test", "file:///tmp/a.txt"),
        );
        assert_eq!(extract_text(&block), "");
    }

    #[tokio::test]
    async fn write_output_pushes_to_buffer() {
        let buf = Arc::new(Mutex::new(OutputRingBuffer::new(10)));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = TeamClient::new(
            Arc::new(Mutex::new(AgentStatus::Idle)),
            Arc::clone(&buf),
            Arc::new(Mutex::new(std::collections::VecDeque::new())),
            AutoApprovePolicy::Never,
            Some(tx),
        );
        client.write_output(OutputType::AgentMessage, "hello".into()).await;
        let entries = buf.lock().await.last_msgs(0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "hello");
        // also sent to channel
        let received = rx.recv().await.unwrap();
        assert_eq!(received.content, "hello");
    }

    #[tokio::test]
    async fn write_output_no_sender() {
        let buf = Arc::new(Mutex::new(OutputRingBuffer::new(10)));
        let client = TeamClient::new(
            Arc::new(Mutex::new(AgentStatus::Idle)),
            Arc::clone(&buf),
            Arc::new(Mutex::new(std::collections::VecDeque::new())),
            AutoApprovePolicy::Never,
            None,
        );
        client.write_output(OutputType::Error, "oops".into()).await;
        let entries = buf.lock().await.last_msgs(0);
        assert_eq!(entries.len(), 1);
    }
}
