# Tasks: isolate-plan-criterion-namespace

> 纪律：每个工作包先固化/先红再改实现（TDD）；精确文件、命令、提交步骤由 `writing-plans` 写进 `cadence/plans/`，本文件只给高层可验收工作包。
> 契约来源：`specs/work-item-plan-single-candidate/spec.md`（ADDED `REQ-WSC-30` / `REQ-WSC-31`）、
> `specs/work-item-typed-outcome-policy/spec.md`（MODIFIED `REQ-TOP-04` / `REQ-TOP-05`）；
> 架构边界与共享面排序见 `design.md`。
> **排序前提**：`design.md`「共享面与排序」已声明的串行链（决策六纯拆分 → mechanicalize 的 prompt 瘦身与其 fixture/断言、typed 判据 → bound → effort → 本 change 的 WIC 措辞与 fixture、`forged_done_when` 删除 → 守卫接线 → close 消费版本化 scope）必须被遵守；指纹代际与命名策略元数据面（`work_item_plan_policy/**` 除 `classify.rs` / `types.rs` 外）无文件重叠，可先行。
>
> **owner 2026-11-11：当前无任何存量内容** ⇒ 见 `design.md` Context 的对照表。**本 change 的契约与实现要求
> 一律不变**；只是下列**仅与存量相关**的验收格**本期无对象**（不会被触发，可标注为「无对象」而非
> 「未实现」）：① 1.1 的旧代际 fixture 哈希锚点固化、② 2.4 的「既有存量 source 逐字不变」、
> ③ 3.4 / 6.3 的 `LegacyPreserved` 保基线 ID 与存量矩阵格、④ 7.2 的「旧 lineage 修订可用 /
> 旧 session 恢复命中旧重复」。**v2 配方与读路径本身仍须逐字保留**（契约「缺省即 v2」「未知取值
> 拒收」的必要条件），只是运行时不被触发。**真正不做**的是**存量实体迁移**（WIC→AC 反写、
> 批量改前缀、全历史指纹重算、旧算法退役）—— 本期不立项。

## 0. 前置：大文件不涨线与模块落位（决策六先落）

- [ ] 0.1 前置对齐：「决策六拆分大文件」（`run/single_candidate.rs`、`workspace_engine/prompts.rs` 等纯移动拆分）**先于本 change 落地**；本 change 的 handler / prompts 接线按 `design.md`「共享面与排序」在 mechanicalize → bound → effort 之后串行执行。验证：拆分提交先于本 change 的相关提交，共享面相对次序与 design 表一致
- [ ] 0.2 命名域模块放**新文件**（如 `workspace_engine/plan_criterion_namespace.rs`、`models/plan_criterion_namespace.rs`）；`lifecycle_store/plan.rs`（cap 1683）与 `lifecycle_store/workspace.rs`（cap 1730）若接线会超 cap，先把被改文件的完整内聚方法 / 类型**纯移动**到相邻新文件再接线，纯移动不得顺手改无关行为。验证：`large_file_guard` 全绿、cap 登记值未涨、纯移动提交与行为改动提交分离

## 1. 基线锚定（先于一切算法改动；REQ-WSC-30 / REQ-TOP-04）

- [ ] 1.1 在**未改实现**的状态下固化旧代际的原始 fixture 哈希锚点（既有 fixture 上产出的 finding identity 与 review scope digest 逐位值），登记为回归基线。验证：基线登记完成且可重复复现同一逐位值
- [ ] 1.2 盘点既有以 `AC-` 为例的编译器 fixture 与 prompt 契约断言，产出「迁 `WIC-` 的新生成例子」与「保留为历史兼容用例」两份清单。验证：清单逐条指明归属，两份清单无交集
- [ ] 1.3 **【防退化 · 新增】固化「同一指纹复评停止」闸门的行为基线** —— 1.1 只钉了 identity 与 digest 的**逐位值**，**没有钉闸门本身的行为**；而代际或算法一旦改错，后果是双向的：**判不出重复 finding ⇒ 无限返修**（用户抱怨③的根源），**误判重复 ⇒ 该修的不修直接放行**（比现状更糟）。故在**未改实现**的状态下，用既有 fixture 记录三类闸门行为的**期望判定与期望副作用**：① 同一 finding 在同一 session 跨轮复现 ⇒ MUST 被判重且**不**触发新一轮返修；② 同一实体不同措辞 ⇒ MUST 判为同一（同身份）；③ 不同实体 ⇒ MUST NOT 判为同一。基线含「闸门是否触发 / 触发了哪一轮 / 预算计数变化」三项，而不只是哈希值。验证：基线可在**未改实现**上复现；改实现后逐条比对**判定结果与副作用**（不只哈希）一致 —— 任一条不一致即视为退化，须定位到具体代际/配方差异后方可继续

## 2. plan 域双前缀只读兼容与命名域守卫（REQ-WSC-30）

- [ ] 2.1 编译器 criterion 分支改为接受 `WIC-` 与 `AC-` 双前缀，标识符词法（固定前缀 + 非空 ASCII 数字）逐字保留；两前缀共享**同一次**唯一性去重。验证：双前缀样例通过、`WAC-` / 大小写变体 / 空后缀 / 非数字后缀仍被拒、重复标识符仍被拒
- [ ] 2.2 新增命名域守卫（新模块），以**新增独立 typed fault** 在三处按固定次序接入同一规则：① handler 预检（同规则、提前诊断、不吞 operational 错误、不授教学）→ ② C 硬检（新交付 compile 成功后、预检 / refs 提升前）→ ③ 权威 completion 复验。新 `WicV1` lineage 的交付出现 plan 域 `AC-` criterion 时拒收并出 typed 诊断。验证：三处次序可观察（① 提前诊断、② compile 成功后提升前拒收、③ completion 以同一规则复验），新生 lineage 的 `AC-` 交付被拒、合法 `WIC-` 交付通过、存量 lineage 不受影响；该 typed fault 的教学重驱资格按 `mechanicalize` 的 MODIFIED REQ-WSC-09 typed 资格清单判定，本 change 不自行扩展 WSC-09（`grammar.rs` 的 `DIAGNOSTIC_CODES` 定长数组需随扩容，见 design D2）
- [ ] 2.3 在教学材料中显式区分 spec 域（`REQ-*` / `NFR-*` / `AC-*` → Traceability 与 `requirement_refs`）与 plan 域（criterion / `done_when_refs` / `reviewer_check_refs`），并把「spec 域 `AC-*` 不得充当本 WI criterion」写成可验收断言。验证：断言矩阵覆盖「spec AC 冒充 criterion」「他 WI 的 `WIC-` 被引用」两类负向用例且均被拒
- [ ] 2.4 回归：双前缀引入后，全 plan criterion 唯一性与本 WI 成员关系判据与引入前等价，既有存量 source 逐字不变、内容寻址不漂移。验证：既有编译与成员校验用例全绿、存量 source 字节与哈希未变
- [ ] 2.5 handler 删除 `forged_done_when` 误判与混合错误文案（`run/single_candidate.rs:404-437` 保留真实 spec 域 `forged` 检查；本 WI 引用合法性由 canonical 成员校验负责）。验证：`LegacyPreserved` lineage 的 plan 域 `AC-001` 不在 spec 注册集时**不再被判伪造**、交付照常进入既有校验；真实 spec 域伪造（`requirement_refs` 未注册）仍被拒

## 3. 服务端命名策略与准入冻结（REQ-WSC-31）

- [ ] 3.1 新增 server-only 命名策略元数据（`WicV1` / `LegacyPreserved`），以可缺省形态挂在 plan lineage 实体记录上；判定按「无历史事实且有新生事实 ⇒ `WicV1`；有历史事实 ⇒ `LegacyPreserved`；缺证据 ⇒ 不判新生并披露未知」三态。验证：三态判定各有可验收用例，未知历史不得落到 `WicV1`
- [ ] 3.2 在真实 SC 首次生成准入以 CAS 冻结命名策略；创建 / ensure / 重入 / 接管路径共同保持该字段，已冻结值不被覆盖。验证：重复准入与接管后策略不变、CAS 冲突走既有重读重评
- [ ] 3.3 客户端声明（legacy / 关闭守卫一类入参）被服务端忽略，策略只以已冻结事实为准。验证：携带绕过声明的请求不改变判定结果
- [ ] 3.4 存量保 ID 与禁止「仅前缀升级」：`LegacyPreserved` 修订保留基线既有 criterion ID，新增必须 `WIC-`，同编号前缀替换（`AC-00x` → `WIC-00x`）被拒收。验证：保留 / 新增 / 前缀升级三类用例分别得到预期结论

## 4. 指纹代际隔离（REQ-TOP-04、REQ-TOP-05）

- [ ] 4.1 把现有 v2 身份配方与 scope 摘要配方**逐字**保留为「旧代际」，引入代际分发点；默认解析结果仍为旧代际，既有行为零变化。验证：第 1 组固化的原始 fixture 哈希逐位不变
- [ ] 4.2 新增代际的稳定 ID 前缀集合（含 `WIC-`）与带 token 边界的 token 扫描：抽取与删除共用同一扫描逻辑、最长前缀先、token 前后不得紧连字母 / 数字 / 下划线。验证：`WIC-` 与 `AC-` 产出不同身份、`WAC-` 不被抽成 `AC-`、同一字符串的取与删结果一致
- [ ] 4.3 `RunHistory` 增可缺省的指纹代际字段：缺省为旧代际，旧代际记录序列化时可省略该字段；新建受控会话（`flow_kind=SingleCandidate` 创建路径；legacy 会话保持缺省旧代际）显式写入新代际；接管沿用父代际。验证：旧记录读取不报错且行为不变、新会话显式写入、接管后子会话代际与父一致；`evaluate.rs:184` 的 `RunHistory` 重建以全字段字面量**逐字段拷贝**代际字段，出现 `..Default::default()` 静默回落时该用例失败
- [ ] 4.4 所有消费面（finding identity、original / seen 集合比较、scope 构造与校验）显式从当前 `RunHistory` 取代际，无任何「最新默认」路径。验证：`identity_for_finding` 的 4 处调用点（`classify.rs:141`、`routing_platform_rescue.rs:178`、`routing_scope.rs:651`、`routing.rs:468`）全部显式传版本，存在静默走默认值的调用点时该用例失败
- [ ] 4.5 代际字段键名与取值域钉死为 `fingerprint_version` ∈ `{"v2","v3"}`；`RunHistory` 带 `deny_unknown_fields`，新字段以**自定义反序列化**处理（缺字段 → 旧代际；未知取值 → 状态损坏拒收），MUST NOT 用 serde default 把未知取值回落到任一已知代际。验证：缺字段记录按旧代际读、未知取值记录被拒收且无 default 回落路径

## 5. scope 版本绑定与代际一致性（REQ-TOP-04、REQ-TOP-05）

- [ ] 5.1 review scope 增可缺省版本：旧代际 scope 摘要配方逐字不变（含 `review_scope_v1:` 前缀；新代际摘要**保留该前缀不变**，只在摘要输入中纳入代际标识），新代际 scope 的摘要输入额外绑定代际版本与 original 集合，Verification 保持既有 `original_fingerprints` 逐项绑定。验证：既有 scope 摘要仍可校验通过、新代际 scope 在版本或 original 集合变化时摘要变化
- [ ] 5.2 scope 代际与 session 代际不一致、或摘要与所声明代际不匹配时按既有结构性协议错误 `Fatal(ProtocolViolation)` 失败关闭，不重算摘要、不降级为人工门、不触发自动返修。验证：该场景进入 durable failed 且不出现人工门或返修调用
- [ ] 5.3 缺证据或未知代际的处置：未知代际按状态损坏拒收；缺可比对的基线事实时走既有等待或人工处理路径。验证：既有 history 集合与任何预算余额在该路径下均未被清空、未被充值，重复闸门未被跳过

## 6. 教学材料与 fixture 更新（共享面，串行；REQ-WSC-30 / REQ-WSC-31）

- [ ] 6.1 按 `design.md`「共享面与排序」，在 `mechanicalize` 的 prompt 瘦身与其断言更新**之后**修改 SC author 教学与人工修订教学：ID 纪律例子改 `WIC-`，并给出「新生 WIC」与「legacy 修订保留 AC」两条分支。验证：两条分支各有可验收断言，且教学块体量门禁仍绿
- [ ] 6.2 按第 1 组的清单更新编译器 fixture 与 prompt 契约断言：新生成例子迁 `WIC-`，真实历史 `AC-` fixture 保留为兼容用例，不做全仓替换。验证：兼容用例仍按旧域通过、新用例按新域通过，两类并存
- [ ] 6.3 新增命名 / 修订矩阵 fixture（覆盖双前缀、存量保 ID、前缀升级拒绝、spec 域与 plan 域分界）。验证：矩阵每格都有对应断言且全部通过
- [ ] 6.4 两处教学分清 spec / plan 域：handler `run/single_candidate.rs:458-487` 与 `workspace_engine/prompts.rs:244-249`，禁止把 done_when 教成全局 spec 清单成员；`work_item_split_engine/prompts.rs:185,306` 的 ID 纪律加入 WIC，`:306` 的 spec Traceability `AC-` 示例**不替换**。验证：教学文案断言覆盖两域分界与「spec 域 `AC-*` 不得充当本 WI criterion」，`:306` 的 spec 示例保持原样

## 7. 验收与收口（REQ-WSC-30 / REQ-WSC-31 / REQ-TOP-04 / REQ-TOP-05）

- [ ] 7.1 契约级验收：`REQ-WSC-30` / `REQ-WSC-31` 与 MODIFIED `REQ-TOP-04` / `REQ-TOP-05` 的每个场景各有可执行断言或可观察行为证据。验证：场景清单逐条对账无遗漏项
- [ ] 7.2 兼容级验收：旧 lineage 修订可用且既有 ID 不变、旧 session 恢复后仍命中旧重复与原复评、新会话从首次 finding 起全程新代际、不因新前缀重置计数。验证：跨代际兼容用例全绿
- [ ] 7.3 版本级验收：旧 `compiler_version` 的 IR 仍可读 / 复验 / 合法发布，不同 `compiler_version` 仍按既有 freshness 拒绝；WIC/AC 双前缀下同一旧 source 的 IR 与内容寻址**不漂移**（映射 REQ-WSC-30 对应场景）。验证：既有 freshness 用例未被放宽，同一旧 source 在双前缀引入前后 IR 与内容寻址逐位一致
- [ ] 7.4 变更收口：本 change 的 strict 校验通过，且本 change 的 spec delta 不与 `mechanicalize` 的 MODIFIED `REQ-WSC-09` 重复。验证：strict 校验通过（具体命令由 `writing-plans` 阶段登记进 `cadence/plans/`），重复性逐条核对无交集
