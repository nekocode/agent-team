use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "agent-team", about = "Multi-agent orchestrator via ACP")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start a new agent session
    Add {
        /// Agent type [gemini, copilot, goose, claude, codex, auggie, kiro,
        /// cline, blackbox, openhands, qoder, opencode, kimi, vibe, qwen,
        /// cagent, fast-agent, stakpak, vtcode, pi]
        agent_type: String,

        /// Custom agent name
        #[arg(long)]
        name: Option<String>,

        /// Working directory for the agent
        #[arg(long)]
        cwd: Option<PathBuf>,

        /// Extra arguments passed to the agent process
        #[arg(long)]
        args: Option<String>,

        /// Run in background (detach from terminal)
        #[arg(long, short = 'b')]
        background: bool,
    },

    /// Shut down an agent
    Rm {
        /// Agent name
        name: String,

        /// Shut down all agents
        #[arg(long)]
        all: bool,
    },

    /// List running agents
    Ls,

    /// Send a prompt to an agent (reads stdin if text omitted)
    Ask {
        /// Agent name
        name: String,

        /// Prompt text (omit to read from stdin)
        text: Option<String>,

        /// Attach file content
        #[arg(long, short = 'f')]
        file: Vec<PathBuf>,
    },

    /// View agent output history
    Log {
        /// Agent name
        name: String,

        /// Show last N messages (0 = all, default: 1)
        #[arg(long, short = 'n', default_value = "1")]
        last: usize,

        /// Show only agent messages (exclude user prompts)
        #[arg(long, short = 'a')]
        agent_only: bool,
    },

    /// Cancel current task
    Cancel {
        /// Agent name
        name: String,
    },

    /// Allow pending permission
    Allow {
        /// Agent name
        name: String,
    },

    /// Deny pending permission
    Deny {
        /// Agent name
        name: String,
    },

    /// Show agent details
    Info {
        /// Agent name
        name: String,
    },

    /// Restart agent process
    Restart {
        /// Agent name
        name: String,
    },

    /// Switch agent mode (e.g. ask, code, architect)
    Mode {
        /// Agent name
        name: String,

        /// Mode ID (e.g. ask, code, architect)
        mode: String,
    },

    /// Set agent config at runtime
    Set {
        /// Agent name
        name: String,

        /// Config key (e.g. model, thinking_budget_tokens)
        key: String,

        /// Config value
        value: String,
    },

    /// Update agent-team to latest version
    Update,
}
