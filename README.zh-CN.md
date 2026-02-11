# agent-team

多 Agent CLI 编排器，基于 [ACP](https://github.com/anthropics/agent-client-protocol)（Agent Client Protocol）协议。在一个终端统一管理 20+ 编程 Agent。

[English](README.md)

## 为什么需要

同时使用多个 AI 编程 agent 意味着在不同终端、上下文和工作流之间来回切换。agent-team 统一了这一切：

- **统一协议**：所有 agent 通过 ACP 通信 — prompt、取消、权限审批、配置变更，一套接口搞定
- **独立 session**：每个 agent 运行在独立进程中，拥有独立的 UDS socket，互不干扰
- **远程操控**：从任意终端发送 prompt、审批权限、查看日志

## 安装

```bash
npm install -g agent-team
```

更新到最新版本：

```bash
agent-team update
```

## 快速开始

```bash
# 启动一个 Gemini agent（前台运行，Ctrl+C 退出）
agent-team add gemini

# 后台启动一个 Claude agent
agent-team add claude -b

# 列出运行中的 agent
agent-team ls

# 发送 prompt
agent-team ask gemini-1 "重构 auth 模块"

# 查看对话记录
agent-team log gemini-1
```

## 支持的 Agent

支持 [agentclientprotocol.com](https://agentclientprotocol.com/get-started/agents) 列出的所有 agent：

| 类型 | 命令 | ACP 模式 |
|------|------|----------|
| gemini | `gemini` | `--experimental-acp` |
| copilot | `copilot` | `--acp` |
| goose | `goose` | `acp` 子命令 |
| claude | `claude-code-acp` | 适配器* |
| codex | `codex-acp` | 适配器* |
| auggie | `auggie` | `--acp` |
| kiro | `kiro-cli` | `acp` 子命令 |
| cline | `cline` | `--acp` |
| blackbox | `blackbox` | `--experimental-acp` |
| openhands | `openhands` | `acp` 子命令 |
| qoder | `qodercli` | `--acp` |
| opencode | `opencode` | `acp` 子命令 |
| kimi | `kimi` | `acp` 子命令 |
| vibe | `vibe-acp` | 独立二进制 |
| qwen | `qwen` | `--acp` |
| cagent | `cagent` | `acp` 子命令 |
| fast-agent | `fast-agent-acp` | 独立二进制 |
| stakpak | `stakpak` | `acp` 子命令 |
| vtcode | `vtcode` | `acp` 子命令 |
| pi | `pi-acp` | 适配器* |

\* 标记为**适配器**的 agent 需要安装额外的 wrapper 二进制。如果 PATH 中找不到适配器，agent-team 会提示安装命令。

## 命令

### Session 管理

| 命令 | 描述 |
|------|------|
| `add <type>` | 启动 agent session（前台）。`-b` 后台运行 |
| `rm <name>` | 关闭 agent。`--all` 关闭全部 |
| `ls` | 列出运行中的 agent |
| `restart <name>` | 重启 agent（保留配置） |
| `info <name>` | 显示 agent 详情 |

### 交互

| 命令 | 描述 |
|------|------|
| `ask <name> [text]` | 发送 prompt 并等待回复。`-f` 附加文件 |
| `log <name>` | 查看对话记录。`-n N` 最后 N 条，`-a` 仅 agent 输出 |
| `cancel <name>` | 取消当前任务 |
| `allow/deny <name>` | 审批权限请求。`--all` 批量审批 |

### 配置

| 命令 | 描述 |
|------|------|
| `mode <name> <mode>` | 切换 agent 模式（ask/code/architect） |
| `set <name> <key> <val>` | 修改运行时配置 |
| `update` | 通过 npm 自更新 |

## 许可证

MIT
