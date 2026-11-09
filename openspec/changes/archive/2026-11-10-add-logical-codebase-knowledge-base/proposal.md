# Proposal

## Why

逻辑代码库（LC）目前只能把「聚合初始化」跑到规则/MCP/OpenSpec 示例为止，**没有知识库这一层**：LC 的成员仓规模一大，跨仓的事实（服务、数据模型、接口、页面、配置）就没有可查询、可追溯、可增量维护的载体；而 Cadence-skills 侧已经具备完整的 Schema 4.0 KnowledgeBase 能力（`knowledge-base-bootstrap` / `knowledge-base-update` / `knowledge-base-context` 等 7 个 skill），平台却没有任何入口去调用它们。

结果是两头的重复劳动：owner 只能回到 AI 会话里手动敲 skill 命令，平台侧看不到任何过程、失败面或产物；知识库文档散落在聚合根目录里，owner 想看、改、存，只能开编辑器手改文件，没有「当前 LC 下的知识库」这一产品面。

本 change 把这条链补齐：LC 仓管理内出现「知识库」模块，三个按钮（初始化 / 更新 / 查看详细内容）覆盖「建库 → 增量维护 → 阅读编辑」的完整闭环，并且把 Cadence-skills 的既有契约（用户显式指定变更包、变更包字段门禁、敏感信息门禁、Manifest 4.0 不变量）**在平台侧同样卡住**，不让平台侧成为绕过 skill 门禁的后门。

## What Changes

- **LC 仓管理新增「知识库」模块**：与「聚合初始化」「聚合索引」卡片同构的一张卡片，含三个用户可见按钮——【知识库初始化】【知识库更新】【查看知识库详细内容】。
- **知识库初始化**：由平台复用聚合初始化的 provider 选择器（同候选顺序 Pi > Claude Code > Codex、同一套 L1 命令可用 / L2 能力已核验判据）选定**一家** provider，由它执行 `/knowledge-base-bootstrap`，产物落在 **LC 聚合根** `<aggregate_root>/cadence/knowledge-base/`。
- **知识库更新**：**变更包由用户撰写**（平台不代写、provider 不代写）。平台提供变更包模板骨架创建、逐字段门禁与显式路径调用 `/knowledge-base-update <变更包目录>`。
- **查看知识库详细内容**：新路由页渲染当前 LC 下的知识库文档树，支持在页内编辑 Markdown 并保存（后端新增受路径授权约束的文件读写面）。
- **知识库状态投影**：新增 `GET .../knowledge-base/status`，把 `manifest.yaml` 的初始化不变量解析成结构化 DTO（YAML 只在 Rust 侧解析）。
- **模板骨架创建**：一键把 Cadence-skills 插件自带的输入模板（9 份）与变更包模板（5 份 + attachments）复制到目标位置；**只补缺失文件，绝不覆盖用户已填写的内容**。
- **变更包门禁（平台侧，P1–P9）**：路径形态、五份文档齐全、必填字段非空（枚举占位视为未填）、「不适用」须带原因、七行领域矩阵、敏感信息扫描、起止提交本地可解析、配置分支与 Manifest 一致、Manifest 八项不变量。平台只卡**机械可判定**的部分，语义一致性仍由 skill 门禁判定。
- **provider turn 的写权限通道**：知识库 provider turn 走 **Normal 相位 + `CodingTargetWrite`（target = canonical 聚合根）**，使「选择期通过」与「起跑被 gateway 放行」落在同一行能力矩阵；不改动聚合初始化冻结的 `AggregateBootstrap` 凭据链。
- **前置条件**：知识库初始化/更新要求**聚合初始化已完成**（根规则与已发布政策存在，且 Cadence-skills 已安装）——这是 Normal 相位的既有语义，也是 skill 可用的物理前提。
- **per-LC 全局单飞**：同一 LC 的 bootstrap 与 update 互斥，任一知识库 operation 在跑时拒绝新启。
- **附带修正**：`【聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。】` 这行红字在 Provider 管理面板里贴着上下容器，补内边距并与聚合初始化卡片的既有样式统一。

## Capabilities

### New Capabilities

- `logical-codebase-knowledge-base`: LC 知识库的建立、增量维护与阅读编辑面——provider 选择与调用、知识库状态投影、输入/变更包模板、变更包门禁、知识库文件读写授权、单飞与取消，以及 UI 三个按钮的可达性与失败面。

### Modified Capabilities

- 无。

## Impact

- **后端（新增）**：`src/product/logical_codebase/knowledge_base/`（operation / store / command / coordinator / templates / change_package / manifest / files）；`src/web/handlers/knowledge_base.rs`；`src/web/app.rs` 新增路由。
- **后端（既有文件改动）**：`web/state/repository_initialization_run_registry.rs`（新增 `InitializationOperationKind::KnowledgeBase`）、`src/product/logical_codebase/failure_copy_table.inc.rs`（新稳定码中文文案）、`src/web/error.rs`（新码 HTTP 状态映射）、`src/product/app_paths.rs`（知识库 operation 落盘目录）。
- **前端（新增）**：`web/src/components/lifecycle/KnowledgeBaseCard.tsx`、`web/src/pages/KnowledgeBasePage.tsx`、`web/src/components/shared/MonacoEditor.tsx`、`web/src/components/shared/MarkdownContent.tsx`、`web/src/api/knowledge-base.ts`、`web/src/api/types/knowledge-base.ts`。
- **前端（既有文件改动）**：`web/src/components/lifecycle/LogicalCodebaseManagementPanel.tsx`（挂载卡片）、`web/src/router.tsx`（新路由）、`web/src/components/lifecycle/ProviderManagementPanel.tsx`（红字内边距）、`web/src/components/lifecycle/AggregateInitializationCard.tsx`（provider 摘要组件公共化）、`web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`（MarkdownContent 提取，原处保留 re-export）。
- **外部依赖**：无新增前端依赖（复用 `monaco-editor` 与 `marked`）；依赖 Cadence-skills 插件的 7 个 `knowledge-base-*` skill 已安装。
- **不改动**：聚合初始化的五步配方、四条 root recipe 命令、`root_recipe_command_index()`、`BootstrapPhaseCredential` 与其重核验链、任何既有 provider 选择语义。
