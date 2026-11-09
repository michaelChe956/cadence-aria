# Design: isolate-plan-criterion-namespace

> 权威来源：owner 2026-11-08 决策报告 §5（决策五：WIC 双前缀只读兼容 + 存量保身份 + 新生强制新域 + 指纹按会话固定代际）。
> 契约文本见 `specs/`：plan 侧 `REQ-WSC-30` / `REQ-WSC-31`，策略侧 MODIFIED `REQ-TOP-04` / `REQ-TOP-05`。
> 本文件只持久化**已确认**的架构边界、职责、权衡、失败处理与迁移策略；行号均以
> `.worktrees/feat-wig-optimize-11-08/` 为根。

## Context

当前 plan 域的验收标准 ID 与 spec 域的验收标准 ID 共用同一字面空间，冲突点有三处：

- 编译侧：`src/product/work_item_plan_compiler/parse.rs:441-455` 的 criterion 分支只接受单一前缀 `AC-`；`validate_identifier:460-485` 以「前缀 + 非空 ASCII 数字」判定（`is_valid_identifier:488-491`），`task_id` / `check_id` 走各自单前缀分支。
- 引用侧：`src/product/work_item_contract/validation.rs:245-312` 用**字符串相等**把 `done_when_refs`、`reviewer_check_refs` 约束到**本 Work Item 的 plan 域 criterion 集合**，`requirement_refs` 约束到本 WI 的 design_traceability 集合；而 `src/product/work_item_split_engine/context.rs:125-138` 从 story + design 正文抽取 `REQ-` / `NFR-` / `AC-` 组成 spec 域 `[design_requirements]` 清单。两域同串时（例如 spec 有 `AC-001`、plan 也有 `AC-001`），「引用的是哪一个」在数据层不可区分。`src/product/work_item_projection/reviewer.rs:19` 的 `requirement_matrix` 正是靠 `done_when_refs` 与 criterion 的字符串相等反推 requirement 归属，歧义在此被放大。
- 指纹侧：`src/product/work_item_plan_policy/fingerprint.rs:8` 的 `STABLE_ID_PREFIXES: [&str; 3] = ["WI-", "CT-", "AC-"]` 被 `stable_ids:143-156`（抽取）与 `remove_stable_ids:174`（剥离）共用，但两处都**没有 token 边界**：`WAC-001` 会被抽出子串 `AC-001`。`RunHistory`（`src/product/work_item_plan_policy/types.rs:188-214`，`serde(default, deny_unknown_fields)`）保存 `seen_fingerprints` 与按 cycle 的 `original_fingerprints`；scope 摘要配方在 `types.rs:405-434`（`scope_digest_for_initial` / `scope_digest_for_verification`，前缀 `review_scope_v1:`）。身份配方另带盐 `v2-stable` / `v2-unstable`（`fingerprint.rs:4-5`）。

因此「引入新命名域」一旦只落在字符串层，会同时踩到两个身份断裂点：实体 ID 被重编号、指纹被重算。

## Goals / Non-Goals

**Goals**

- 读：新命名域 `WIC-` 与既有 `AC-` 在 plan criterion 上双前缀只读兼容，旧计划仍可编译与派生修订。
- 生：新生 SC 计划强制 `WIC-`；存量 lineage 保既有 ID。
- 身份：lineage 级命名策略由服务端持久化并冻结，不由客户端声明，不由模型措辞推导。
- 算法：指纹与 scope 摘要按**会话**固定算法代际；旧代际算法逐字保留为冻结历史的兼容读路径。
- 不改历史：旧 source / IR / 机械报告 / 引用 / 指纹 / 审批记录与其内容寻址全部保持原样。

**Non-Goals**

- 不做存量实体迁移（WIC→AC 反写、批量改前缀、全历史指纹重算、旧算法退役）。决策 §9 已把它们划入后续数据迁移项目。
- 不 bump `compiler_version`（`src/product/work_item_plan_compiler/grammar.rs:148` 的 `work_item_plan_compiler/v1`），不借命名域改造夹带 IR 语义版本升级。
- 不改 legacy JSON outline / draft / batch 流程的命名域。
- 不扩引用范围：成员校验、spec 编号抽取与 reviewer 投影的既有语义保持不变。

## Decisions

### D1 · criterion 双前缀只读兼容，词法不放宽

只改 criterion 分支的前缀集合为 `["WIC-", "AC-"]`；`task_id` / `check_id` / work item ID 仍单前缀。校验顺序固定为「先确认任一前缀合法 → 再做一次 seen 插入去重」，**不循环调用两次 validator**，避免同一标识符被重复计为两次冲突。`is_valid_identifier` 的「前缀 + 非空 ASCII 数字」判定逐字保留。

*备选与否决*：放宽为「任意 `XXX-数字`」——否决，会把 `WAC-` 之类混合前缀纳入合法集合，把命名隔离变成前缀黑名单；接受大小写变体——否决，同一实体出现两种字面即引入新的身份分叉。

### D2 · spec 域与 plan 域职责分界写进教学与守卫，不改引用面

spec 域 `REQ-*` / `NFR-*` / `AC-*` 只服务 Traceability 与 `requirement_refs`；plan 域 ID 只服务本 WI 的 criterion、`done_when_refs`、`reviewer_check_refs`。分界由两处承担：SC author 教学与人工修订教学的材料措辞，以及新候选编译 / 提升边界上的命名域守卫。

*为什么不改引用面*：`validation.rs` 的成员校验是**闭集比较**（引用必须落在本 WI 的 criterion 集合内），它本身已经正确地把「别的 WI 的 `WIC-`」拒掉；真正的缺陷是**输入侧**把两个域的同串 ID 混在一个池子里讲。改校验逻辑去猜「这个 `AC-001` 来自哪个域」既不可判定，也会把 spec 域引用面扩大——决策明确「不因迁名扩引用范围」。

**`forged_done_when` 删除（归本 change）**：handler `run/single_candidate.rs:404-437` **保留**真实 spec 域 `forged` 检查（`requirement_refs` 不在 spec 注册集仍拒），**删除**把 plan 域 `done_when` 引用拿去 spec 注册集判伪造的 `forged_done_when` 及其混合错误文案——本 WI 引用合法性由 canonical validator（`validation.rs` 的成员闭集校验）负责。净效果：`LegacyPreserved` lineage 交付中的 plan 域 `AC-001` 不在 spec 注册集时**不再被判伪造**；真实 spec 域伪造仍拒。配套教学面：`single_candidate.rs:458-487` 与 `workspace_engine/prompts.rs:244-249` 两处教学均分清 spec 与 plan 域，禁止把 done_when 教成全局 spec 清单成员；`work_item_split_engine/prompts.rs:185,306` 的 ID 纪律加入 WIC（`:306` 的 spec Traceability `AC-` 示例**不替换**，保持 spec 域示例语义）。

**守卫诊断与教学重驱资格**：命名域守卫诊断是**新增独立 typed fault**——`AC-` 在双前缀下语法合法，不触发 `invalid_work_item_id`，也不因诊断码前缀或「缺少」措辞自动获得教学重驱资格。该资格由 `mechanicalize-plan-compile-repair` 的 MODIFIED REQ-WSC-09（typed 资格清单，全局序 1）承载；本 change 只提供 typed 诊断并在教学面**引用**该清单，**不**自行扩展 WSC-09。另需登记：`work_item_plan_compiler/grammar.rs:140` 的 `DIAGNOSTIC_CODES` 为定长 4 数组，新增该 typed fault 码需扩容此数组（本 change 只登记事实，不在规划产物中预写代码）。

### D3 · 命名策略是服务端 lineage 元数据，在真实 SC 首次生成准入以 CAS 冻结

新增 server-only 命名策略（`WicV1` / `LegacyPreserved`），以**可缺省**字段挂在 plan lineage 实体记录上（`src/product/models/lifecycle.rs` 的 `IssueWorkItemPlan` 一侧；新类型可独立成 `plan_criterion_namespace` 模块）。冻结点：真实 SC 首次生成准入，与 `src/product/lifecycle_store/plan.rs` 的创建 / ensure 路径共同保持该字段。

判定规则（三态，缺证据即不判新生）：

| 事实 | 判定 |
|---|---|
| 无历史 source / IR / 评审 / 发布事实，且存在真实新生事实 | `WicV1` |
| 存在任一历史事实 | `LegacyPreserved` |
| 既无历史事实也无新生事实（未知） | **不冻结命名策略**；不得判为 `WicV1`、不伪装新生；按未知处理（待既有等待 / 人工面）并披露未知 |

*为什么是 lineage 实体而不是 session 实体*：命名域属于「这份计划的身份」，不属于「这次运行」。挂在 session 上会在接管 / 新建 session 时丢失 lineage 的域信息；挂在 plan 实体上则重入、接管、重复准入都读到同一份事实（已冻结即不变）。session 侧只在准入时读它。

*备选与否决*：由客户端在请求里声明「legacy=true / new=true」——否决，客户端声明不能作为身份事实；用 compiler 版本猜命名域——否决，编译器版本表达的是 IR 语义，不是命名域；读取时按内容推断——否决，存量计划里同时存在旧 AC 与新 WIC 时推断结果不可证。

**CAS 载体（钉死）**：`IssueWorkItemPlan`（`models/lifecycle.rs:299`）现无 revision 字段，仅 `lifecycle_store/plan.rs` 的 plan 记录带 `membership_revision`。命名策略元数据落在 **plan 记录**（`IssueWorkItemPlan` 的可缺省字段），更新走 **plan 文件锁内 RMW**；CAS 载体＝plan 的 `membership_revision`（若其语义不足以承载命名策略写入，则在 plan 记录**新增独立 revision 字段**），**不得**依赖 session record 的 CAS。创建 / ensure / SC 准入（`lifecycle_store/plan.rs:36-125,132-187`）共同保持该元数据；**重入 / 接管不覆盖已有策略**——「已冻结不可改」只约束已判定的 `WicV1` / `LegacyPreserved`，未知态不冻结、可待事实补全后再判定。

**措辞澄清（同名异物）**：本文与 spec 中的「lineage」指**命名策略代际**（`WicV1` / `LegacyPreserved`），与 `work-item-revisions/{plan}/lineage.json` 所记录的 revision lineage **不是同一物**；后者是修订历史链，不在本 change 语义内，全文统一按此措辞。

**守卫的三处接入位置与相对次序**：① handler 预检（决策六拆分后的 `single_candidate_prevalidation.rs`）——与权威判定用**同一规则**提前诊断，不吞 operational 错误、不授教学；② C 硬检——置于新交付 compile 成功后、预检 / refs 提升前；③ 权威 completion 复验——completion 路径以同一规则复验。三处共用同一守卫实现（`workspace_engine/plan_criterion_namespace.rs`），次序固定 ①→②→③，后一处不因前一处已拒而放松。

### D4 · 存量保 ID、新生强制 WIC；禁止「仅前缀升级」

`LegacyPreserved` lineage 的修订以该 lineage 的**基线不可变 source** 中已声明的 criterion ID 集合为基线：**未经修复闭包与业务授权**，MUST NOT 删除、重编号或替换旧 baseline 的 criterion ID；经授权的删除走修复闭包既有合同。「仅前缀升级」替换整组身份在任何情况下 MUST NOT 发生。集合外新增的 criterion 必须使用 `WIC-`。若同编号的 criterion 在基线为 `AC-00x` 而修订写成 `WIC-00x`，视为「仅前缀升级」，拒收。

*为什么不提供批量迁移*：迁移需要同步 source / IR / report / refs / review scope / fingerprint / approval，而不可变记录不能原地改；兼容读取的成本远低于一次跨七类记录的数据迁移。

### D5 · 指纹代际隔离：v2 逐字保留 + v3 新算法（同代际两套配方）

- **v2（冻结历史的兼容读路径）**：`STABLE_ID_PREFIXES = ["WI-", "CT-", "AC-"]`、无 token 边界、盐 `v2-stable` / `v2-unstable`——身份配方与 scope 摘要配方**逐字保留**，只用于读旧历史与旧 scope。
- **v3（新代际）**：稳定 ID 前缀集合为 `["WIC-", "WI-", "CT-", "AC-"]`；抽取与剥离**共用同一个 token 扫描逻辑**（不是两份实现）；匹配**最长前缀先**；token 前后不得紧连字母 / 数字 / 下划线。
- 同一字符串的「取」与「删」结果必须完全一致（现状两处逻辑虽同源但分别维护，v3 收为一份）。

**为什么必须 token 边界**：`WAC-001` 被抽出 `AC-001` 会让「一个实体的两种误写」落进同一身份集合；边界规则把 `WAC-001` 判为「不含稳定 ID」，落入 unstable 身份路径，按既有 fail-safe 进人工处理，而不是被当成明确 ID 参与判重。

**纠正一处常见表述**：现状抽取层确实会把 `WAC-001` 的子串抽成 `AC-001`，但尾段文本同样参与 hash 输入，因此**不得断言「`WAC` 必然与某 `AC` 完整 hash 碰撞」**。可以断言的是「存在误抽取风险」；不能断言的是「完整 hash 必然相等」。正确表述：v3 的边界规则消除**误抽取**，v2 只服务冻结历史兼容。

**v2 抽不到 WIC＝强制人工，这才是 v3 的直接理由**：v2 抽取域（`WI-` / `CT-` / `AC-`）对 `WIC-001` 抽不到任何稳定 ID（`WI-` / `AC-` 皆非其子串），新域实体在 v2 下全部落入 unstable 身份域、被 fail-safe 强制人工——**这才是需要 v3 的直接理由**（不只是 `WAC-` 误抽取这一面）。

### D6 · 代际随会话固定，缺省 v2，scope 绑定版本

- `RunHistory` 增**可缺省**的指纹代际字段：缺省即 v2（与既有历史等价），序列化旧 v2 记录时该字段可省略；新建受控会话**显式**写入 v3。**「受控会话」＝以 `flow_kind=SingleCandidate` 创建路径建立的会话；legacy（非 SC）会话保持缺省 v2，不写新代际。**会话创建路径当前以 `run_history: Default::default()` 落历史（`src/product/lifecycle_store/workspace.rs:325`），改为在创建时写入显式代际。
- 代际字段的键名与取值域**钉死**：键名 `fingerprint_version`，取值域 `{"v2","v3"}`；未知取值按**状态损坏**拒收。`RunHistory` 带 `deny_unknown_fields`，该字段需**自定义反序列化**（缺字段 → v2；未知取值 → 拒收），**不得**用 serde default 把未知取值回落到任一已知代际。
- `evaluate.rs:184` 以**全字段字面量**重建 `RunHistory`：新增代际字段 MUST 在该处**逐字段拷贝**，MUST NOT 用 `..Default::default()`（否则新代际静默回落 v2）。
- 接管路径已完整复制父 `run_history`（`workspace.rs:485`，同一处把 `provider_start_ledger` 清空），因此接管天然沿用父代际；resume / rearm / 新候选**不得**重置代际或预算。
- review scope 增**可缺省**版本：v2 scope 的摘要配方逐字不变；v3 scope 的摘要输入纳入代际标识并绑定 original 集合，**摘要前缀保留既有 `review_scope_v1:` 不变**（前缀属既有场景写死项，不动）；Verification scope 保持既有 `original_fingerprints` 逐项绑定不放松。scope 与 session 代际不一致 → 走既有结构性协议错误 `Fatal(ProtocolViolation)`，**不重算摘要假装同一 invocation，不降级为人工门**。
- 所有消费面（finding identity、original / seen 集合比较、scope 构造与校验）显式从当前 `RunHistory` 取代际，**不允许任何 caller 走「最新默认」**。

*为什么不直接改算法而无版本*：只改边界规则即使不改任何 `AC-` 字面，也会改变历史字符串的解析结果，进而改变 hash；旧 lineage 的 seen / original 集合随即失去可比性。

### D7 · compiler version 不 bump

双前缀是**兼容接受新字面形式**：同一份旧 source 的 lower 结果与内容寻址不漂移（验收需显式断言）。命名纪律由独立命名策略承担、算法代际由独立版本承担，都不借编译器版本表达。

*失败即停*：若实施发现旧输入的 IR 发生非等价变化，则「不 bump」的前提不成立，必须停止上线并走显式 compiler 迁移，而不是强改历史 hash。

### D8 · 兼容矩阵（唯一权威表）

| 维度 | 旧世代（v2 / `LegacyPreserved`） | 新世代（v3 / `WicV1`） | 迁移与切换规则 |
|---|---|---|---|
| 指纹算法代际标识 | `RunHistory` 缺该字段 ⇒ 读作 v2 | 显式写入 v3 | 出生时写定；同一 session 内**不可切换** |
| 稳定 ID 前缀集合 | `["WI-", "CT-", "AC-"]` | `["WIC-", "WI-", "CT-", "AC-"]` | v2 配方逐字保留（只读旧历史）；v3 最长前缀先 |
| ID token 边界 | 无边界（`WAC-001` 可被抽出 `AC-001`） | 前后不得紧连字母 / 数字 / 下划线 | 仅 v3 生效；v2 历史**不重算** |
| 身份配方盐 | `v2-stable` / `v2-unstable` | v3 另立盐，语义分区不变 | v2 盐配方逐字保留 |
| scope 摘要配方 | `review_scope_v1:` 前缀 + 既有 canonical 编码 | `review_scope_v1:` 前缀**不变**，摘要输入纳入代际标识并绑定 original 集合；Verification 保持既有 `original_fingerprints` 逐项绑定 | v2 scope 摘要配方**不变**，旧 scope 仍可校验通过 |
| plan 域 criterion ID | `AC-` | `WIC-`（存量 lineage 的既有 `AC-` 一律保留） | 存量不重编号；新增必须 WIC；禁止仅前缀升级 |
| 命名策略元数据 | `LegacyPreserved` | `WicV1` | 真实 SC 首次生成准入时 CAS 冻结，已冻结不可改 |
| 实体 ID 迁移 | 不迁移 | 不提供 WIC→AC 反写 | 迁移 ID = **无迁移**；跨域同号不视为同一实体 |
| 原始 fixture 哈希锚点 | v2 身份配方在既有 fixture 上产出的 finding identity 与 scope digest **逐位值** | 由 v3 产出的新值 | v2 golden 必须在**未改代码**上先固化，变更后逐位比对相同；v3 侧断言 `WIC-` 与 `AC-` 不同身份、`WAC-` 不被抽为 `AC-` |

> 关于「原始 fixture 哈希」：本 design **不预写具体哈希数值**——数值必须由未改代码的实际产出固化，属实施阶段的 golden 锚点；先固化再改算法是唯一能证明「v2 逐位不变」的做法。固化清单与数值由集成 owner 在实施期登记。

### D9 · 失败处理与回退

| 失败面 | 处理 | 回退 |
|---|---|---|
| 命名策略无法判定（缺证据） | 不判新生、**不冻结**，按未知处理（待既有等待 / 人工面）+ 披露未知 | 同左 |
| 交付含 `AC-` criterion 但 lineage 为 `WicV1` | 编译/提升边界拒收，出 typed 诊断 | 同左 |
| 交付仅前缀升级整组身份 | 拒收，不降级为「自动改名」 | 同左 |
| scope 代际与 session 代际不一致 | `Fatal(ProtocolViolation)`，durable failed | 不得改配置为人工门「绕过」 |
| 未知代际取值 | 按状态损坏拒收 | 不得猜测为已知代际后继续消费 |
| 旧二进制回滚 | `RunHistory` 为 `deny_unknown_fields`，旧二进制读到新字段会拒收 | **不得直接回滚旧二进制**；回退只能停在「停新生 WIC 生成」这一层 |
| `IssueWorkItemPlan` 含新命名策略字段后的旧二进制回滚 | 该实体**无** `deny_unknown_fields`，旧二进制读改写 plan 记录会**静默丢弃**新字段，「已冻结不可改」在回滚后失效 | **不得直接回滚旧二进制**；命名策略字段的写入与新二进制同批上线 |
| `ReviewInvocationScope` / Raw DTO（`types.rs:259,360` 带 `deny_unknown_fields`）加版本字段 | 旧二进制读到带版本的 scope 记录会**拒收** | 同上；scope 版本字段的写入与新二进制同批上线 |

回退的最小粒度：**只停新 WIC 生成**（把新生路径退回只读兼容），不删双前缀 reader、不清 history、不重算指纹、不删除已落盘的 v3 记录。

## 共享面与排序

本 change 与多个 WIG change 存在共享文件，**必须串行，不得并发各自改同一文件**。下表按裁决处置 §2.2 的全局串行顺序做**双边登记**：既写本 change 的诉求，也写共写包的诉求与相对次序。

| 共享面 | 本 change 的诉求 | 共写包与其诉求 | 归属与排序 |
|---|---|---|---|
| `src/web/workspace_ws_handler/run/single_candidate.rs`（handler） | `:404-437` 保留真实 spec 域 `forged` 检查，**删除 `forged_done_when` 与混合错误文案**；`:458-487` 教学分清 spec / plan 域 | 决策六拆分（prevalidation / teaching / tests 模块）→ `mechanicalize`（typed 判据）→ `bound`（claim / 教学）→ `effort`（env 装配） | 决策六 → mechanicalize → bound → effort → **isolate（本 change 最后落，不覆盖前序语义）** |
| `src/product/workspace_engine/prompts.rs` | `:244-249` 教学分清 spec / plan 域 | 决策六拆分（`prompts/sc_compile_teaching.rs` 等）→ `effort`（SC author 出口） | 决策六 → effort → **isolate** |
| `src/product/work_item_split_engine/prompts.rs` | `:185,306` ID 纪律加入 WIC（`:306` 的 spec Traceability `AC-` 示例**不替换**） | 无其它 WIG 包共写 | 本 change 独占该两处 |
| `src/product/lifecycle_store/workspace.rs` | 创建点 `:325` 的 `run_history: Default::default()` 改为写入显式代际；接管 `:485` 的 `run_history.clone()` 已覆盖、**不改** | `bound`（窗口 / ledger 段）→ `effort`（创建 / 接管逐字段） | bound → effort → **isolate** |
| `src/product/lifecycle_store/plan.rs` | `:36-125,132-187` 创建 / ensure 与 SC 准入共同保持命名策略元数据 | 无其它 WIG 包共写；大文件 cap 1683 不得涨，接线前先纯移动（见 tasks 前置工作包） | 本 change 独占；cap 不涨 |
| `src/product/work_item_plan_policy/classify.rs` | `identity_for_finding` 调用点 `:141` 先行落地版本传递 | `close-contract-findings-before-pass`（闭合谓词） | **isolate → close 串行共写**：isolate 先落版本传递，close 只加闭合谓词、不改版本语义 |
| `src/product/work_item_plan_policy/types.rs` | `RunHistory` 代际字段与 scope 版本字段 | `bound`（ledger entry） | bound → **isolate** |
| 其余 `src/product/work_item_plan_policy/**`（`fingerprint.rs`、`evaluate.rs` 等） | 指纹代际与命名策略元数据面：`identity_for_finding` 全部 4 处调用点（`classify.rs:141`、`workspace_engine/review/routing_platform_rescue.rs:178`、`workspace_engine/review/routing_scope.rs:651`、`workspace_engine/review/routing.rs:468`）与 `evaluate.rs:184` 重建点 | 无其它 WIG 包共写 | 本 change 独占**指纹代际与命名策略元数据面**（`classify.rs` 除外，见上行；**不是**整个 `work_item_plan_policy/*` 目录独占） |
| `src/product/work_item_split_engine/prompts/sc_author_teaching.rs` | ID 纪律段落的例子改 `WIC-`，并区分「新生 WIC」与「legacy 修订保留 AC」两条分支 | `mechanicalize` 波次 7「prompt 分层瘦身」（≤8K 字符 / ≤25 硬措辞） | **串行**：`mechanicalize` 瘦身先落，本 change WIC 措辞在其后；顺序相反则瘦身度量基线被本 change 改动，验收失真（双边登记：mechanicalize 侧同样登记本 change 后续接手） |
| `src/product/work_item_split_engine/tests/prompt_contract*.rs`（`prompt_contract.rs`、`prompt_contract_weak_model.rs`、`prompt_contract_sc_revision.rs`） | 断言新生成例子为 `WIC-`，保留真实历史 `AC-` fixture 作兼容用例 | `mechanicalize` 按新契约更新既有断言（不许删测试） | **串行**：mechanicalize 先行；两个 change 的断言合并在同一提交里落地，避免同一断言被两处改写 |
| `src/product/work_item_plan_compiler/fixtures/*`（含 `compiler-diagnostics/`） | 新生成例子迁 `WIC-`；保留真实历史 AC fixture 作为兼容测试；新增命名/修订矩阵 fixture | `mechanicalize` 编译器 fixture 随 typed 诊断口径更新 | **串行**：mechanicalize 先行；fixture 目录为单一所有权点，同一提交内先改 fixture 再改断言 |
| `src/product/work_item_plan_compiler/parse.rs` | criterion 分支双前缀 | `mechanicalize` 波次 2 的 lowering typed fault 落在 `lower.rs` / `types.rs`，不改 `parse.rs` | **可并行**（文件不重叠） |
| `openspec/changes/mechanicalize-plan-compile-repair/specs/work-item-plan-single-candidate/spec.md` 的 **MODIFIED REQ-WSC-09** | 本 change **不改**该文件；守卫 typed fault 的教学重驱资格**引用**其 typed 资格清单（见 D2） | `mechanicalize` 独占；决策 §1.4 要求在已有 MODIFIED WSC-09 中加入 typed IR 与 structure | **归属 `mechanicalize`（集成 owner 统一执行）**；本 change 的 delta 只写不与 WSC-09 重复的部分 |
| `src/product/workspace_engine/single_candidate.rs` / `conversational_gate_parts/revision_turn.inc.rs`（编译与提升边界） | 命名域守卫接在这两个边界 | 决策六的 handler 拆分（`run/single_candidate_prevalidation.rs` / `single_candidate_teaching.rs`）会搬动同一批代码 | **串行**：先由决策六的纯拆分落地（行为不变），本 change 再接守卫；否则守卫的落点会在拆分中被搬走 |

**排序结论**：决策六纯拆分 → `mechanicalize`（prompt 瘦身、fixture / 断言、typed 判据）→ `bound` → `effort` → **本 change**（WIC 措辞与 fixture 更新、`forged_done_when` 删除、守卫接线）→ `close`（消费 isolate 的版本化 scope、`classify.rs` 闭合谓词）。`work_item_plan_policy/**` 的指纹代际与命名策略元数据面与 `mechanicalize` / `bound` / `effort` 无文件重叠，可先行；`classify.rs` 与 `types.rs` 除外（按上表次序串行）。

## Risks / Trade-offs

| 风险 | 缓解 |
|---|---|
| 旧 lineage 仍存在 `AC-` 与 spec 域 `AC-` 同串，命名隔离**没有**消除历史歧义 | 明确承认：域由上下文（哪个集合、哪类引用）而非字面区分；评审输入侧按域解释（plan 域集合 vs spec 域清单）。**不得声称命名已消除历史歧义** |
| 兼容算法（v2）与新算法（v3）长期并存，维护面变大 | v2 是冻结历史的只读路径，不接受任何新写入；新写入只走 v3；并存期的终点由后续数据迁移项目决定 |
| 新代际的 scope 摘要配方变更，旧 scope 不可校验 | v2 scope 配方逐字保留，旧 scope 仍可校验；代际不一致按 `Fatal(ProtocolViolation)` 失败关闭而不是重算 |
| `RunHistory` 为 `deny_unknown_fields`，新增字段使旧二进制读不了新记录 | 回退策略已声明「不得直接回滚旧二进制」；发布顺序要求新字段写入与新版本二进制同批上线 |
| 教学材料改动影响弱模型输出质量 | 新旧两条分支各自给例子；真实历史 AC fixture 保留为兼容用例，不全仓替换；质量验收按项目 prompt 验证规则执行 |
| 命名守卫把「模型写错前缀」误判为业务缺陷并扩大人工门 | 守卫只判前缀与命名域，不判语义；`AC-` 在 `WicV1` lineage 上是可教学的类型化诊断，交既有教学通道处理，不新增 attempt |

## Migration Plan

1. **锚定**：在**未改代码**上固化 v2 的原始 fixture 哈希（finding identity 与 scope digest 逐位值）与既有断言基线。
2. **v2 保留**：把现有 v2 身份配方与 scope 摘要配方原样保留为「旧代际」，新增代际分发点，默认仍解析为 v2。
3. **v3 引入**：新增带 token 边界的 token 扫描（取/删同源），新增 v3 稳定 ID 前缀集合与 v3 盐，scope 摘要绑定版本；所有消费面显式从 `RunHistory` 取代际。
4. **代际落盘**：`RunHistory` 加可缺省代际字段（缺省 v2）；新建受控会话显式写 v3；接管沿用父代际。
5. **命名策略落盘**：plan 记录加可缺省命名策略字段；在真实 SC 首次生成准入冻结（`WicV1` / `LegacyPreserved`；未知不冻结、披露未知）。CAS 载体＝plan 的 `membership_revision`（不足则新增独立 revision 字段），见 D3。
6. **兼容读取上线**：parser 双前缀只读兼容与旧 fixture 兼容用例同批上线；此时新旧会话可并存。
7. **新生强制**：新 lineage 开始按 `WicV1` 产出 `WIC-`；教学材料同步给出「新生 / legacy 修订」两条分支。

**回退**：停在第 6 步（只读兼容已上、新生未强制）即可回到「不产生新 `WIC-`」的状态；已落盘的 v3 记录与已冻结的 `WicV1` 策略**不得**通过改配置回退成 v2 / `LegacyPreserved`；旧二进制**不得**直接回滚。

## Open Questions

以下三项**不改变**本 change 的 specs、架构选型与工作包拆分，可在实施期或后续立项处理：

1. 旧代际 v2 算法的**退役条件与退役项目归属**——决策 §9 已把「全历史 v2 指纹重算与移除旧算法」划入后续数据迁移项目，本期只保证可读，故不阻塞。
2. 评审输入侧如何**呈现**同串歧义（是否在 reviewer 投影里显式标注域标签）——`work_item_projection/reviewer.rs` 现有投影已按「criterion 集合（plan 域）/ requirement 清单（spec 域）」分列，是否再加可见标注属呈现面（E 包）范围，不影响本 change 的契约。
3. 原始 fixture 哈希锚点的**清单选取**（哪几个 fixture 构成回归基线）——由集成 owner 在实施期确定并登记，不改变契约。

**已由 2026-11-09 处置裁决定案**：命名策略元数据的持久化载体＝plan 记录（`IssueWorkItemPlan` 可缺省字段）+ plan 文件锁内 RMW，CAS 载体＝plan 的 `membership_revision`（不足则新增独立 revision 字段），**不得**依赖 session record 的 CAS（见 D3）——原「载体选 plan 还是 session」的开放项就此关闭，不再列为需 owner 裁决项。
