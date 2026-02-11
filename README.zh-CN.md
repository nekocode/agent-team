# agent-team

多 Agent CLI 编排器，基于 [ACP](https://agentclientprotocol.com/get-started/introduction)（Agent Client Protocol）协议。在一个终端统一管理 20+ 编程 Agent。

[English](README.md)

![Cover](cover.jpg)

## 为什么需要

每个编程 agent 都有自己的 CLI、自己的工作流、自己的一套玩法。agent-team 给它们一个统一的控制面：

- **一套接口管所有 agent**：prompt、取消、权限审批、配置变更 — 同一套命令适用于 20+ agent
- **独立 session**：每个 agent 运行在独立进程中，拥有独立的 UDS socket，互不干扰
- **随时随地操控**：从任意终端发送 prompt、审批权限、查看日志

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

`gemini` `copilot` `goose` `claude`\* `codex`\* `auggie` `kiro` `cline` `blackbox` `openhands` `qoder` `opencode` `kimi` `vibe` `qwen` `cagent` `fast-agent` `stakpak` `vtcode` `pi`\*

\* 需要安装额外的适配器二进制。如果 PATH 中找不到，agent-team 会提示安装命令。

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
| `allow/deny <name>` | 审批权限请求 |

### 配置

| 命令 | 描述 |
|------|------|
| `mode <name> <mode>` | 切换 agent 模式（ask/code/architect） |
| `set <name> <key> <val>` | 修改运行时配置 |
| `update` | 通过 npm 自更新 |

## 配合 AI Agent 使用

### 直接告诉 agent

最简单的方式 — 直接让 agent 去用：

```
使用 agent-team 管理编程 agent。运行 agent-team --help 查看用法。
```

`--help` 输出足够详细，大多数 agent 看完就能上手。

### AGENTS.md / CLAUDE.md

写到项目指令文件里，效果更稳定：

````markdown
## 多 Agent 编排

使用 `agent-team` 启动和协调编程 agent。运行 `agent-team --help` 查看所有选项。

用法：
- `agent-team add gemini -b` - 后台启动 Gemini agent
- `agent-team ls` - 列出运行中的 agent
- `agent-team ask <name> "task"` - 发送 prompt 并等待回复
- `agent-team log <name> -a -n 1` - 查看最后一条 agent 回复
- `agent-team cancel <name>` - 取消当前任务
- `agent-team allow/deny <name>` - 审批权限请求
- `agent-team rm <name>` - 关闭 agent

典型流程：
1. 启动 agent：`agent-team add gemini -b && agent-team add copilot -b`
2. 分配任务：`agent-team ask gemini-1 "重构 auth 模块"`
3. 查看进展：`agent-team log gemini-1`
4. 清理：`agent-team rm --all`
````

## 许可证

MIT
