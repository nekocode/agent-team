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

/// Unix: uid, Windows: pid
fn platform_id() -> u32 {
    #[cfg(unix)]
    { unsafe { libc::getuid() } }
    #[cfg(not(unix))]
    { std::process::id() }
}

// ==================== Agent 注册表（声明式） ====================

struct AgentDef {
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    /// 需要适配器时的安装命令
    install_hint: Option<&'static str>,
}

const AGENT_REGISTRY: &[AgentDef] = &[
    // -- 原生 ACP：--acp flag --
    AgentDef { name: "copilot",    command: "copilot",    args: &["--acp"], install_hint: None },
    AgentDef { name: "auggie",     command: "auggie",     args: &["--acp"], install_hint: None },
    AgentDef { name: "cline",      command: "cline",      args: &["--acp"], install_hint: None },
    AgentDef { name: "qoder",      command: "qodercli",   args: &["--acp"], install_hint: None },
    AgentDef { name: "qwen",       command: "qwen",       args: &["--acp"], install_hint: None },
    // -- 原生 ACP：--experimental-acp flag --
    AgentDef { name: "gemini",     command: "gemini",     args: &["--experimental-acp"], install_hint: None },
    AgentDef { name: "blackbox",   command: "blackbox",   args: &["--experimental-acp"], install_hint: None },
    // -- 原生 ACP：acp 子命令 --
    AgentDef { name: "goose",      command: "goose",      args: &["acp"], install_hint: None },
    AgentDef { name: "kiro",       command: "kiro-cli",   args: &["acp"], install_hint: None },
    AgentDef { name: "openhands",  command: "openhands",  args: &["acp"], install_hint: None },
    AgentDef { name: "opencode",   command: "opencode",   args: &["acp"], install_hint: None },
    AgentDef { name: "kimi",       command: "kimi",       args: &["acp"], install_hint: None },
    AgentDef { name: "cagent",     command: "cagent",     args: &["acp"], install_hint: None },
    AgentDef { name: "stakpak",    command: "stakpak",    args: &["acp"], install_hint: None },
    AgentDef { name: "vtcode",     command: "vtcode",     args: &["acp"], install_hint: None },
    // -- 独立 ACP 二进制 --
    AgentDef { name: "vibe",       command: "vibe-acp",       args: &[], install_hint: None },
    AgentDef { name: "fast-agent", command: "fast-agent-acp", args: &[], install_hint: None },
    // -- 需要适配器 --
    AgentDef { name: "claude", command: "claude-code-acp", args: &[], install_hint: Some("npm install -g @zed-industries/claude-code-acp") },
    AgentDef { name: "codex",  command: "codex-acp",       args: &[], install_hint: Some("npm install -g @zed-industries/codex-acp") },
    AgentDef { name: "pi",     command: "pi-acp",          args: &[], install_hint: Some("npm install -g pi-acp") },
];

impl Default for TeamConfig {
    fn default() -> Self {
        let agent_types = AGENT_REGISTRY
            .iter()
            .map(|def| {
                (
                    def.name.to_string(),
                    AgentTypeConfig {
                        command: def.command.to_string(),
                        default_args: def.args.iter().map(|s| s.to_string()).collect(),
                    },
                )
            })
            .collect();

        let id = platform_id();
        Self {
            auto_approve: AutoApprovePolicy::Never,
            output_buffer_size: 10000,
            agent_types,
            default_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            socket_dir: std::env::temp_dir().join(format!("agent-team-{}", id)),
        }
    }
}

// ==================== 适配器提示 ====================

/// 需要额外适配器的 agent，返回安装提示
pub fn adapter_hint(agent_type: &str) -> Option<(&'static str, &'static str)> {
    AGENT_REGISTRY
        .iter()
        .find(|d| d.name == agent_type)
        .and_then(|d| d.install_hint.map(|hint| (d.command, hint)))
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
        let (cmd, install) = adapter_hint("claude").unwrap();
        assert_eq!(cmd, "claude-code-acp");
        assert!(install.contains("@zed-industries/claude-code-acp"));
    }
}
