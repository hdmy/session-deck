# Session Deck

Session Deck 是一个本地优先的桌面应用，用来整理、搜索、阅读和继续 AI 编程会话。它从本机 AI coding agent 的历史文件或数据库中建立可重建索引，把零散的 Session 组织成 `Project → Agent → Session → Context`，并提供受控的历史续聊入口。

> 当前项目仍处于早期开发阶段。Provider 解析、索引和阅读能力已覆盖主要本地场景，但不同 Agent 的续聊能力取决于其原生 CLI 是否已接入。

## 中文

### 解决什么问题

AI 编程会话通常分散在不同 Agent 的本地目录中。Session Deck 让这些历史变成一个可检索的本地工作台：

- 跨项目、跨 Agent 搜索 Session、标题、用户输入和 Agent 回复。
- 以两栏界面浏览项目树、Session 元数据和完整上下文。
- 用 Focus 模式查看用户问题与最终回复，用 Full 模式查看原始时间线、thinking、tool use 和 tool result。
- 阅读主分支、alternate branch、compact/rewind 信息，以及文件变更摘要。
- 隐藏、重命名、pin 和最近使用排序只影响 Session Deck 本地索引，不修改 Provider 原始记录。
- 从历史会话生成本地确定性的 summary、topics、tags、决策、排障记录和知识卡片。
- 使用本地 256 维特征向量做语义相似度搜索，发现相关 Session、重复问题和解决方案。
- 在明确用户操作后，通过 Provider 自己的能力继续 Claude 会话；支持实时 transcript tail 和非破坏性 fork。

### 当前支持的 Provider

| Provider | 本地数据 | 阅读 | 全局搜索 | 续聊 / 分叉 |
| --- | --- | --- | --- | --- |
| Claude Code | `~/.claude/projects` 下的 JSONL | 支持 | 支持 | 支持 `resume`、`fork` 和运行中 tail |
| Codex | `~/.codex/sessions` 下的 JSONL | 支持 | 支持 | 当前未接入 |
| Gemini CLI | `~/.gemini/tmp` 下的 Session 文件 | 支持 | 支持 | 当前未接入 |
| OpenCode | 本地 `opencode.db` SQLite 数据库 | 支持 | 支持 | 当前未接入 |

Provider 数据只作为只读来源。Session Deck 不登录 Agent、不上传 transcript，也不直接重写 Provider 文件。

### 安全与数据边界

- 原始 transcript 和 Provider 数据库是 source of truth，Session Deck 只读访问。
- SQLite/FTS5 是可删除、可重建的本地派生索引，不是原始记录的替代品。
- 只有完整扫描成功并确认源文件消失后，索引才会清理对应记录；失败或未授权扫描不会被当成删除。
- 扫描和续聊互斥；续聊结束后再重新索引对应 Session。
- Claude 续聊使用校验过的 executable、历史 `cwd`、native session ID 和参数数组，不接受任意 shell 字符串。
- `--dangerously-skip-permissions` 默认关闭，启用时需要显式风险确认。
- Markdown 预览会清理可执行 HTML、外部资源、媒体和 iframe，避免自动加载网络内容。
- 不包含 telemetry、analytics、云同步、登录、远程控制或远程模型调用。

### 技术架构

```text
Provider source（只读）
  -> discovery + loss-tolerant parser
  -> normalized sessions / branches / turns
  -> provider-scoped reconciliation
  -> SQLite + FTS5 + local knowledge vectors
  -> typed Tauri commands
  -> Vue navigation / reader / terminal
```

- `src-tauri/src/providers/`：Provider 数据发现和解析适配器。
- `src-tauri/src/scanner/`：只读枚举、指纹和扫描生命周期。
- `src-tauri/src/indexer/`：归一化数据和事务性 reconciliation。
- `src-tauri/src/storage/`：SQLite schema、migration、read model 和搜索。
- `src-tauri/src/knowledge/`：本地确定性知识提取和向量生成。
- `src-tauri/src/runtime/`：校验后的 continuation spec、PTY 生命周期和进程事件。
- `src-tauri/src/commands.rs`：窄而类型化的 Tauri command 边界。
- `src/features/`：Vue 3 的浏览、阅读、知识卡片、设置和终端界面。

技术栈：Tauri 2、Vue 3、TypeScript、Rust、SQLite/FTS5、`marked`、`DOMPurify` 和 `xterm`。

### 本地开发

前置依赖：Node.js、pnpm、Rust，以及当前平台运行 Tauri 2 所需的系统依赖。

```bash
pnpm install
pnpm tauri dev
```

常用检查：

```bash
pnpm typecheck
pnpm test
pnpm build
cd src-tauri && cargo test --all-targets
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

真实数据 smoke test 默认忽略，只输出聚合统计，不输出 prompt、回复、工具结果或 transcript 内容：

```bash
cd src-tauri && cargo test --test real_data -- --ignored --nocapture
```

### 设计决策

- [ADR 0001：本地派生索引与安全 reconciliation](docs/adr/0001-local-derived-index.md)
- [ADR 0002：Provider-owned continuation](docs/adr/0002-provider-owned-continuation.md)
- [ADR 0003：conversation graph 与 read model](docs/adr/0003-conversation-read-model.md)

## English

### What it does

AI coding history is usually split across several local agent directories. Session Deck turns that history into a searchable local workspace:

- Search sessions, titles, user prompts, and agent responses across projects and providers.
- Browse a two-column project/session navigator and context reader.
- Use Focus mode for prompts and final responses, or Full mode for the original timeline, thinking, tool use, and tool results.
- Inspect main and alternate branches, compact/rewind hints, and file-change summaries.
- Hide, rename, pin, and sort sessions locally without changing provider-owned records.
- Generate deterministic local summaries, topics, tags, decisions, troubleshooting notes, and editable knowledge cards.
- Use local 256-dimensional feature vectors for semantic similarity search across related sessions.
- Continue Claude sessions only after an explicit user action, with live transcript tailing and non-destructive forking.

### Supported providers

| Provider | Local source | Reader | Global search | Resume / fork |
| --- | --- | --- | --- | --- |
| Claude Code | JSONL under `~/.claude/projects` | Yes | Yes | `resume`, `fork`, and live tail |
| Codex | JSONL under `~/.codex/sessions` | Yes | Yes | Not connected yet |
| Gemini CLI | Session files under `~/.gemini/tmp` | Yes | Yes | Not connected yet |
| OpenCode | Local `opencode.db` SQLite database | Yes | Yes | Not connected yet |

Provider data is read-only. Session Deck does not log in to agents, upload transcripts, or rewrite provider-owned files.

### Safety and data boundaries

- Provider transcripts and databases remain the source of truth; Session Deck reads them without modifying them.
- SQLite/FTS5 is a rebuildable local derived index, not an archive of record.
- Indexed records are removed only after a complete scan confirms that their source disappeared; failed or unauthorized scans never imply deletion.
- Scanning and continuation are mutually exclusive. The affected session is re-indexed after continuation exits.
- Claude continuation uses a validated executable, historical `cwd`, native session ID, and argument array instead of an arbitrary shell string.
- `--dangerously-skip-permissions` is off by default and requires explicit risk acknowledgement.
- Markdown preview sanitizes executable HTML, external resources, media, and iframes.
- There is no telemetry, analytics, cloud sync, login, remote control, or remote model call.

### Architecture

```text
read-only provider source
  -> discovery + loss-tolerant parser
  -> normalized sessions / branches / turns
  -> provider-scoped reconciliation
  -> SQLite + FTS5 + local knowledge vectors
  -> typed Tauri commands
  -> Vue navigation / reader / terminal
```

- `src-tauri/src/providers/`: provider discovery and parser adapters.
- `src-tauri/src/scanner/`: read-only enumeration, fingerprints, and scan lifecycle.
- `src-tauri/src/indexer/`: normalized data and transactional reconciliation.
- `src-tauri/src/storage/`: SQLite schema, migrations, read models, and search.
- `src-tauri/src/knowledge/`: deterministic local knowledge extraction and vectors.
- `src-tauri/src/runtime/`: validated continuation specs, PTY lifecycle, and process events.
- `src-tauri/src/commands.rs`: narrow typed Tauri command boundary.
- `src/features/`: Vue 3 browsing, reader, knowledge-card, settings, and terminal UI.

Stack: Tauri 2, Vue 3, TypeScript, Rust, SQLite/FTS5, `marked`, `DOMPurify`, and `xterm`.

### Local development

Prerequisites: Node.js, pnpm, Rust, and the platform dependencies required by Tauri 2.

```bash
pnpm install
pnpm tauri dev
```

Useful checks:

```bash
pnpm typecheck
pnpm test
pnpm build
cd src-tauri && cargo test --all-targets
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

The real-data smoke test is ignored by default and prints aggregate counts only; it does not print prompts, responses, tool output, or transcript excerpts:

```bash
cd src-tauri && cargo test --test real_data -- --ignored --nocapture
```

### Architecture decisions

- [ADR 0001: Local derived index and safe reconciliation](docs/adr/0001-local-derived-index.md)
- [ADR 0002: Provider-owned continuation](docs/adr/0002-provider-owned-continuation.md)
- [ADR 0003: Conversation graph and read model](docs/adr/0003-conversation-read-model.md)
