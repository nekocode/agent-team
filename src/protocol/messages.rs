use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ==================== Session 协议 ====================
// 每个 session 管一个 agent，请求无需 name 字段

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionRequest {
    GetStatus,
    Prompt {
        text: String,
        files: Vec<FileAttachment>,
    },
    GetOutput {
        last: usize,
        #[serde(default)]
        agent_only: bool,
    },
    Cancel,
    ApprovePermission,
    DenyPermission,
    Restart,
    Shutdown,
    SetMode { mode: String },
    SetConfig { key: String, value: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionResponse {
    Ok {
        message: String,
    },
    Error {
        message: String,
    },
    Status {
        summary: AgentSummary,
    },
    Output {
        agent_name: String,
        entries: Vec<OutputEntry>,
    },
}

impl SessionRequest {
    pub fn label(&self) -> &str {
        match self {
            Self::GetStatus => "GetStatus",
            Self::Prompt { .. } => "Prompt",
            Self::GetOutput { .. } => "GetOutput",
            Self::Cancel => "Cancel",
            Self::ApprovePermission => "ApprovePermission",
            Self::DenyPermission => "DenyPermission",
            Self::Restart => "Restart",
            Self::Shutdown => "Shutdown",
            Self::SetMode { .. } => "SetMode",
            Self::SetConfig { .. } => "SetConfig",
        }
    }
}

// ==================== 辅助类型 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttachment {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub name: String,
    pub agent_type: String,
    pub cwd: String,
    pub status: String,
    pub uptime: String,
    pub prompt_count: u64,
    pub pending_permissions: usize,
    /// agent 自报名称（来自 ACP initialize）
    pub agent_info_name: Option<String>,
    /// agent 自报版本
    pub agent_info_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEntry {
    pub timestamp: String,
    pub update_type: OutputType,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputType {
    UserPrompt,
    AgentMessage,
    AgentThought,
    ToolCallStart,
    ToolCallUpdate,
    ToolCallResult,
    PlanUpdate,
    PromptResponse,
    PermissionRequest,
    ModeUpdate,
    ConfigUpdate,
    Error,
}

impl OutputType {
    pub fn label(&self) -> &str {
        match self {
            Self::UserPrompt => "prompt",
            Self::AgentMessage => "message",
            Self::AgentThought => "thought",
            Self::ToolCallStart => "tool",
            Self::ToolCallUpdate => "tool_update",
            Self::ToolCallResult => "tool_result",
            Self::PlanUpdate => "plan",
            Self::PromptResponse => "done",
            Self::PermissionRequest => "permission",
            Self::ModeUpdate => "mode",
            Self::ConfigUpdate => "config",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for OutputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::fmt::Display for SessionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_request_roundtrip() {
        let req = SessionRequest::Prompt {
            text: "hello".into(),
            files: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: SessionRequest = serde_json::from_str(&json).unwrap();
        match back {
            SessionRequest::Prompt { text, .. } => assert_eq!(text, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn session_response_roundtrip() {
        let resp = SessionResponse::Status {
            summary: AgentSummary {
                name: "gemini-1".into(),
                agent_type: "gemini".into(),
                cwd: "/tmp".into(),
                status: "idle".into(),
                uptime: "1m 0s".into(),
                prompt_count: 0,
                pending_permissions: 0,
                agent_info_name: None,
                agent_info_version: None,
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: SessionResponse = serde_json::from_str(&json).unwrap();
        match back {
            SessionResponse::Status { summary } => {
                assert_eq!(summary.name, "gemini-1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn output_entry_serde() {
        let entry = OutputEntry {
            timestamp: "2026-02-09T12:00:00Z".into(),
            update_type: OutputType::AgentMessage,
            content: "Hello world".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("AgentMessage"));
        let back: OutputEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "Hello world");
    }

    #[test]
    fn session_request_labels() {
        let cases: Vec<(SessionRequest, &str)> = vec![
            (SessionRequest::GetStatus, "GetStatus"),
            (SessionRequest::Prompt { text: "".into(), files: vec![] }, "Prompt"),
            (SessionRequest::GetOutput { last: 0, agent_only: false }, "GetOutput"),
            (SessionRequest::Cancel, "Cancel"),
            (SessionRequest::ApprovePermission, "ApprovePermission"),
            (SessionRequest::DenyPermission, "DenyPermission"),
            (SessionRequest::Restart, "Restart"),
            (SessionRequest::Shutdown, "Shutdown"),
            (SessionRequest::SetMode { mode: "code".into() }, "SetMode"),
            (SessionRequest::SetConfig { key: "k".into(), value: "v".into() }, "SetConfig"),
        ];
        for (req, expected) in cases {
            assert_eq!(req.label(), expected);
        }
    }

    #[test]
    fn output_type_labels() {
        let cases: Vec<(OutputType, &str)> = vec![
            (OutputType::UserPrompt, "prompt"),
            (OutputType::AgentMessage, "message"),
            (OutputType::AgentThought, "thought"),
            (OutputType::ToolCallStart, "tool"),
            (OutputType::ToolCallUpdate, "tool_update"),
            (OutputType::ToolCallResult, "tool_result"),
            (OutputType::PlanUpdate, "plan"),
            (OutputType::PromptResponse, "done"),
            (OutputType::PermissionRequest, "permission"),
            (OutputType::ModeUpdate, "mode"),
            (OutputType::ConfigUpdate, "config"),
            (OutputType::Error, "error"),
        ];
        for (ot, expected) in cases {
            assert_eq!(ot.label(), expected);
        }
    }

    #[test]
    fn file_attachment_roundtrip() {
        let fa = FileAttachment {
            path: PathBuf::from("/tmp/test.rs"),
            content: "fn main() {}".into(),
        };
        let json = serde_json::to_string(&fa).unwrap();
        let back: FileAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, PathBuf::from("/tmp/test.rs"));
        assert_eq!(back.content, "fn main() {}");
    }
}
