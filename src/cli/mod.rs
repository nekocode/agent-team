mod commands;
mod display;
mod update;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::net::UnixStream;

use crate::config::TeamConfig;
use crate::protocol::messages::{SessionRequest, SessionResponse};
use crate::protocol::transport::{JsonLineReader, JsonLineWriter};

pub use commands::{Cli, Command};

pub fn parse() -> Cli {
    Cli::parse()
}

pub fn run(cli: Cli) -> Result<()> {
    // update 是纯同步，无需 tokio runtime
    if matches!(cli.command, Command::Update) {
        return update::run_update();
    }

    let rt = tokio::runtime::Runtime::new()?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, run_async(cli))
}

async fn run_async(cli: Cli) -> Result<()> {
    let config = TeamConfig::default();

    match cli.command {
        Command::Add {
            agent_type,
            name,
            cwd,
            args,
            background,
        } => {
            // 检查 agent 类型是否支持
            let type_config = config.agent_types.get(&agent_type)
                .ok_or_else(|| anyhow::anyhow!(
                    "Unknown agent type '{}'. Supported: {}",
                    agent_type,
                    {
                        let mut types: Vec<&str> = config.agent_types.keys()
                            .map(|s| s.as_str()).collect();
                        types.sort();
                        types.join(", ")
                    },
                ))?;

            // 适配器提示：检测命令是否在 PATH
            if let Some(hint) = crate::config::adapter_hint(&agent_type) {
                if !command_exists(&type_config.command) {
                    eprintln!(
                        "Adapter '{}' not found in PATH.\n\
                         Install: {}\n",
                        hint.adapter, hint.install,
                    );
                    std::process::exit(1);
                }
            }

            let resolved_name = name
                .unwrap_or_else(|| config.gen_name(&agent_type));

            if background {
                launch_background(
                    &config, &agent_type, &resolved_name,
                    cwd.as_deref(), args.as_deref(),
                )?;
                return Ok(());
            }

            let extra_args = args
                .map(|a| a.split_whitespace().map(String::from).collect())
                .unwrap_or_default();
            let effective_cwd = cwd
                .unwrap_or_else(|| config.default_cwd.clone());

            // 启动独立 session（阻塞，stdout 输出）
            crate::session::server::run(
                resolved_name,
                agent_type,
                config,
                extra_args,
                effective_cwd,
            )
            .await?;
        }

        Command::Rm { name, all } => {
            if all {
                // 扫描所有 socket，逐个 Shutdown
                let names = config.scan_sessions();
                if names.is_empty() {
                    println!("No agents running");
                    return Ok(());
                }
                let mut count = 0;
                for n in &names {
                    match send(&config, n, SessionRequest::Shutdown).await {
                        Ok(resp) => {
                            display::print_session_response(&resp);
                            count += 1;
                        }
                        Err(_) => eprintln!("Error: Failed to shut down {}", n),
                    }
                }
                println!("Shut down {} agents", count);
            } else {
                let resp = send(&config, &name, SessionRequest::Shutdown).await?;
                display::print_session_response(&resp);
            }
        }

        Command::Ls => {
            let names = config.scan_sessions();
            if names.is_empty() {
                println!("No agents running");
                return Ok(());
            }
            let mut summaries = vec![];
            for n in &names {
                match send(&config, n, SessionRequest::GetStatus).await {
                    Ok(SessionResponse::Status { summary }) => {
                        summaries.push(summary);
                    }
                    Ok(SessionResponse::Error { message }) => {
                        eprintln!("Error: {}: {}", n, message);
                    }
                    Err(_) => {
                        // send() 已清理残留 socket
                        eprintln!("Error: {} unreachable (cleaned)", n);
                    }
                    _ => {}
                }
            }
            display::print_agent_list(&summaries);
        }

        Command::Ask { name, text, file } => {
            let text = match text {
                Some(t) => t,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .context("Failed to read from stdin")?;
                    let buf = buf.trim().to_string();
                    if buf.is_empty() {
                        anyhow::bail!("No prompt text provided");
                    }
                    buf
                }
            };

            let mut files = Vec::new();
            for path in file {
                let content = tokio::fs::read_to_string(&path)
                    .await
                    .with_context(|| format!("Cannot read {}", path.display()))?;
                files.push(crate::protocol::messages::FileAttachment {
                    path,
                    content,
                });
            }

            prompt_and_wait(&config, &name, text, files).await?;
        }

        Command::Log { name, last, agent_only } => {
            let resp = send(
                &config,
                &name,
                SessionRequest::GetOutput { last, agent_only },
            )
            .await?;
            display::print_session_response(&resp);
        }

        Command::Cancel { name } => {
            let resp =
                send(&config, &name, SessionRequest::Cancel).await?;
            display::print_session_response(&resp);
        }

        Command::Allow { name, all } => {
            if all {
                let names = config.scan_sessions();
                let mut total = 0;
                for n in &names {
                    if let Ok(SessionResponse::Ok { .. }) =
                        send(&config, n, SessionRequest::ApprovePermission).await
                    {
                        total += 1;
                    }
                }
                println!("Allowed {} permissions", total);
            } else {
                let name = name.unwrap_or_default();
                if name.is_empty() {
                    anyhow::bail!("Specify agent name or use --all");
                }
                let resp = send(
                    &config,
                    &name,
                    SessionRequest::ApprovePermission,
                )
                .await?;
                display::print_session_response(&resp);
            }
        }

        Command::Deny { name } => {
            let resp = send(
                &config,
                &name,
                SessionRequest::DenyPermission,
            )
            .await?;
            display::print_session_response(&resp);
        }

        Command::Info { name } => {
            let resp =
                send(&config, &name, SessionRequest::GetStatus).await?;
            display::print_session_response(&resp);
        }

        Command::Restart { name } => {
            let resp =
                send(&config, &name, SessionRequest::Restart).await?;
            display::print_session_response(&resp);
        }

        Command::Mode { name, mode } => {
            let resp =
                send(&config, &name, SessionRequest::SetMode { mode }).await?;
            display::print_session_response(&resp);
        }

        Command::Set { name, key, value } => {
            let resp = send(
                &config,
                &name,
                SessionRequest::SetConfig { key, value },
            )
            .await?;
            display::print_session_response(&resp);
        }

        Command::Update => unreachable!("handled before runtime"),
    }
    Ok(())
}

// ==================== prompt（轮询等待） ====================

async fn prompt_and_wait(
    config: &TeamConfig,
    name: &str,
    text: String,
    files: Vec<crate::protocol::messages::FileAttachment>,
) -> Result<()> {
    let resp = send(
        config,
        name,
        SessionRequest::Prompt { text, files },
    )
    .await?;
    if !matches!(resp, SessionResponse::Ok { .. }) {
        display::print_session_response(&resp);
        return Ok(());
    }

    // 轮询 GetStatus 直到 idle / error / waiting_permission
    // 无超时限制 — AI 输出可能很长，由用户 Ctrl+C 中止
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let resp = send(config, name, SessionRequest::GetStatus).await?;
        if let SessionResponse::Status { ref summary } = resp {
            match summary.status.as_str() {
                "idle" | "error" | "waiting_permission" => break,
                _ => {}
            }
        }
    }

    // 取最后一条消息（agent 回复 / 权限请求）
    let resp = send(config, name, SessionRequest::GetOutput { last: 1, agent_only: false }).await?;
    display::print_session_response(&resp);
    Ok(())
}

// ==================== session 通信 ====================

async fn send(
    config: &TeamConfig,
    name: &str,
    req: SessionRequest,
) -> Result<SessionResponse> {
    let sock_path = config.session_socket(name);
    let stream = match UnixStream::connect(&sock_path).await {
        Ok(s) => s,
        Err(e) => {
            // 进程已死但 socket 残留 → 清理
            let _ = std::fs::remove_file(&sock_path);
            return Err(e).with_context(|| {
                format!("Cannot connect to agent '{}'. Is it running?", name)
            });
        }
    };

    let (read, write) = stream.into_split();
    let mut writer = JsonLineWriter::new(write);
    let mut reader = JsonLineReader::new(read);

    writer.write(&req).await?;
    let resp: SessionResponse = reader
        .read()
        .await?
        .context("Session closed connection unexpectedly")?;

    Ok(resp)
}

// ==================== 后台启动 ====================

fn launch_background(
    config: &TeamConfig,
    agent_type: &str,
    name: &str,
    cwd: Option<&std::path::Path>,
    args: Option<&str>,
) -> Result<()> {
    config.ensure_socket_dir()?;

    let exe = std::env::current_exe()
        .context("Cannot resolve executable path")?;

    // 重建命令行（不带 --background）
    let mut cmd_args = vec!["add".to_string(), agent_type.to_string()];
    cmd_args.extend(["--name".into(), name.to_string()]);
    if let Some(c) = cwd {
        cmd_args.extend(["--cwd".into(), c.display().to_string()]);
    }
    if let Some(a) = args {
        cmd_args.extend(["--args".into(), a.to_string()]);
    }

    let log_path = config.session_log(name);
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("Cannot create log: {}", log_path.display()))?;

    let mut cmd = std::process::Command::new(exe);
    cmd.args(&cmd_args)
        .stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);

    // Unix：新进程组，脱离终端
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd.spawn()
        .context("Failed to spawn background process")?;

    // 等 socket 出现（最多 10s）
    let sock_path = config.session_socket(name);
    let mut ready = false;
    for _ in 0..100 {
        if sock_path.exists() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    if ready {
        println!(
            "Agent '{}' started (pid: {}, log: {})",
            name, child.id(), log_path.display(),
        );
    } else {
        eprintln!(
            "Warning: Agent '{}' may not have started (check {})",
            name, log_path.display(),
        );
    }
    Ok(())
}

// ==================== 工具函数 ====================

fn command_exists(cmd: &str) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("sh")
            .args(["-c", &format!("command -v {}", cmd)])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("where")
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}
