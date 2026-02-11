use crate::protocol::messages::{
    AgentSummary, OutputEntry, OutputType, SessionResponse,
};

// ==================== 终端输出格式化 ====================
// 所有输出面向 Agent 阅读：纯文本、无颜色、结构清晰

pub fn print_session_response(resp: &SessionResponse) {
    match resp {
        SessionResponse::Ok { message } => {
            println!("{}", message);
        }

        SessionResponse::Error { message } => {
            eprintln!("Error: {}", message);
        }

        SessionResponse::Status { summary } => {
            println!("Name: {}", summary.name);
            println!("Type: {}", summary.agent_type);
            if let Some(ref info_name) = summary.agent_info_name {
                let ver = summary.agent_info_version.as_deref().unwrap_or("?");
                println!("Agent: {} v{}", info_name, ver);
            }
            println!("Cwd: {}", summary.cwd);
            println!("Status: {}", summary.status);
            println!("Uptime: {}", summary.uptime);
            println!("Prompts: {}", summary.prompt_count);
            println!("Pending: {}", summary.pending_permissions);
        }

        SessionResponse::Output { agent_name, entries } => {
            print_entries(agent_name, entries);
        }
    }
}

// ==================== agent 列表 ====================

pub fn print_agent_list(agents: &[AgentSummary]) {
    if agents.is_empty() {
        println!("No agents running");
        return;
    }

    let headers = ["NAME", "TYPE", "STATUS", "UPTIME", "PROMPTS", "PENDING", "CWD"];
    let rows: Vec<[String; 7]> = agents
        .iter()
        .map(|a| {
            [
                a.name.clone(),
                a.agent_type.clone(),
                a.status.clone(),
                a.uptime.clone(),
                a.prompt_count.to_string(),
                a.pending_permissions.to_string(),
                a.cwd.clone(),
            ]
        })
        .collect();

    let mut widths = headers.map(|h| h.len());
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    for (i, h) in headers.iter().enumerate() {
        if i > 0 {
            print!("  ");
        }
        print!("{:<w$}", h, w = widths[i]);
    }
    println!();

    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                print!("  ");
            }
            print!("{:<w$}", cell, w = widths[i]);
        }
        println!();
    }

    // 有 pending 权限时提示操作方式
    let pending: Vec<_> = agents
        .iter()
        .filter(|a| a.pending_permissions > 0)
        .collect();
    if !pending.is_empty() {
        println!();
        for a in &pending {
            println!(
                "Tip: {} has {} pending — allow {} / deny {}",
                a.name, a.pending_permissions, a.name, a.name,
            );
        }
    }
}

// ==================== 输出格式化 ====================

/// 对话流显示：<msg> 包裹每条消息，空行分隔段落
fn print_entries(agent_name: &str, entries: &[OutputEntry]) {
    // 当前角色：user / agent / ""
    let mut role = "";
    let mut has_content = false; // msg 内是否已有内容
    let mut prev_was_text = false; // 上一条是正文（AgentMessage/Thought）
    let mut after_interaction = false; // 上一条是交互点
    let mut i = 0;

    while i < entries.len() {
        let entry = &entries[i];

        // PromptResponse 跳过
        if matches!(entry.update_type, OutputType::PromptResponse) {
            i += 1;
            continue;
        }

        let new_role = if matches!(entry.update_type, OutputType::UserPrompt) {
            "user"
        } else {
            "agent"
        };

        // 角色切换或交互点后 → 关旧 msg、开新 msg
        if new_role != role || after_interaction {
            if !role.is_empty() {
                println!("</msg>\n");
            }
            if new_role == "user" {
                println!("<msg role=\"user\">");
            } else {
                println!("<msg role=\"agent\" name=\"{}\">", agent_name);
            }
            role = new_role;
            has_content = false;
            prev_was_text = false;
        }
        after_interaction = false;

        // 用户 prompt
        if matches!(entry.update_type, OutputType::UserPrompt) {
            println!("{}", entry.content.trim());
            has_content = true;
            i += 1;
            continue;
        }

        // agent 内容
        match entry.update_type {
            OutputType::AgentMessage | OutputType::AgentThought => {
                let disc = std::mem::discriminant(&entry.update_type);
                let mut text = String::new();
                while i < entries.len()
                    && std::mem::discriminant(&entries[i].update_type) == disc
                {
                    text.push_str(&entries[i].content);
                    i += 1;
                }
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                if has_content {
                    println!();
                }
                println!("{}", text);
                has_content = true;
                prev_was_text = true;
            }
            _ => {
                if prev_was_text {
                    println!();
                }
                println!("[{}] {}", entry.update_type.label(), entry.content);
                prev_was_text = false;
                has_content = true;
                after_interaction = matches!(entry.update_type, OutputType::PermissionRequest);
                i += 1;
            }
        }
    }

    // 关闭最后一个 msg
    if !role.is_empty() {
        println!("</msg>");
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_summary(name: &str) -> AgentSummary {
        AgentSummary {
            name: name.into(),
            agent_type: "mock".into(),
            cwd: "/tmp".into(),
            status: "idle".into(),
            uptime: "1m 0s".into(),
            prompt_count: 3,
            pending_permissions: 0,
            agent_info_name: None,
            agent_info_version: None,
        }
    }

    fn make_entry(update_type: OutputType, content: &str) -> OutputEntry {
        OutputEntry {
            timestamp: "2026-01-01T00:00:00Z".into(),
            update_type,
            content: content.into(),
        }
    }

    // -- print_session_response --

    #[test]
    fn response_ok() {
        print_session_response(&SessionResponse::Ok {
            message: "done".into(),
        });
    }

    #[test]
    fn response_error() {
        print_session_response(&SessionResponse::Error {
            message: "something broke".into(),
        });
    }

    #[test]
    fn response_status() {
        print_session_response(&SessionResponse::Status {
            summary: make_summary("alice"),
        });
    }

    #[test]
    fn response_status_with_agent_info() {
        let mut s = make_summary("bob");
        s.agent_info_name = Some("Gemini".into());
        s.agent_info_version = Some("1.0".into());
        print_session_response(&SessionResponse::Status { summary: s });
    }

    #[test]
    fn response_output_empty() {
        print_session_response(&SessionResponse::Output {
            agent_name: "test".into(),
            entries: vec![],
        });
    }

    #[test]
    fn response_output_with_entries() {
        print_session_response(&SessionResponse::Output {
            agent_name: "test".into(),
            entries: vec![
                make_entry(OutputType::UserPrompt, "hello"),
                make_entry(OutputType::AgentMessage, "world"),
            ],
        });
    }

    // -- print_agent_list --

    #[test]
    fn agent_list_empty() {
        print_agent_list(&[]);
    }

    #[test]
    fn agent_list_single() {
        print_agent_list(&[make_summary("alice")]);
    }

    #[test]
    fn agent_list_multiple() {
        let mut bob = make_summary("bob");
        bob.pending_permissions = 2;
        print_agent_list(&[make_summary("alice"), bob]);
    }

    // -- print_entries --

    #[test]
    fn entries_user_then_agent() {
        let entries = vec![
            make_entry(OutputType::UserPrompt, "ask something"),
            make_entry(OutputType::AgentMessage, "here is the "),
            make_entry(OutputType::AgentMessage, "answer"),
            make_entry(OutputType::PromptResponse, "done"),
        ];
        print_entries("bot", &entries);
    }

    #[test]
    fn entries_tool_calls() {
        let entries = vec![
            make_entry(OutputType::AgentMessage, "let me check"),
            make_entry(OutputType::ToolCallStart, "read /tmp/a.txt"),
            make_entry(OutputType::ToolCallResult, "file content"),
            make_entry(OutputType::AgentMessage, "found it"),
        ];
        print_entries("bot", &entries);
    }

    #[test]
    fn entries_permission_splits() {
        let entries = vec![
            make_entry(OutputType::UserPrompt, "edit file"),
            make_entry(OutputType::AgentMessage, "sure"),
            make_entry(OutputType::PermissionRequest, "allow edit?"),
            make_entry(OutputType::ToolCallResult, "edited"),
            make_entry(OutputType::AgentMessage, "done"),
        ];
        print_entries("bot", &entries);
    }

    #[test]
    fn entries_empty_agent_message() {
        let entries = vec![
            make_entry(OutputType::AgentMessage, "   "),
            make_entry(OutputType::AgentMessage, "real content"),
        ];
        print_entries("bot", &entries);
    }

    #[test]
    fn entries_prompt_response_skipped() {
        let entries = vec![
            make_entry(OutputType::PromptResponse, "done"),
        ];
        print_entries("bot", &entries);
    }

    #[test]
    fn entries_thought() {
        let entries = vec![
            make_entry(OutputType::AgentThought, "thinking..."),
            make_entry(OutputType::AgentMessage, "answer"),
        ];
        print_entries("bot", &entries);
    }
}
