// ==================== Mock ACP Echo Agent ====================
// 用于集成测试的简单 ACP agent
// 接收 prompt → echo 回消息 → 返回 PromptResponse

use agent_client_protocol as acp;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

struct MockAgent;

#[async_trait::async_trait(?Send)]
impl acp::Agent for MockAgent {
    async fn initialize(
        &self,
        _args: acp::InitializeRequest,
    ) -> acp::Result<acp::InitializeResponse> {
        Ok(acp::InitializeResponse::new(acp::ProtocolVersion::V1))
    }

    async fn authenticate(
        &self,
        _args: acp::AuthenticateRequest,
    ) -> acp::Result<acp::AuthenticateResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn new_session(
        &self,
        _args: acp::NewSessionRequest,
    ) -> acp::Result<acp::NewSessionResponse> {
        Ok(acp::NewSessionResponse::new(acp::SessionId::new(
            "mock-session-1",
        )))
    }

    async fn prompt(
        &self,
        args: acp::PromptRequest,
    ) -> acp::Result<acp::PromptResponse> {
        let _text: String = args
            .prompt
            .iter()
            .filter_map(|block| match block {
                acp::ContentBlock::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))
    }

    async fn cancel(
        &self,
        _args: acp::CancelNotification,
    ) -> acp::Result<()> {
        Ok(())
    }
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let stdin = tokio::io::stdin().compat();
        let stdout = tokio::io::stdout().compat_write();

        let (_conn, io_task) = acp::AgentSideConnection::new(
            MockAgent,
            stdout,
            stdin,
            |fut| {
                tokio::task::spawn_local(fut);
            },
        );

        tokio::task::spawn_local(async move {
            if let Err(e) = io_task.await {
                eprintln!("Mock agent IO error: {}", e);
            }
        });

        // agent 端只需要让 io_task 跑着处理消息就行
        // 当 stdin 关闭（父进程退出）时 io_task 会结束
        // 这里用一个简单的 pending 等待
        std::future::pending::<()>().await;
    });
}
