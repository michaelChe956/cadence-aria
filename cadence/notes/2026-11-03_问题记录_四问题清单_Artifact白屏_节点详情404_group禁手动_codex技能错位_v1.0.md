# 问题记录：四问题清单（Artifact 白屏 / timeline-node-details 大量 404 / work item group 禁手动生成模式 / codex 技能路径与清单不一致）

日期：2026-11-03
状态：**owner 裁定「先记录，不解决」** —— 本文件只做问题留痕，不开 OpenSpec change、不实施、不向 owner 索取现场取证数据
触发来源：owner 于 2026-11-03 一次性报出以下 4 个问题

> **元信息 1**：本记录的行号均基于 **main 分支**（commit 827a990，`.worktrees/main/`）；bugfix-b-11-02 分支同域代码行号可能偏移，引用时请按符号名定位。
>
> **元信息 2**：素材来源：2026-11-03 对 main 分支的只读勘察（codegraph + 精确 grep）；勘察未改任何代码，未做根因定案。

> **本文件性质**：四个问题的**问题留痕清单**。只做格式化与组织，事实/推测照录素材；素材中标 `[INFERENCE]` 或「推测」的内容在本文件中保持同样标注，**不得**被读作已验证事实。不含修法、不含方案、不含实施排期。

---

## P1 Artifact 白屏（首屏空白，刷新后恢复）

### owner 报告原话

> Artifact 会出现白屏的情况，刷新之后才会出现。

### 锚点（代码）

展示组件与数据链：

- `web/src/components/chat-workspace/ArtifactPane.tsx:18-31` —— `ArtifactPane` 由 `artifactVersions`（版本摘要列表）+ `artifact`（当前全文）两路 props 驱动；`selectedVersion` 初始值取 `latestVersion?.version ?? null`（`:31-33`）。
- `ArtifactPane.tsx:76` —— 内容兜底链 `markdown = selectedMarkdown ?? artifact ?? "等待 Artifact"`。
- `ArtifactPane.tsx:331-334` —— 版本存在但内容不可加载且无 sessionId 时渲染 `artifact-unavailable`「Artifact 内容未加载」（空白形态之一）。
- `web/src/pages/ChatCockpitPage.tsx:1245-1253` —— cockpit「产物审核」页签渲染 `ArtifactReviewPanel`，`artifactVersions={selectedState?.artifactVersions ?? []}`；关键分支 `sessionId={isCurrentSession ? sessionId : null}`（`:1249`）。
- `ArtifactPane.tsx:80-82` 附近 —— `canLoadArtifact = Boolean(currentSessionId && loadArtifactVersion)`。
- `ChatCockpitPage.tsx:977-983` —— effect：进入 `author_confirm` 等 stage 时自动切 `drilldownView="artifact"`。

数据来源：

- `web/src/state/workspace-ws-store.ts:272-273` —— `artifactVersions` 优先取 session_state 快照里的 `artifact_version_summaries`，缺省回退完整 `artifact_versions`。
- `workspace-ws-store.ts:794-807` —— live 期间只有 WS `artifact_update` 事件走 `upsertArtifactVersionSummary` 增量补版本（事件定义 `web/src/api/types/workspace.ts:355-383`）。
- `web/src/api/workspace-content.ts:52-56` —— `fetchWorkspaceArtifactVersion` → `GET /api/workspace-sessions/{sid}/artifact-versions/{version}`（内容本体是懒加载）。
- `src/web/handlers/workspace_session.rs:368-383` —— 上述接口的后端实现。
- `src/product/workspace_engine/session_state.rs:393` —— 后端快照组装面：`artifact_version_summaries` 进 session_state。

### 已知事实

- `ArtifactPane` 的内容展示走**兜底链**：版本正文缺失时依次退到 `artifact` 字段、再退到占位文案「等待 Artifact」；版本存在但内容不可加载且无 sessionId 时渲染「Artifact 内容未加载」。
- 版本摘要列表 `artifactVersions` **优先来自 session_state 快照**的 `artifact_version_summaries`；快照重建（刷新/重连）时才有这份完整列表。
- live 期间版本只能靠 WS `artifact_update` 事件**增量**补齐。
- 非当前会话时 `sessionId` 传 `null` → `canLoadArtifact=false` → 懒加载链路整个关闭，只剩 `artifact` 字段兜底。
- 进入 `author_confirm` 等 stage 会自动切到 `drilldownView="artifact"`。

### 未决 / 推测

- **候选空白分支（未定案，多个候选）**：① 快照 summaries 缺 markdown；② 非当前会话 `sessionId=null` 关掉懒加载；③ 自动切页签时版本列表尚未到达。
- 上述三条**均未定案**，本记录不排他、不选定其一；「刷新后恢复」这一现象本身已由快照/增量两条数据来源解释，但**未证明**是哪一条候选分支造成首屏空白。

---

## P2 `timeline-node-details` 大量 404

### owner 报告原话

> `GET /api/workspace-sessions/workspace_session_0012/timeline-node-details/timeline_node_001` 会有很多 404，不一定是 node_001，也可能是别的 node。

### 锚点（代码）

后端：

- 路由注册：`src/web/app.rs:531-534` → `handlers::workspace_session_timeline_node_detail`。
- Handler：`src/web/handlers/workspace_session.rs:326-334` —— 直接 `LifecycleStore::load_node_detail`，文件不存在即 NotFound。
- 存储：`src/product/lifecycle_store/workspace_timeline_artifacts.inc.rs:64-80` —— detail 是 `workspace_timeline_root/<session>/timeline_node_details/<node_id>.json` 单文件，不存在即 `NotFound{kind:"node_detail"}`。
- 错误映射：`src/web/handlers/support.rs:471-473` → `node_detail_not_found`；`src/web/error.rs:74-76` 归 404。
- detail 写入面：`src/product/workspace_engine/session_state_parts/timeline.rs:328-365`、`session_state_parts/stream.rs:30-138`（`update_node_detail` / `save_node_detail`）。
- 快照装配：`src/product/workspace_engine/session_state.rs:312-350`（快照也只装已有 detail）。

前端：

- `web/src/components/chat-workspace/TimelineNodeList.tsx:187-192` —— 「门卡/上下文类节点本就不产生 usage 事件」自认注释。
- `web/src/pages/useCockpitNodeDetailHydration.ts:19-25` —— `detailHydrationNodeIds` 用排除法（非 active、非 paused）对**所有**呈现节点拉 detail，**不做「该节点是否有 detail」预判**。
- `web/src/pages/useCockpitNodeDetailHydration.ts:63-67` —— effect 尾部 `.catch(() => hydratedNodeIdsRef.current.delete(nodeId))`。

### 已知事实

- detail 只由引擎在节点**运行期间**经 `update_node_detail` / `save_node_detail` 写入；**门卡/上下文类节点从不产生 detail** → 对这类节点请求 detail **天生 404（合法）**。
- 快照也只装已有 detail，因此「快照里有 / 没有 detail」与磁盘文件存在性一致。
- 前端两处放大器（已验证）：① 用排除法对所有呈现节点拉 detail，**不预判**该节点是否有 detail；② 404 后 `.catch` 把该 id 从去重集合删除 → `timelineNodes` / `activeNodeId` 每次变化都会对该节点**重新发起请求**。
- **结论性事实**：404 中相当一部分是**合法**的（节点天生无 detail），但前端①不预判、②404 后不记忆 → 同一节点反复请求，形成噪音；噪音量级与节点数正相关。

### 未决 / 推测

- 素材对 P2 **未给出推测性结论**（前端放大器部分标注为「已验证」）。
- 未定边界：404 中「合法 404」与「异常 404」的占比未量化；是否调整前端预判/记忆策略**不在本记录范围**（本记录不认定修法）。

---

## P3 work item group 不允许手动生成模式（产品决定，未实施）

### owner 裁定原话

> work item group 不允许手动生成模式，从弹窗里去掉。

### 锚点（代码）

- radio 渲染：`web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:294-327` —— `{automationAvailable ? <fieldset>生成模式…手动/自动}`；「手动」radio 与 `updateAutomationMode("manual")` 在 `:308-311`。
- `automationAvailable` 语义：`WorkItemPlanOptionsDialog.tsx:56-57`（「仅已确认 Design 显示自动化模式选择（缺省手动）」）；传入点 `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:1036-1040` = `card.kind === "design_spec" && card.raw.confirmation_status === "confirmed"`。
- 打开链路（两条，同一函数）：`web/src/components/lifecycle/useIssueLifecycleGeneration.ts:182-190` `openWorkItemPlanOptions(card)` → `pendingWorkItemPlanLaunch`；调用点 `:221-224`（`handleGenerateNext`，design_spec → 生成 Work Item Plan，**产出即 work_item_group 卡**）与 `:323-326`（`handleLaunchWorkspace("work_item", design_spec 卡)`）。
- group 卡（`kind:"work_item_group"`）本身不直接打开该弹窗 —— group 是 design_spec 卡走此弹窗生成的**结果**（卡片映射 `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx:952-954`，`work_item_group → workspace_type:"work_item_plan"`）。

### 已知事实（最小落点，只列落点不做方案）

1. `WorkItemPlanOptionsDialog.tsx:308-311` 的「手动」radio 分支（或 `:294-327` 整个 fieldset 固定为 automatic）；
2. 若只按「本次提交将生成 group」收紧：调用侧 `useIssueLifecycleGeneration.ts:597-601` 已有 `card.kind !== "design_spec"` 守卫可作参照；
3. `IssueLifecycleWorkbench.tsx:1036-1040` 的 `automationAvailable` 计算处。

### 性质与门禁

- **性质**：owner 可见行为变更（去掉一个用户可选模式）→ 按仓规**走 OpenSpec change 后实施**。
- **当前状态**：未实施。本记录只留痕，**不开 change、不改代码**。

---

## P4 codex 作为 provider 时固定输出「技能路径与清单再次不一致…」

### owner 报告原话

> codex 作为 provider 时总会出现：「技能路径与清单再次不一致；我改从技能根目录定位实际文件，并继续查找权威政策正文。未命中常规路径后，我扩大到隐藏目录和权威根外层，仅做定位不修改。」

### 锚点（代码）

- 提示词要求 provider「必调 Skill」的拼装点：`src/product/coding_workspace_engine/prompts.rs:422-423`、`:441-442`、`:491-492`、`:527-528`（「必调 Skill：using-superpowers → executing-plans / requesting-code-review…」）。
- workspace 侧提示词：`src/web/workspace_context/prompts.rs:96-101`、`:186-190`（「必须遵守 using-superpowers 与 writing-plans…若 provider 环境没有对应技能文件，也必须遵守本 prompt 内嵌的纪律摘要」）。
- 平台侧技能落盘（cadence_skills 层）：`src/product/cadence_skills/paths.rs:19-22` —— `shared_skills_root=~/.agents/skills`、`codex_skills_root=~/.codex/skills/skills`（**双层 skills/skills**）、`claude_skills_root=~/.claude/skills`。
- 技能同步：`src/product/cadence_skills/link_sync.rs:49-96` —— 把每个技能以 symlink 同步到三层根（codex 层指向 shared 层，`:84-88`）。
- 「权威政策正文」发布点：`src/web/handlers/aggregate_initialization/production_dependencies.inc.rs:329-333`（`publish_recipe_policy`，REQ-BOOT-05）与 `:522` `publish_and_verify_root_policy`；与技能链接同属五步协议（`src/product/logical_codebase/aggregate_initialization.rs:62-74`，`MachineSkills` 第一步）。
- codex 实际读技能的路径证据：`web/src/hooks/useWorkspaceWs.test.tsx:139`、`:357` —— provider 运行期发出的命令是
  `sed -n '1,220p' /home/michael/.codex/superpowers/skills/using-superpowers/SKILL.md`
  即 codex 环境的 superpowers 技能根是 **`~/.codex/superpowers/skills/`**。
- MachineSkills 落盘摘要：`src/product/logical_codebase/aggregate_initialization_coordinator.rs:1052-1056`（仅 `machine_skills.json` 摘要）。

### 已知事实

- 平台把 codex 技能链接放在 `~/.codex/skills/skills/<name>`（`paths.rs:21`），而 codex 运行期扫描/引用 `~/.codex/superpowers/skills/<name>` —— **不同根**，「清单里路径 ≠ 实际落盘路径」在此处成立（**已验证的两条路径并存**）。
- 提示词侧确实存在「必调 Skill」的硬要求（coding 侧 + workspace 侧两处）。
- 「技能清单文件由谁写」：**未在 main worktree 内找到**独立清单写入点（搜过 `skills_manifest`、`SKILL.md` 写入；MachineSkills 落盘仅 `machine_skills.json` 摘要）。清单更可能是 provider 侧（superpowers / AGENTS.md）自持，平台未写。此条为「**未找到 + 事实边界**」，**非定案**。

### 未决 / 推测

- **推测（[INFERENCE]，未验证）**：那句固定自述很可能是 codex 按 superpowers 清单逐个试探 `SKILL.md` 路径失败后的兜底话术；是平台 `~/.codex/skills/skills` 双层根造成、还是 codex 环境自身清单配置问题，**需真机对照 `~/.codex/` 实际目录树才能定案**。
- 以上为推测，**不得**按已验证事实引用。

---

## 关联

### P4 与既有票据的区分

- `cadence/notes/2026-11-01_待办_平台已知未修问题清单_v1.0.md` P1-14（skills 步失败 `stderr_summary` 恒空 → 报错是「no stderr summary」废话）：讲 **MachineSkills 步失败时错误信息被吞**（失败面 / 可观测性）；P4 是**成功路径上**的路径语义不一致 —— **不同层，相关非同根**。
- `cadence/notes/2026-10-29_问题记录_聚合注册预检与skills初始化耦合_v1.0.md:52-82`：聚合根做过 skills 初始化 → 注册预检撞死（注册阻塞）；与 P4 共享「skills 初始化写入物」这一对象 —— **同族（skills 初始化产物正确性）不同症状**。

### P1 与 2026-10-30《写入范围可见性与本地产物屏蔽》的关系

2026-10-30《写入范围可见性与本地产物屏蔽》讲 coder 写入过滤与 `.gitignore`（`local_artifacts.rs`、`prompts.rs`），属 **git / lint 面**，与 P1 的前端展示链**无直接代码交集** —— 仅**词面相关，未找到关联**。

### 同分支同日另两份问题记录

- `cadence/notes/2026-11-03_问题记录_计划生成停在人工分诊点_plan_generation_needs_human_v1.0.md`
- `cadence/notes/2026-11-03_问题记录_自动化目标预检忽略所选provider_多仓自动授权受阻_v1.0.md`
