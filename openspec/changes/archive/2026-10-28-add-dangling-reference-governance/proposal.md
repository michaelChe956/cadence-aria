# Proposal: add-dangling-reference-governance

> **r2 修订说明（2026-10-28）**：本 proposal 经两位独立审查者裁定「需修」后修订。修订**不改变** owner 已拍板的三层结构与任何硬边界；改动集中在**证据契约收紧**、**不可实现口径换成可实现口径**、**未核实事实换成实测结论**。逐条处置表见技术方案 §12，REQ 清单见 `specs/dangling-reference-governance/spec.md`（REQ-DRG-01–24）。
>
> **r5 修订说明（2026-10-28，owner 拍板后）**：本轮把**人工触发改为自动执行**、把①层容错**接到产品读路径**。要点：销账改为**服务启动时自动跑一次**（**删除人工确认令牌环节**与人工 CLI 入口）；清单摘要**仅用于审计关联**、不再是删除凭据（计算口径保留）；**新增 REQ-DRG-24**（①层容错接入产品读路径）；REQ-DRG-11 补「清单由启动时的巡检自动产出」。**未放宽**任何删除边界。
>
> **r3 修订说明（2026-10-28，闭环复审后）**：本轮只做**文本级互斥消解**，未改任何 owner 边界。要点：L2 口径统一为「该类型专属可算条件」；「不可判定」拆为「读取失败」与「证据不足 / 删除未完成」两类；REQ-DRG-19 的「无 L1 不删」加 L2 限定；intent 销账条款引用 REQ-DRG-20 四前置且四项**均在锁内重读**；幂等不得免除锁语义；留痕落点去「既有目录族」表述，改为状态根下**单一全局 JSONL 台账**；attempt 存在性路二改为**枚举**（禁用带写回副作用的单条会话读取）；窗口删除粒度改为**按 attempt 聚合**；令牌写死「只报不删残留条目进摘要」。

## Why

删除 story / design / work item group 之后，系统里留下三类指向「已不存在对象」的旁支/记账类记录——自动化 intent（`automation-plan-intent.json` / `automation-generation-intent.json`）、评审窗口与快照（`reviewer-windows/`、`reviewer-snapshots/`，**全局目录**，issue 级级联够不着）、实体身份台账条目（`state/entity-identity/`，同样在 issue 目录之外）。owner 逐条核对 186 条持久化记录后确认：真实悬空 37 条（intent 8、窗口/快照 12、台账 17），且它们已经从「偶发不适」升级为**真实阻塞**——owner 删掉 work item group 后重建流程，点「启用自动化」必然报 `story spec story_spec_0005 not found for automation enrollment`，重新开启自动化的路径被永久堵死。

**r2 实测校准（2026-10-28，只读 live 状态）**：台账条目实测 **16** 条（owner 口径 17），其中**仅 5 条**有删除墓碑；12 条窗口/快照记录的引用位**全部**是 coding attempt 形态（**无一条** session 形态）；2 个 intent 文件的 plan/session 引用是**预派生 id**（plan 尚未创建时已写入文件）。这些实测把「悬空」的**证据门槛**从「目标不存在」抬高到「有删除墓碑」——直接决定了本 change 的销账规则比 r1 严格得多。

根因不是「删除做得不够」，而是三件事从未被设计过：① 级联删除只清「子级产物」，从设计上就没有 intent / reviewer 资源 / 台账的席位；② 自动化 enrollment 被「终止」（逻辑失效）而非删除，文件仍在 → 其 `source` 字段继续指向已删实体；③ 「读到一条旁支记录」与「主记录缺失」在代码里没有区分，导致一条旁支悬空就能把整条用户流程带崩。

本 change 把这件事从「随机爆炸」变成**可枚举、可容错、可按类型销账**的三层治理能力：读侧只对旁支引用容错（跳过 + 告警），全量巡检先只报不改，随后按类型自动销账（幂等 + 留痕 + 清理后自证归零）。**r5：销账改在服务启动时自动跑一次**（「我重启下服务不就行了」），人工确认令牌环节与人工 CLI 入口一并删除；清单仍然逐次落盘供事后核对。

## What Changes

### 第 ① 层：读侧容错（先做）

- 新增**统一的「目标已删」判定能力**：按**字段名**（而非 ID 前缀）判定一条记录里某个引用指向哪类实体，再核对目标记录是否仍存在。目标类型覆盖 story_spec / design_spec / plan / session / attempt / checkout / aggregate_index / logical_member / enrollment 九类。
- **attempt 的存在性核对必须两路**（coding attempt / 规划面 workspace session）：规划面窗口把 workspace session id 写在 attempt 位上，单路核对会**把活的规划评审窗口误判为悬空并物理删除**（REQ-DRG-21）。
- 新增**四个「容错列举」只读 API**（自动化 intent 清单、reviewer 窗口清单、reviewer 快照清单、实体身份台账清单）：读到「指向已删对象」的条目时**跳过该条目 + 记一条告警**，**不得让整次列举失败**。intent 清单**只枚举两个 intent 文件名**，不把 enrollment 记录纳入悬空判定（REQ-DRG-03）。
- **严守边界**（本能力最核心的约束）：
  - **只对旁支/记账类引用容错**。主记录的缺失语义**一律不变**——`story_spec_not_found` 等既有错误码与文案逐字不动，既有测试不得变。
  - **只吞「目标已删」这一种错**。JSON 解析失败 / schema 不符 / IO 失败 / 权限错误**照旧上抛**，不得被误吞。
  - **既有 fail-closed 读路径一律不接入**：窗口租约列举、reviewer 资源物理清理、scope 准入与墓碑复核、intent 的版本化单条读——这些路径跳过一条会变成 fail-open（漏清 / 漏拦），比留残留更危险。
- **不注入既有读路径内部**：新增容错版列举是**新 API**，既有读路径零改动（受行数红线约束，见 Impact）。**r5 例外**：①层容错**接到产品里真的读这些旁支记录并顺着 id 去查目标的路径**（REQ-DRG-24）——**实体身份台账按 owner 反查**；只跳过「**已确认目标不存在**」，读失败照旧上抛，主记录缺失语义与 fail-closed 读路径不变。**reviewer 窗口清单不接入**（经核实：启动补偿扫描是**收口面**，孤儿窗口必须被释放，否则租约永不释放、③ 层也永不删它；既有 F12 用例即以「attempt 记录不存在的孤儿窗口」为夹具并断言它必须被补偿）。

### 第 ② 层：巡检（先 dry-run，只报不改；**r5：服务启动时自动产出清单**）

- 巡检范围覆盖 `.aria` 下全部持久化记录：issue 级（`projects/<p>/issues/<i>/**`）+ 三类全局分区（`reviewer-windows/**`、`reviewer-snapshots/**`、`state/entity-identity/**`）+ LC scope 目录。
- 判据为**记录类型 → 引用字段 → 目标类型**的静态表，再核对目标记录是否仍存在；表外的记录类型一律归「未归类」。判据表**补登记** intent 的**五个授权引用字段**（`enrollment_id` / `plan_id` / `session_id` / `source.stories[].id` / `source.designs[].id`）与 enrollment 的**全部引用字段**（顶层 `plan_id` / `session_id` / `source.*`、当前绑定代际 `binding_history.current.*`、历史代 `binding_history.previous[].*` 的 source）（REQ-DRG-09）。
- 输出一份清单（记录路径 + 悬空引用类型与目标 id + 建议动作 + 「已删」证据），落 **`cadence/analysis-docs/`**（按 `rule://document-storage` 的「分析报告」分类）。
- **dry-run 阶段只有读路径，不存在任何删除分支**：巡检前后状态树逐文件校验和必须完全一致。
- **读失败进「不可判定」计数**，不进悬空计数、不进归零断言；不为 0 时**不得宣称治理完成**（REQ-DRG-23）。巡检扫描到的记录自身解析失败、分区目录不可读同样进不可判定。「读取失败按未悬空」这条口径**只约束 enrollment 重新授权判定**，不得外推到巡检（REQ-DRG-18）。
- **attempt 存在性路二必须用「枚举」**：窗口 / 快照记录不带 `project_id` / `issue_id`，故巡检遍历**全部 project × issue** 列举 workspace session 并在一次巡检内建 session id 集合；MUST NOT 使用会在读路径上写回补写的单条会话读取（会破坏零副作用契约）；枚举失败一律归**不可判定**，MUST NOT 当成「目标不存在」（REQ-DRG-21）。

### 第 ③ 层：按类型自动销账（**r5：服务启动时自动执行**）

- 按类型定规则，不搞一刀切。**销账证据门槛**（REQ-DRG-02）：
  - **L1 直证** = 删除墓碑存在 **且** 删除阶段为已完成态 **且** 带完成时间 **且** 无待人工处置的删除失败事实——四条**同时**成立；删除进行中（`purging`）、缺完成时间或已失败的墓碑**不算** L1，归「**证据不足 / 删除未完成**」并只报不删（**不**归「不可判定」——「不可判定」只收**读取失败**或**所需存在性数据源 / 作用域不可用**）。
  - **L2 推定** = 目标不存在 **且** **该类型在证据等级表中给出的专属可算条件**成立；**只对证据等级表标为 L2 的目标类型可用**（当前仅 reviewer 窗口簇），其余一律只报不删；**上游无法定义的类型降级只报不删**。
- 具体动作：自动化 intent（story/design/plan **有 L1 墓碑**且满足四道前置条件）删；评审窗口与快照（两路目标皆无 **且** 窗口四条件成立）删——删除粒度**按 attempt 聚合**：同一 attempt 下的**全部**窗口（含归档）都满足才删，任一不满足则一个都不删；实体身份台账条目（plan **有 L1 墓碑**）删；**实体删除墓碑永不删**；**未归类与降级类型一律只报不删**。
- **intent 销账的四道前置条件**（REQ-DRG-20）：在 enrollment 文件锁内删除 + **锁内重读三项**（enrollment 状态、generation intent 相位、L1 墓碑证据；「已终止」以 `terminated_at` 有值为准，仍启用一律不删）+ 只认 L1 墓碑（session/授权引用缺失不构成依据）+ generation intent 相位必须为「已认领且未派发」（防双跑检查点不可丢）。**幂等不得免除锁语义**。
- **三道闸**：① **r5 自动模式**——销账在**服务启动时自动跑一次**，**无人工确认令牌环节**（删除判据 = 白名单类型 + 证据达标，MUST NOT 比上表更宽）；② 按类型定规则，白名单外绝不删；③ 每一步幂等 + 留痕（何时 / 因哪个实体被删 / 清了哪条 / 依据的证据等级），清理后**再巡检一次自证归零**——归零**只统计销账白名单内且本次可删的类型**，「只报不删残留」单列一栏且不参与归零，未归零必须逐条列出、不得宣称治理完成。
- **清单摘要仅用于审计关联**（REQ-DRG-22）：计算口径不变（findings 规范化序列化摘要），但**不再是删除凭据**；清单上仍展示该摘要，便于人核对两次巡检是否同一现场。
- 留痕落在**状态根下单一全局的追加式 JSONL 台账**（`{state_root}/entity-deletions/dangling-reference/cleanup.jsonl`，**目录由本 change 新建**），行形态沿用既有根级 JSONL 审计规范；MUST NOT 写入 issue 级删除墓碑目录，MUST NOT 扩展 `DeletionEntityKind`（既有删除记录机制是每 issue 级逐实体墓碑，无法承载一对多的销账留痕——技术方案 §3.5.1 已如实说明；落点归属留 owner 拍板，见 U10）。
- 销账**不删**自动化 enrollment 记录（其重新授权语义由在途工作裁定），**不删**任何用户业务数据（story / design / plan 正文、versions、artifact）。

### 与在途工作的关系

`FixAutomationEnrollmentDanglingSource` **已落地并合并**（commit `dd62e5db`）「自动化那条线」：前端重新开启路径改用当前 card 的 source、后端允许「旧 source 悬空 → 按当前已确认 source 重新授权并留痕」、级联删除把 source 引用该实体的 enrollment 一并**失效**。本 change **不重复设计**该语义，只把它作为**已落地的前提契约**承接：巡检与销账必须把「enrollment 的 source 指向已删 story/design」视为已知的、在途覆盖的状态，不判为未归类，也不对 enrollment 记录执行删除。两侧**共用同一套「目标已删」判定能力与同一个告警码**。

### 非目标

- 不改既有删除语义与既有级联清理覆盖面（`DeletionEntityKind`、`EntityDeletionRecord` 形状、墓碑路径段、删除准入与 fail-closed 判定一律不动）；**不把 intent / reviewer-windows / reviewer-snapshots / entity-identity 挂进级联删除闭包**——销账是独立的、闸门控制的动作。
- 不改主记录的缺失语义；不删墓碑；不删用户业务数据。
- **不删除任何不具备 L1 证据、且不满足证据等级表中该类型 L2 专属可算条件的旁支记录**（r2 收紧、r3 加限定）：无墓碑的台账条目、intent 的 session/授权引用、缺 plan/session 的 intent、非「已认领且未派发」相位的 generation intent、降级类型，一律只报不删。（窗口簇按 L2 删，与本条不冲突。）
- 不改动既有 enrollment 文件锁的持有方与顺序（销账只在同一临界区内借用它）。
- 不新增依赖；生产目录操作用 `std::fs`。
- 不动 provider 适配层 / `ProviderToolPolicy` / gateway envelope / `StreamingProviderInput`。
- 不碰行数红线文件；不新增常驻清理任务；不 bump `PRODUCT_DATA_SCHEMA_VERSION`。
- 不设计 enrollment 的重新授权语义（归在途工作）。

## Capabilities

### New Capabilities

- `dangling-reference-governance`: 悬空引用（指向已删对象的旁支/记账类引用）的三层治理能力——读侧只对旁支引用容错（跳过 + 告警，主记录缺失语义与真实错误不变）、全量巡检只报不改（读失败隔离为「不可判定」）、按类型幂等销账（证据分级 + 留痕 + 清理后自证归零）。

### Modified Capabilities

无。既有 capability 中与本能力相邻的 `coding-attempt-deletion` / `coding-attempt-deletion-cleanup` / `work-item-group-deletion` 规范的是**删除既有实体**的边界与覆盖面；本能力规范的是**已删实体留下的旁支残留**的治理，不改写它们的任何 requirement。

## Impact

- **受影响 spec**：仅新增 `dangling-reference-governance`；不修改既有 spec，不删改既有 change。
- **新增代码面**（全部落在新目录 `src/product/dangling_reference/`，天然不触行数红线）：判定能力与判据表（含证据等级表与 attempt 两路核对）、四个容错列举 API、巡检遍历器与清单渲染（含不可判定隔离）、按类型销账（intent 四前置条件 + 窗口四条件）与留痕台账。既有文件的改动**仅限三处极小胶水**：`src/product/mod.rs`（+1 行模块声明）、`src/product/app_paths.rs`（+约 8 行台账路径）、`src/cli.rs`（+约 15 行巡检/清理子命令）。
- **既有文件改动**（r5）：`src/cli.rs`（**删除**巡检/销账子命令）、`src/web/app.rs`（启动期自动巡检 + 自动销账）、`src/product/coding_workspace_engine/code_review.rs` 与 `src/product/lifecycle_store/plan.rs`（**r5 接入①层容错**，且两者在门禁登记表内零余量 → 改动 MUST 为零行数增长）。其余既有文件零改动（尤其：`src/web/handlers/automation_enrollment.rs`、`src/product/issue_automation_store.rs`、`src/product/reviewer_window_store.rs`、`src/product/reviewer_snapshot.rs`、`src/product/entity_deletion_cascade.rs`）。
- **下游零改动**：provider 适配层（codex/claude/pi/kimi）、gateway 与 envelope、`ProviderToolPolicy`、`SessionResumeFingerprint`、`StreamingProviderInput`、daemon canonical writeback、既有 prompt 预算与内存态 engine 测试。
- **数据面**：新增状态根下的一个分区（留痕台账，**目录由本 change 新建**），不改既有数据格式，不 bump `PRODUCT_DATA_SCHEMA_VERSION`。
- **验收**：三层各自定向用例 + 控制器统一回归；dry-run 的「状态树逐文件校验和不变」、销账后的「按类型归零（只统计可删类型）」与「findings 规范化序列化逐字节一致」是最强断言；真机核对需要 owner 另行授权 provider 调用，不以 fake provider 冒充。
- **残余风险明示保留**：① 层当前没有面向用户的既有消费面，其「不阻塞用户流程」是**预防性**收益而非当下止血（真正的止血来自在途的自动化线修复）；**r5 已把①层接到产品读路径**（reviewer 窗口清单 + 台账按 owner 反查），但 `reviewer_window_store.rs` 1921/1921 仍零余量、其既有 fail-closed 语义保持不变；无独立墓碑的实体其「已删」只能推定，**上游无法定义者一律降级只报不删**；**规划面窗口的现场行为本现场无样本**（12/12 为 coding attempt 形态），两路核对是防御性设计；**generation intent 的防双跑检查点不可丢** → 相位非「已认领且未派发」时永远不会被本能力清理（属「只报不删残留」）；该工作树的行数门禁在 HEAD 基线即已报红（本 change 不加剧，归属另行治理）。
- **需 owner 拍板**：销账留痕台账的落点归属（新建 `state_root/entity-deletions/` 子树 vs 改归既有根级审计分区范式）——见技术方案 §11 U10。
- 技术方案文档：`cadence/designs/2026-10-27_技术方案_悬空引用治理与读侧容错_v1.0.md`（含 §2.4 实测证据与 §12 逐条处置表）。
