# Context Vault 任务清单

> 按此前确认的优先级整理。`[x]` 表示已实现并完成自动化验证，`[ ]` 表示待做或待人工验收。
>
> 更新时间：2026-08-07

## 当前里程碑

- [x] P0：Claude Code 历史会话索引、浏览、搜索、Markdown 阅读和受控续聊已实现。
- [x] P0：原始 Claude JSONL 保持只读；续聊时只允许由 Claude CLI 写回自己的 transcript。
- [x] P0：扫描与续聊互斥，续聊结束后重新索引同一个 Session。
- [x] P0：Rust、Vue、真实 Claude 数据 smoke、Tauri release build 已通过。
- [ ] P0：用户实际启动桌面应用并完成一次人工验收。

## P0 — MVP 与 Claude 受控续聊

### 产品边界与架构

- [x] 确定本地优先 Desktop App 形态：Tauri 2 + Vue 3 + Rust。
- [x] 建立 provider-neutral 模块边界：scanner、parser、indexer、storage、search、viewer、runtime。
- [x] 参考 Claude Code VS Code extension 的历史浏览与续聊能力，吸收 resume/terminal 等本地能力，同时排除登录、云端和远程控制。
- [x] 建立 Claude continuation ADR：Context Vault 不重写 JSONL，续聊由 Claude CLI 负责。
- [x] 建立 conversation graph/read-model ADR：文件顺序不直接等于用户看到的对话顺序。
- [x] 遵循 Claude 源文件保留逻辑：源文件被清理后，完整扫描才从本地派生索引中移除对应记录。

### Claude 数据扫描与解析

- [x] 扫描 `~/.claude/projects/<project-slug>/*.jsonl`。
- [x] 自动识别项目、项目路径、项目名称和 Session 数量。
- [x] 排除 `subagents` 等嵌套 transcript，避免子 Agent 被误显示为顶层 Session。
- [x] 解析 Session 标题：custom title、AI title、首条用户 prompt 逐级回退。
- [x] 解析首条/最后一条用户输入、开始/结束时间、修改时间、cwd、branch、模型、tool count。
- [x] 解析 Claude `uuid`、`parentUuid`、`message.id`、`tool_use_id`、`tool_result` 等关联字段。
- [x] 处理 compact boundary、preserved messages、assistant fragments 和主分支选择。
- [x] 过滤 meta、team、sidechain 节点，同时保留可读的主链活动。
- [x] 对 malformed JSON、partial tail 和 source changed during scan 做容错处理。

### 存储、索引与搜索

- [x] 使用 SQLite 保存可重建的派生 read model。
- [x] 保存 provider ID 与 provider-native session ID，避免未来多 Agent ID 冲突。
- [x] 保存 timeline lineage、conversation turns、turn activities 和 final response 标记。
- [x] 使用 FTS5 支持项目名、Session 标题、用户 prompt、Agent 回复的全局搜索。
- [x] 支持中文短查询和关键词结果定位。
- [x] 完整扫描成功时安全 reconciliation；失败或不完整扫描不执行删除。
- [x] 测试证明源文件扫描/解析不会修改内容、mtime 或 size。

### 两栏浏览体验

- [x] 左栏合并全局搜索、Projects 和 Sessions，右栏显示 Context Reader。
- [x] 项目树支持展开/折叠、Session 数量和最新活动信息。
- [x] 项目悬浮信息包含名称、路径和最近更新时间。
- [x] Session 悬浮信息包含日期、branch 和最后一次有意义的用户输入。
- [x] Session 列表显示可读标题，不只显示 session ID。
- [x] Reader 支持 Focus / Full 两种投影。
- [x] Focus 模式按用户 turn 显示 prompt、最终回复和折叠后的活动摘要。
- [x] Full 模式保留用户、Agent、thinking、tool use、tool result 的原始顺序。
- [x] 完成的 turn 只将最后一段 assistant 文本作为主要最终响应。
- [x] 提供 loading、error、empty、partial parse 状态。

### Markdown 预览

- [x] 最终 Agent 回复支持 Rendered Markdown 和 Raw 两种查看模式。
- [x] 支持复制原始 Markdown。
- [x] 使用 `marked` + `DOMPurify`，不渲染可执行 HTML。
- [x] 移除 `href`、`src`、media、iframe 等外部资源和导航属性。
- [x] 保护 fenced code / inline code 中的 HTML-like 代码，避免代码内容被误删。

### Claude 受控续聊

- [x] 使用历史 cwd 和 native session ID 构造 `claude --resume <session-id>`。
- [x] 只接受经过校验的 executable、cwd、UUID 和直接参数数组，不接受 shell 字符串。
- [x] 全局 Settings 支持自动解析 Claude executable 或自定义 executable path。
- [x] `--dangerously-skip-permissions` 默认关闭。
- [x] 开启危险权限前要求显式风险确认；P0 不提供自由参数输入框。
- [x] 提供 preflight：可执行文件、版本、历史 cwd 和只读 command preview。
- [x] 右栏底部嵌入 PTY terminal，支持输入、resize、最小化和关闭。
- [x] PTY 输出以原始字节传输，避免跨 chunk 的 UTF-8 损坏。
- [x] 限制 PTY 输入、输出轮询大小和 resize 范围。
- [x] scan 与 continuation 通过同一 lifecycle gate 互斥。
- [x] 处理自然退出、错误、组件卸载、stale start 和 stale poll 的句柄回收。
- [x] 续聊结束或关闭后刷新索引并重新选择原 Session。
- [x] 暂不实现 live transcript tail；首版体验为“终端运行，结束后同步历史”。

### P0 验证与人工验收

- [x] Rust unit/parser/storage 测试通过。
- [x] Vue tests、`vue-tsc`、Vite production build 通过。
- [x] `cargo clippy --all-targets -- -D warnings` 通过。
- [x] 真实 `~/.claude/projects` 只读 smoke 通过，输出仅包含聚合统计。
- [x] Tauri release binary 构建通过。
- [x] 两栏布局、Settings、Markdown 和 terminal dock 完成无 Tauri 后端的可视化验收。
- [ ] 启动桌面应用，确认真实项目树和 Session 标题符合预期。
- [ ] 搜索一个历史关键词（例如 `CloudBase`），确认能跨项目定位 Session。
- [ ] 打开真实 Session，确认同一轮对话没有被错误拆成多个顶层记录。
- [ ] 手动执行一次 preflight，并在确认后测试一次真实 `继续对话`。
- [ ] 确认续聊结束后 Session 列表和 Reader 出现新 turn。

## P1 — 本地历史管理与体验增强

> P0 人工验收通过后再开始。P1 仍然保持本地优先，不修改原始 JSONL。

### 本地 Session 管理

- [x] “隐藏” Session：只从 Context Vault 本地索引隐藏，不删除 Claude 原始记录。
- [x] 取消隐藏 / 恢复隐藏 Session。
- [x] 本地重命名 Session 标题，不写入原始 JSONL。
- [x] 本地 pin / 最近使用排序。
- [x] 明确区分“从索引隐藏”和“源文件已被 Claude 清理”。

### 历史阅读与续聊体验

- [x] live transcript tail：续聊过程中增量读取新写入的 JSONL。
- [x] 续聊期间显示当前 Session、进程状态和不可用操作原因。
- [x] Fork / branch continuation：通过 provider-owned 能力创建非破坏性分支，并在本地 read model 中保留关系。
- [x] Rewind / alternate branch reader：浏览或恢复历史分支，不直接改写原始 JSONL。
- [x] 文件变更与代码修改摘要视图，关联到对应 turn 和 tool activity。
- [x] 更细的 tool summary 和活动统计。
- [x] Session 内部关键词高亮、上下文定位和快捷键完善。
- [x] 对被 compact / rewind 影响的历史提供更明确的分支提示。

### 索引与项目体验

- [x] 文件监听或增量扫描，减少完整扫描成本。
- [x] 项目别名、worktree 归并和 cwd 变化的展示策略。
- [x] 可配置 Claude source root 与扫描频率。
- [x] 更完整的历史数据变化提示和索引诊断面板。

## P2 — 多 Agent 与知识库增强

> P2 需要在 P1 的本地管理模型稳定后再拆分实现。

### 多 Agent Provider

- [ ] Codex session provider adapter。
- [ ] Gemini CLI session provider adapter。
- [ ] OpenCode session provider adapter。
- [ ] provider capabilities 映射：reader、search、resume、branching、worktree。
- [ ] 统一 Project → Agent → Session → Context 展示模型。
- [ ] 跨 Agent 全局搜索和 provider 筛选。

### AI Coding 工作历史知识库

- [ ] AI 自动生成 Session summary、主题和标签。
- [ ] 基于历史内容的语义搜索 / 向量索引。
- [ ] 相关 Session、重复问题和解决方案关联。
- [ ] 从 Session 提取可复用的架构决策、排障经验和代码变更摘要。
- [ ] 可人工编辑的知识卡片与引用回链。

## 明确不纳入当前计划

以下内容不作为 P1/P2 的默认开发项，除非产品方向重新确认：

- 登录、用户系统、多人协作。
- 云同步、远程 Session、Remote Teleport。
- 将代码、prompt、回复或 transcript 上传到服务器。
- Context Vault 直接修改、删除或重写 Claude 原始 JSONL。
- 在 Context Vault 内重实现 Claude Agent、权限系统或完整 streaming SDK。

## 开始下一阶段的门槛

只有在 P0 人工验收完成并确认体验没有方向性问题后，才开始 P1。建议顺序：

1. [ ] 先实现“隐藏”与本地索引状态模型。
2. [ ] 再实现 live tail 和续聊期间的状态同步。
3. [ ] 再补项目/worktree 与索引增量能力。
4. [ ] 最后评估 Codex、Gemini CLI、OpenCode provider 的数据样本和 parser 方案。
