# agent-team

A multi-agent CLI orchestrator via the [ACP](https://github.com/anthropics/agent-client-protocol) (Agent Client Protocol). Manage 20+ coding agents from one terminal.

[中文文档](README.zh-CN.md)

## Why

Working with multiple AI coding agents today means juggling separate terminals, contexts, and workflows. agent-team unifies them:

- **One protocol**: All agents speak ACP — a standard interface for prompting, cancelling, permissions, and configuration
- **Independent sessions**: Each agent runs in its own process with its own UDS socket. No shared state, no interference
- **Remote control**: Send prompts, review permissions, read logs — all from any terminal

## Install

```bash
npm install -g agent-team
```

Update to the latest version:

```bash
agent-team update
```

## Quick Start

```bash
# Start a Gemini agent (foreground, Ctrl+C to exit)
agent-team add gemini

# Start a Claude agent in the background
agent-team add claude -b

# List running agents
agent-team ls

# Send a prompt
agent-team ask gemini-1 "Refactor the auth module"

# Read the conversation
agent-team log gemini-1
```

## Supported Agents

All agents listed on [agentclientprotocol.com](https://agentclientprotocol.com/get-started/agents):

| Type | Command | ACP Mode |
|------|---------|----------|
| gemini | `gemini` | `--experimental-acp` |
| copilot | `copilot` | `--acp` |
| goose | `goose` | `acp` subcommand |
| claude | `claude-code-acp` | Adapter* |
| codex | `codex-acp` | Adapter* |
| auggie | `auggie` | `--acp` |
| kiro | `kiro-cli` | `acp` subcommand |
| cline | `cline` | `--acp` |
| blackbox | `blackbox` | `--experimental-acp` |
| openhands | `openhands` | `acp` subcommand |
| qoder | `qodercli` | `--acp` |
| opencode | `opencode` | `acp` subcommand |
| kimi | `kimi` | `acp` subcommand |
| vibe | `vibe-acp` | Standalone binary |
| qwen | `qwen` | `--acp` |
| cagent | `cagent` | `acp` subcommand |
| fast-agent | `fast-agent-acp` | Standalone binary |
| stakpak | `stakpak` | `acp` subcommand |
| vtcode | `vtcode` | `acp` subcommand |
| pi | `pi-acp` | Adapter* |

\* Agents marked **Adapter** require a separate wrapper binary. agent-team will prompt you with install instructions if the adapter is not found in PATH.

## Commands

### Session Management

| Command | Description |
|---------|-------------|
| `add <type>` | Start agent session (foreground). `-b` for background |
| `rm <name>` | Shut down agent. `--all` for all agents |
| `ls` | List running agents |
| `restart <name>` | Restart agent (preserves config) |
| `info <name>` | Show agent details |

### Interaction

| Command | Description |
|---------|-------------|
| `ask <name> [text]` | Send prompt and wait for response. `-f` to attach files |
| `log <name>` | Read conversation. `-n N` for last N messages, `-a` for agent-only |
| `cancel <name>` | Cancel current task |
| `allow/deny <name>` | Approve or reject permission requests. `--all` for batch |

### Configuration

| Command | Description |
|---------|-------------|
| `mode <name> <mode>` | Switch agent mode (ask/code/architect) |
| `set <name> <key> <val>` | Change runtime config |
| `update` | Self-update via npm |

## License

MIT
