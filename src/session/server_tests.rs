use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, Mutex};

use crate::acp_client::team_client::{PendingPermission, PermissionDecision};
use crate::config::TeamConfig;
use crate::protocol::messages::{OutputEntry, OutputType, SessionRequest, SessionResponse};
use crate::session::agent::{AgentHandle, AgentStatus, OutputRingBuffer};
use crate::session::server::{cleanup_socket, handle_request, no_session, Event};

fn stub_handle(name: &str) -> Rc<RefCell<AgentHandle>> {
    Rc::new(RefCell::new(AgentHandle {
        name: name.into(),
        agent_type: "mock".into(),
        cwd: PathBuf::from("/tmp"),
        extra_args: vec![],
        status: Arc::new(Mutex::new(AgentStatus::Idle)),
        started_at: Instant::now(),
        output_buffer: Arc::new(Mutex::new(OutputRingBuffer::new(100))),
        pending_permissions: Arc::new(Mutex::new(VecDeque::new())),
        prompt_count: 0,
        session_id: None,
        acp_conn: None,
        child: None,
        agent_info: None,
    }))
}

fn test_event_tx() -> mpsc::UnboundedSender<Event> {
    mpsc::unbounded_channel().0
}

#[tokio::test]
async fn get_status() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(&h, &config, SessionRequest::GetStatus, &etx).await;
    match resp {
        SessionResponse::Status { summary } => {
            assert_eq!(summary.name, "test");
            assert_eq!(summary.status, "idle");
        }
        _ => panic!("expected Status"),
    }
}

#[tokio::test]
async fn prompt_no_connection() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config,
        SessionRequest::Prompt { text: "hello".into(), files: vec![] },
        &etx,
    ).await;
    assert!(matches!(resp, SessionResponse::Error { .. }));
}

#[tokio::test]
async fn cancel_no_session() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(&h, &config, SessionRequest::Cancel, &etx).await;
    assert!(matches!(resp, SessionResponse::Error { .. }));
}

#[tokio::test]
async fn shutdown_response() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(&h, &config, SessionRequest::Shutdown, &etx).await;
    match resp {
        SessionResponse::Ok { message } => assert!(message.contains("shutting down")),
        _ => panic!("expected Ok"),
    }
}

#[tokio::test]
async fn set_mode_no_connection() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config,
        SessionRequest::SetMode { mode: "code".into() },
        &etx,
    ).await;
    assert!(matches!(resp, SessionResponse::Error { .. }));
}

#[tokio::test]
async fn set_config_no_connection() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config,
        SessionRequest::SetConfig { key: "model".into(), value: "gpt-4".into() },
        &etx,
    ).await;
    assert!(matches!(resp, SessionResponse::Error { .. }));
}

#[tokio::test]
async fn get_output_empty() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config,
        SessionRequest::GetOutput { last: 0, agent_only: false },
        &etx,
    ).await;
    match resp {
        SessionResponse::Output { agent_name, entries } => {
            assert_eq!(agent_name, "test");
            assert!(entries.is_empty());
        }
        _ => panic!("expected Output"),
    }
}

#[tokio::test]
async fn get_output_with_entries() {
    let h = stub_handle("test");
    {
        let buf = h.borrow().output_buffer.clone();
        let mut b = buf.lock().await;
        b.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::UserPrompt,
            content: "hello".into(),
        });
        b.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::AgentMessage,
            content: "world".into(),
        });
    }
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config,
        SessionRequest::GetOutput { last: 0, agent_only: false },
        &etx,
    ).await;
    match resp {
        SessionResponse::Output { entries, .. } => assert_eq!(entries.len(), 2),
        _ => panic!("expected Output"),
    }
}

#[tokio::test]
async fn get_output_agent_only() {
    let h = stub_handle("test");
    {
        let buf = h.borrow().output_buffer.clone();
        let mut b = buf.lock().await;
        b.push(OutputEntry {
            timestamp: "t0".into(),
            update_type: OutputType::UserPrompt,
            content: "user".into(),
        });
        b.push(OutputEntry {
            timestamp: "t1".into(),
            update_type: OutputType::AgentMessage,
            content: "agent".into(),
        });
    }
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config,
        SessionRequest::GetOutput { last: 0, agent_only: true },
        &etx,
    ).await;
    match resp {
        SessionResponse::Output { entries, .. } => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].content, "agent");
        }
        _ => panic!("expected Output"),
    }
}

#[tokio::test]
async fn approve_no_pending() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config, SessionRequest::ApprovePermission, &etx,
    ).await;
    match resp {
        SessionResponse::Error { message } => assert!(message.contains("No pending")),
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn deny_no_pending() {
    let h = stub_handle("test");
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config, SessionRequest::DenyPermission, &etx,
    ).await;
    match resp {
        SessionResponse::Error { message } => assert!(message.contains("No pending")),
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn approve_with_pending() {
    let h = stub_handle("test");
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let queue = h.borrow().pending_permissions.clone();
        queue.lock().await.push_back(PendingPermission {
            tool_info: "edit /tmp/a.txt".into(),
            response_tx: tx,
        });
    }
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config, SessionRequest::ApprovePermission, &etx,
    ).await;
    match resp {
        SessionResponse::Ok { message } => assert!(message.contains("Approved")),
        _ => panic!("expected Ok"),
    }
    let decision = rx.await.unwrap();
    assert!(matches!(decision, PermissionDecision::Approve));
}

#[tokio::test]
async fn deny_with_pending() {
    let h = stub_handle("test");
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let queue = h.borrow().pending_permissions.clone();
        queue.lock().await.push_back(PendingPermission {
            tool_info: "rm /tmp/danger".into(),
            response_tx: tx,
        });
    }
    let config = TeamConfig::default();
    let etx = test_event_tx();
    let resp = handle_request(
        &h, &config, SessionRequest::DenyPermission, &etx,
    ).await;
    match resp {
        SessionResponse::Ok { message } => assert!(message.contains("Denied")),
        _ => panic!("expected Ok"),
    }
    let decision = rx.await.unwrap();
    assert!(matches!(decision, PermissionDecision::Deny));
}

#[test]
fn cleanup_socket_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("test.sock");
    std::fs::File::create(&sock).unwrap();
    assert!(sock.exists());
    cleanup_socket(&sock);
    assert!(!sock.exists());
}

#[test]
fn cleanup_socket_noop_if_missing() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("gone.sock");
    cleanup_socket(&sock);
}

#[test]
fn no_session_returns_error() {
    let resp = no_session();
    match resp {
        SessionResponse::Error { message } => {
            assert!(message.contains("No active session"));
        }
        _ => panic!("expected Error"),
    }
}

#[tokio::test]
async fn restart_unknown_agent_type() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let h = stub_handle("test");
        let config = TeamConfig::default();
        let etx = test_event_tx();
        let resp = handle_request(
            &h, &config, SessionRequest::Restart, &etx,
        ).await;
        match resp {
            SessionResponse::Error { message } => {
                assert!(message.contains("Unknown agent type"));
            }
            _ => panic!("expected Error"),
        }
        assert_eq!(h.borrow().get_status(), AgentStatus::Stopping);
    }).await;
}
