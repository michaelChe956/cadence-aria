# Design

## Context

见 `proposal.md`（Why）与 `../2026-11-10_技术方案_知识库输入草稿与页面改版_v1.0.md`（完整技术方案与出处）。这里只记录**架构决策与权衡**。

本 change 是在已归档的知识库能力（`2026-11-10-add-logical-codebase-knowledge-base`）之上追加三块：全域前置门、两段式输入草稿 + 用户确认、详情页改版。

## Decisions

### D1 前置门统一到冷启动投影的「规则/政策」步
**决定**：知识库的「聚合初始化已完成」一律判定为 `LogicalCodebaseBootstrapProjector::project(project_id, lc_id)` 的 `rules_policy` 步 `status == Completed`；投影读不出 / IO 故障 ⇒ 判未完成（fail-closed）。既有 `preflight.rs::aggregate_initialization_completed`（根规则文件 + 政策 artifact）**被替换**，不并存两套口径。

**理由**：owner 明确选定「与平台既有冷启动口径一致」。冷启动投影的 `rules_policy` 步已经综合了政策 artifact、recipe operation 状态与材料漂移，比「两个文件存在」更接近「聚合初始化真的完成了」；而且投影是**平台已经在用**的事实，UI 抽屉里显示的「规则/政策 已完成 / 未开始」与知识库的门禁用同一口径，不会出现「抽屉说完成、知识库说没完成」。

**代价**：投影计算比两个文件存在检查重（要解析 authority resolution）。缓解：投影是纯只读、无 provider 启动；且门在 HTTP 层一次判定，页面级可复用同一投影。

### D2 前置门放在 HTTP 层一处，覆盖全部端点
**决定**：门在 KB handler 的公共入口做一次（而非每个端点各自判），失败统一映射到 `knowledge_base_requires_aggregate_initialization` + 中文可执行文案；`preflight_bootstrap` / `preflight_update` 内部改为复用同一函数。

**理由**：端点已 12+ 个且会继续增（本次就加了输入清单、输入确认、起草），逐端点各写一遍必然漂移。集中在一处 ⇒ 新端点自动受约束。

**替代方案**：在 `knowledge_base` 模块里做一个「guarded handler」包装器 —— 与 D2 等价，取舍在实现层（路由注册处统一包一层 vs handler 内部统一入口检查），以改动面最小者为准。

### D3 输入「未准备」判定 = 与插件模板逐字比对
**决定**：`user-input/` 下的 9 份输入**全部缺失**或**全部与插件模板 `cadence-init/skills/knowledge-base-bootstrap/user-input/` 逐字相同** ⇒ 判未准备（需要起草）；**任一份**与模板不同 ⇒ 判已准备（不再起草）。

**理由**：`knowledge-base-user-input` skill 自己的门禁 1 就是这个判据（基线字段仍为空、状态仍是模板默认占位、示例行与填写提示仍在 ⇒ 未填写的模板占位，允许覆盖）。平台用同一判据 ⇒ 平台与 skill 不会各判各的，出现「平台说需要起草、skill 说已填写」的错位。

**替代方案**：解析模板占位标记（更语义化，但模板文案会变 ⇒ 判定跟着变脆）。逐字比对更稳：模板是插件资产，平台直接读插件源。

### D4 起草 operation 复用知识库 operation 机制，新增 kind
**决定**：新增 `KnowledgeBaseOperationKind::UserInput` 与 `KnowledgeBaseStepKind::PrepareUserInput`，provider turn 的 cwd = canonical 聚合根，命令 = `skill_command_prefix` + `knowledge-base-user-input`（复用既有前缀唯一来源），写入面审计允许面 = `cadence/knowledge-base/user-input/**`。

**理由**：起草与 bootstrap 在「选 provider → 起 turn → 轮询 → 取消 → 单飞 → 落 durable 事实」上完全同构，复用既有 operation 机制比另起一套省一整套代码；审计允许面与 skill 的「只写这个目录」契约逐字对齐，越界即失败。

**代价**：知识库 operation 的 kind 从 2 变 3，既有序列化字段需兼容（旧记录反序列化不得失败）⇒ 新枚举值用 `#[serde(other)]` 或默认值兜底（实现时确认既有 store 的反序列化策略）。

### D5 确认记录落平台侧 .aria，不写 user-input
**决定**：确认记录 per-LC 持久化在平台侧（如 `knowledge-base/user-input-confirmations.json`），记录 `path → { digest, confirmed_at }`；`user-input/` 目录内**不新增任何文件**。

**理由**：skill 的生成后自检第 1 条明确要求「`user-input/` 内无其它新增文件」，平台若往里写标记文件会破坏该不变量，也会让 skill 的「已填写」判定看到外来文件。

**确认失效语义**：查询时逐份比对记录摘要与文件当前摘要 ⇒ 不一致即视为未确认（编辑后自动失效，无需清理记录，也无需后台任务）。

**过期确认**：确认请求携带客户端读到的摘要，服务端重算当前摘要比对 ⇒ 不一致即拒（`knowledge_base_user_input_confirmation_stale`）。避免「客户端拿着旧内容确认了已经变过的文件」。

### D6 两段式由后端按状态决定，前端按钮不分段
**决定**：【知识库初始化】是一个意图，后端按当前状态决定跑哪一段：输入未准备 ⇒ 起草；已准备未确认 ⇒ 拒绝并给未确认清单；已确认 ⇒ 跑 bootstrap。前端只呈现状态（未准备 / 待确认 N 份 / 可执行）。

**理由**：状态判定若放前端，两端会各判一次且可能不一致（用户刷新、并发、直连 API 都会打脸）。后端判定 = 单一事实源；前端只消费状态投影。

**替代方案**：前端显式发 `kind=user_input|bootstrap`（用户明确点「生成草稿」时才发前者）——详情页的「重新生成草稿」入口用它，但主按钮走后端自动判定。

### D7 详情页做成沉浸式工作区（路由级 opt-out）
**决定**：CockpitShell 支持**路由级沉浸模式**：当前路径命中知识库路由白名单时，只渲染 children，不渲染全局浮动件（设置触发器、图片创作气泡、版本更新说明/引导浮层）。知识库页自带顶栏（标题 + 项目/LC 关联信息 + 状态徽标 + 返回工作台按钮），左文件树固定宽可折叠、右编辑器占满剩余高度。

**理由**：owner 要求「去掉设置按钮 + 编辑窗口拉满 + 有返回主页」。若仍在应用外壳内，外壳的浮动件与侧栏会持续抢占注意力，沉浸模式是达成「全屏工作区」的最直接手段。

**代价**：CockpitShell 多一个路由分支 ⇒ 既有路由行为必须零变化（白名单只含知识库路由；既有测试全绿把关）。

**信息来源**：项目名 / LC 名从既有 LC 清单与 bootstrap 投影取（详情页已能拿到 `projectId`/`lcId`，投影接口已存在），不新增查询。

## Risks / Trade-offs

- **[投影口径比旧判定重]** → 只读、无 provider 启动；页面级复用同一投影避免重复计算。
- **[起草会覆盖未填写的模板占位]** → 平台在起草前明示「将覆盖 N 份未填写模板」，并把覆盖清单写入 operation 输出摘要（skill 自身也要求报告）。
- **[skill 在目标项目形态不满足时自停]**（非 Java 且非 Vue/React）→ 失败原文透传，失败面提示形态前提。
- **[确认记录与文件内容漂移]** → 服务端重算摘要 + 查询时比对，漂移即失效。
- **[沉浸模式影响外壳既有行为]** → 白名单单路由 + 既有测试全量把关。
- **[operation kind 增加枚举值]** → 旧记录反序列化必须仍成功（实现时确认 store 的反序列化兜底策略）。

## Migration Plan

- 纯增量：既有端点行为不变（新增门只在「聚合初始化未完成」时改变 —— 这是本次的有意行为变更）；既有 operation 记录可反序列化。
- 无数据迁移；确认记录首次使用即创建。

## Open Questions

- 真机上 `/knowledge-base-user-input` 在**多成员仓 LC** 下的起草效果（skill 以「目标项目根」为工作目录，聚合根下是多个成员仓）需要 owner 用真实 LC 验证一次；平台侧不改 skill 契约，只透传其产出与失败。
