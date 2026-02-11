use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ==================== Agent 类型配置 ====================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTypeConfig {
    pub command: String,
    pub default_args: Vec<String>,
}

// ==================== 权限策略 ====================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AutoApprovePolicy {
    Always,
    Never,
    ReadOnly,
}

// ==================== 全局配置 ====================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TeamConfig {
    pub auto_approve: AutoApprovePolicy,
    pub output_buffer_size: usize,
    pub agent_types: HashMap<String, AgentTypeConfig>,
    pub default_cwd: PathBuf,
    pub socket_dir: PathBuf,
}

impl Default for TeamConfig {
    fn default() -> Self {
        let mut agent_types = HashMap::new();

        // -- 原生 ACP：--acp flag --
        for (name, cmd, flag) in [
            ("copilot", "copilot", "--acp"),
            ("auggie", "auggie", "--acp"),
            ("cline", "cline", "--acp"),
            ("qoder", "qodercli", "--acp"),
            ("qwen", "qwen", "--acp"),
        ] {
            agent_types.insert(
                name.to_string(),
                AgentTypeConfig {
                    command: cmd.to_string(),
                    default_args: vec![flag.to_string()],
                },
            );
        }

        // -- 原生 ACP：--experimental-acp flag --
        for (name, cmd) in [
            ("gemini", "gemini"),
            ("blackbox", "blackbox"),
        ] {
            agent_types.insert(
                name.to_string(),
                AgentTypeConfig {
                    command: cmd.to_string(),
                    default_args: vec!["--experimental-acp".to_string()],
                },
            );
        }

        // -- 原生 ACP：acp 子命令 --
        for (name, cmd) in [
            ("goose", "goose"),
            ("kiro", "kiro-cli"),
            ("openhands", "openhands"),
            ("opencode", "opencode"),
            ("kimi", "kimi"),
            ("cagent", "cagent"),
            ("stakpak", "stakpak"),
            ("vtcode", "vtcode"),
        ] {
            agent_types.insert(
                name.to_string(),
                AgentTypeConfig {
                    command: cmd.to_string(),
                    default_args: vec!["acp".to_string()],
                },
            );
        }

        // -- 独立 ACP 二进制 --
        for (name, cmd) in [
            ("vibe", "vibe-acp"),
            ("fast-agent", "fast-agent-acp"),
        ] {
            agent_types.insert(
                name.to_string(),
                AgentTypeConfig {
                    command: cmd.to_string(),
                    default_args: vec![],
                },
            );
        }

        // -- 需要适配器 --
        for (name, cmd) in [
            ("claude", "claude-code-acp"),
            ("codex", "codex-acp"),
            ("pi", "pi-acp"),
        ] {
            agent_types.insert(
                name.to_string(),
                AgentTypeConfig {
                    command: cmd.to_string(),
                    default_args: vec![],
                },
            );
        }

        let uid = unsafe { libc::getuid() };
        let socket_dir =
            std::env::temp_dir().join(format!("agent-team-{}", uid));

        Self {
            auto_approve: AutoApprovePolicy::Never,
            output_buffer_size: 10000,
            agent_types,
            default_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            socket_dir,
        }
    }
}

// ==================== 适配器提示 ====================

pub struct AdapterHint {
    pub adapter: &'static str,
    pub install: &'static str,
}

/// 需要额外适配器的 agent，返回安装提示
pub fn adapter_hint(agent_type: &str) -> Option<AdapterHint> {
    match agent_type {
        "claude" => Some(AdapterHint {
            adapter: "claude-code-acp",
            install: "npm install -g @zed-industries/claude-code-acp",
        }),
        "codex" => Some(AdapterHint {
            adapter: "codex-acp",
            install: "npm install -g @zed-industries/codex-acp",
        }),
        "pi" => Some(AdapterHint {
            adapter: "pi-acp",
            install: "npm install -g pi-acp",
        }),
        _ => None,
    }
}

// ==================== Session socket 辅助 ====================

impl TeamConfig {
    /// agent name → socket 路径
    pub fn session_socket(&self, name: &str) -> PathBuf {
        self.socket_dir.join(format!("{}.sock", name))
    }

    /// agent name → 后台日志路径
    pub fn session_log(&self, name: &str) -> PathBuf {
        self.socket_dir.join(format!("{}.log", name))
    }

    /// 确保 socket 目录存在
    pub fn ensure_socket_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.socket_dir)
    }

    /// 扫描活跃 session，返回 agent 名字列表
    pub fn scan_sessions(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.socket_dir) else {
            return vec![];
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".sock").map(|s| s.to_string())
            })
            .collect();
        names.sort();
        names
    }

    /// 生成下一个 agent 名字：扫描已有 socket，{type}-{max+1}
    pub fn gen_name(&self, agent_type: &str) -> String {
        let prefix = format!("{}-", agent_type);
        let max_num = self
            .scan_sessions()
            .iter()
            .filter_map(|name| {
                name.strip_prefix(&prefix)
                    .and_then(|suffix| suffix.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0);
        format!("{}-{}", agent_type, max_num + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_socket_path() {
        let config = TeamConfig::default();
        let path = config.session_socket("gemini-1");
        assert!(path.to_string_lossy().ends_with("gemini-1.sock"));
    }

    #[test]
    fn gen_name_no_existing() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = TeamConfig::default();
        config.socket_dir = dir.path().to_path_buf();
        assert_eq!(config.gen_name("gemini"), "gemini-1");
    }

    #[test]
    fn gen_name_with_existing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("gemini-1.sock")).unwrap();
        std::fs::File::create(dir.path().join("gemini-2.sock")).unwrap();
        let mut config = TeamConfig::default();
        config.socket_dir = dir.path().to_path_buf();
        assert_eq!(config.gen_name("gemini"), "gemini-3");
    }

    #[test]
    fn gen_name_different_types() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("gemini-1.sock")).unwrap();
        std::fs::File::create(dir.path().join("claude-1.sock")).unwrap();
        let mut config = TeamConfig::default();
        config.socket_dir = dir.path().to_path_buf();
        assert_eq!(config.gen_name("gemini"), "gemini-2");
        assert_eq!(config.gen_name("claude"), "claude-2");
        assert_eq!(config.gen_name("copilot"), "copilot-1");
    }

    #[test]
    fn scan_sessions_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = TeamConfig::default();
        config.socket_dir = dir.path().to_path_buf();
        assert!(config.scan_sessions().is_empty());
    }

    #[test]
    fn scan_sessions_finds_sockets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("alice.sock")).unwrap();
        std::fs::File::create(dir.path().join("bob.sock")).unwrap();
        std::fs::File::create(dir.path().join("not-a-socket.txt")).unwrap();
        let mut config = TeamConfig::default();
        config.socket_dir = dir.path().to_path_buf();
        let sessions = config.scan_sessions();
        assert_eq!(sessions, vec!["alice", "bob"]);
    }

    #[test]
    fn session_log_path() {
        let config = TeamConfig::default();
        let path = config.session_log("gemini-1");
        assert!(path.to_string_lossy().ends_with("gemini-1.log"));
    }

    #[test]
    fn ensure_socket_dir_creates() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = TeamConfig::default();
        config.socket_dir = dir.path().join("nested").join("sockets");
        config.ensure_socket_dir().unwrap();
        assert!(config.socket_dir.exists());
    }

    #[test]
    fn all_agent_types_registered() {
        let config = TeamConfig::default();
        let expected = [
            "gemini", "copilot", "goose", "claude", "codex",
            "auggie", "kiro", "cline", "blackbox", "openhands",
            "qoder", "opencode", "kimi", "vibe", "qwen",
            "cagent", "fast-agent", "stakpak", "vtcode", "pi",
        ];
        for name in expected {
            assert!(
                config.agent_types.contains_key(name),
                "Missing agent type: {}", name,
            );
        }
        assert_eq!(config.agent_types.len(), expected.len());
    }

    #[test]
    fn adapter_hint_known() {
        assert!(adapter_hint("claude").is_some());
        assert!(adapter_hint("codex").is_some());
        assert!(adapter_hint("pi").is_some());
        assert!(adapter_hint("gemini").is_none());
        assert!(adapter_hint("unknown").is_none());
    }

    #[test]
    fn adapter_hint_install_cmd() {
        let hint = adapter_hint("claude").unwrap();
        assert_eq!(hint.adapter, "claude-code-acp");
        assert!(hint.install.contains("@zed-industries/claude-code-acp"));
    }
}
