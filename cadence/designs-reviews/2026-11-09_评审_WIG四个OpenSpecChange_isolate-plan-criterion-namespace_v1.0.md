# 评审：isolate-plan-criterion-namespace（v1.0）

- 评审对象：`openspec/changes/isolate-plan-criterion-namespace/`（proposal 57 / design 172 / tasks 52 / WSC delta 84 / TOP delta 268）
- 依据：`cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §5、§6.3
- 行号均以 `.worktrees/feat-wig-optimize-11-08/` 为根；本报告未修改任何被审文件，未 git add。

**状态：PASS_WITH_ISSUES**（无阻断项；15 项问题，其中重要 11、次要 4）

**一句话结论**：契约主干（双前缀、词法不放宽、命名策略三态、存量保 ID、指纹代际按会话固定、不 bump compiler）与决策 §5.1~§5.6 基本逐条对应，strict 校验通过；但**共享面登记不完整且与 effort 互相矛盾**、决策 §5.4 的 handler 删改项（`forged_done_when` 等）**未入 tasks**、`REQ-WSC-31` 冻结点/CAS 载体未闭合，实施前应先补这些。

## 问题清单

| 编号 | 严重度 | 位置 | 问题 | 证据 | 建议修法 |
|---|---|---|---|---|---|
| R-01 | 重要 | design.md:125-139「共享面与排序」；effort/proposal.md:62 | **跨 change 共享面不一致（核心）**：WIC 的 D6 要改 `lifecycle_store/workspace.rs` 创建点（`run_history: Default::default()`，实测 `:325`，design 写 `:326`）并依赖接管 `:485` 的 `run_history.clone()`，但共享面表**只登记 mechanicalize 与决策六**，未登记该文件。effort 在 proposal.md:62 声称「与 isolate 无共享写面」，而 effort/design.md 共享面第 3 行登记该文件 `:324-335`（创建）/`:481-499`（接管），bound-sc/design.md:14,103 也依赖 `:481-499`。三个 change 实际同写 workspace.rs 创建 struct 字面量，登记不对称（WIC 缺、effort 声明与事实不符、bound 未列 effort 依赖）。`models/workspace.rs` SC 字段区**不是**WIC 写面（`RunHistory` 按值嵌入，新字段落在 `work_item_plan_policy/types.rs`），此处无冲突 | grep 结果：effort design 行 126-127；effort proposal:62；workspace.rs:325/485 | WIC design 共享面表新增一行：`lifecycle_store/workspace.rs`（创建写显式代际；接管无需改动，clone 天然继承），与 effort 冻结策略写入、bound 窗口引用串行，顺序由集成 owner 定；effort proposal:62 改为「共享 lifecycle_store/workspace.rs 创建点」。该文件登记 cap 1730（实测 1728），见 R-11 |
| R-02 | 重要 | design.md:137；tasks.md 全文；决策 §5.4 | **`run/single_candidate.rs` handler 未登记且决策要求的改动无 task**：决策 §5.4 要求删除 `single_candidate.rs:404-437` 的 `forged_done_when` 及混合错误文案、改两处教学（`:458-487`、`workspace_engine/prompts.rs:244-249`）、`work_item_split_engine/prompts.rs:185,306`。本 change 全文对 `forged`、`prompts.rs`、`lower.rs:610` 零命中。现状：handler 对 `done_when_refs` 中 `AC-` 开头且不在 spec 注册集的引用判伪造（`:416-427`），与 `REQ-WSC-30`「plan 域 ID 只服务本 WI criterion」直接张力：LegacyPreserved 的 plan 域 `AC-001` 若不在 spec 清单会被误拒。同文件另有 effort（`:1096-1112`,`:510`）、bound、mechanicalize（`:84-100`,`:1301-1313`）、决策六拆分（`single_candidate_prevalidation.rs`/`_teaching.rs`）写入 | grep `forged` 于 change 目录：0 命中；single_candidate.rs:412-431 | tasks 增一项：删 `forged_done_when` 分支但保留 `design_traceability` 的 `forged`；WSC-30 增场景「plan 域 AC- 引用不得被 spec 注册集误判」；共享面表登记该 handler，并改写 design:137 行（该行把 engine 边界与决策六 handler 拆分混为一谈，决策六不搬 `workspace_engine/single_candidate.rs`，但 §6.2 要把 engine completion 的布尔授权参数改为 assessment/claim 接口，与 bound 的硬检同点） |
| R-03 | 重要 | design.md:137,D9；tasks 2.2 | **守卫接线次序与 handler 预检缺失**：守卫落 `workspace_engine/single_candidate.rs` 编译/提升边界与 `revision_turn.inc.rs`，但决策 §5.2 另要求「handler 预检用同一规则提前诊断，权威 completion 仍复验」，design/tasks 只写编译/提升/人工修订边界。与 bound 的 C 硬检（置于新交付 compile 成功后、预检/refs 提升前）、mechanicalize 的 typed 资格判据的相对顺序未定 | 决策 §6.2 末段；tasks 2.2 | D3/D9 补：预检位置、复验位置、与 C 硬检的先后；revision_turn.inc.rs:1158-1216 本轮**未核** |
| R-04 | 重要 | design.md D9「交付含 AC- 但 lineage 为 WicV1」；risk 表末行 | **守卫诊断的教学资格未定**：`AC-` 在双前缀下语法合法，**不会**触发 `invalid_work_item_id`；`WSC-09`（主规格:480）教学重驱资格限 parse 语法类（至少 missing_section/invalid_ears/invalid_work_item_id），`grammar.rs:140` `DIAGNOSTIC_CODES` 为定长 4 数组。design 只写「交既有教学通道、不新增 attempt」，未说明走 WSC-09 重驱窗口、bound 的 C 窗口还是不可教学直接拒收，也无独立 WSC-31 场景断言「WicV1 交付含 AC- 被拒」（仅隐含在场景 1 THEN） | grammar.rs:140-145；主 spec:480 | 明确新增 typed fault 并规定其教学资格与是否消耗 C 窗口；WSC-31 增独立场景 |
| R-05 | 重要 | design.md:135「work_item_plan_policy/* 本 change 独占」；D6 | **策略面「独占、与串行链无重叠、可先行」不成立**：`identity_for_finding` 实有三处调用（`classify.rs:141`、`routing_platform_rescue.rs:178`、`routing_scope.rs:651`），决策 §5.3 另列 `routing.rs:468`；这些文件在 close-contract/design.md:155-165 已与 fix-author、mechanicalize 登记串行（fix-author → mechanicalize → close-contract），WIC 不在其表内、其表也不在 WIC 内。另 `evaluate.rs:184` 以**全字段字面量**重建 `RunHistory`，新字段若漏拷或用 `..Default::default()` 会静默回落 v2；`workspace_policy_route.rs:80,104` 整体替换 `stored.run_history`；`session_state.rs:594` 用 `RunHistory::default()` | grep 结果；evaluate.rs:184-192 | 共享面表增这些行并定序（建议先落「纯机械的代际参数化、默认 v2、零行为变化」提交）；tasks 4.3/4.4 增断言「delta apply 与 policy-route 持久化后代际不变」；close-contract 表需同步增 WIC 行（本报告不改他 change，转 Main） |
| R-06 | 重要 | WSC delta:46-64；design D3 | **REQ-WSC-31 冻结点与三态未闭合**：场景 1 缺决策 §5.2 的「且有真实新生事实」合取条件；场景 2 的触发「存量 lineage 首次被本平台识别」在 D3 无定义（D3 冻结点仅「真实 SC 首次生成准入」）；场景 4「未知」未说明是否冻结、可否后续重评（与「已冻结不可改」冲突）。CAS 载体不明：`IssueWorkItemPlan`（`models/lifecycle.rs:299`）无 revision 字段（plan.rs 仅有 `membership_revision`），CAS 仅在 session record 上有惯用法；且「lineage」与 `work-item-revisions/{plan}/lineage.json` 同名异物 | lifecycle.rs:299-323；plan.rs:707 | D3 指明载体（plan 记录字段 + 文件锁内 RMW，或 lineage.json）与「未知态不冻结」；场景 1/2/4 补条件 |
| R-07 | 重要 | design.md D9 回退表 | **回退风险只覆盖 RunHistory**：`IssueWorkItemPlan` 无 `deny_unknown_fields`，旧二进制读改写会**静默丢弃**新命名策略字段（[INFERENCE]，依 serde 语义），使「已冻结不可改」在回滚后失效；`ReviewInvocationScope`/Raw DTO 带 `deny_unknown_fields`（types.rs:259,360），加版本字段同样令旧二进制拒收，D9 未列 | lifecycle.rs:299-320；types.rs:259,360 | D9 补两项；发布顺序注明与新二进制同批 |
| R-08 | 重要 | WSC delta；tasks 2.4,7.3 | **compiler_version 不 bump / IR 不漂移无 spec 场景**：D7「失败即停」与 tasks 7.3 只在 design/tasks，WSC-30 仅有「source 字节不改写」，不覆盖 lower 结果、IR 内容寻址、旧 `compiler_version` 仍可读/复验/发布、不同版本仍按 freshness 拒绝；tasks 7.3 无对应 requirement 场景 | WSC delta:32-35 | WSC-30 增场景（WIC/AC 双前缀下旧 source 的 IR 与内容寻址不漂移；freshness 不放宽） |
| R-09 | 重要 | TOP delta:82,149-172；D5 | **token 边界/取删同源/最长前缀无契约场景**：TOP-04 保留场景仍写「受限提取 WI/CT/AC」（:82，未含 WIC）；新增场景只讲代际固定/scope，未断言 `WAC-001` 不抽成 `AC-001`、`WIC-001≠AC-001`、取与删一致。另实测 v2 对 `WIC-001` 抽不到任何稳定 ID（`WI-`/`AC-` 皆非子串）→ 进 unstable 域强制人工（fingerprint.rs:98-108），这才是 WicV1 需要 v3 的直接理由，D5 只论证 WAC。「最长前缀先」在现有前缀集合下是空约束，无害 | fingerprint.rs:143-206 | TOP-04 增「新代际稳定 ID 扫描」场景；D5 补 WIC 理由 |
| R-10 | 重要 | WSC delta:44,59；design D4 | 决策 §5.2:「旧 baseline 的 ID 改变/消失按修复闭包与业务授权处理」；delta/D4 写成绝对「MUST NOT 删除/重编号/替换」，会挡住 owner 授权的合法删除。主规格 grep 未找到另一处 protected-field 表述，故以决策原文为准 | 决策 §5.2 | 改为「无授权不得…；授权删除走修复闭包」，保留「仅前缀升级」硬拒 |
| R-11 | 重要 | 全 change 无「行数」字样 | **大文件门禁未提**：WIC 触及 `lifecycle_store/plan.rs`（1682/cap 1683）、`lifecycle_store/workspace.rs`（1728/cap 1730）、`revision_turn.inc.rs`（1505/cap 1505，余量 0），决策 §6.3 明言未为其申请例外，须先把完整内聚方法/类型移到新文件再接线 | large_file_guard.rs:120,126,190-191 | tasks 增前置：纯移动后再接线；命名域模块放新文件 |
| R-12 | 次要 | TOP delta:167；types.rs:407-441 | 「新代际 scope 绑定版本与 original 集合」：Initial 无 original 集合；Verification 摘要**本已**逐项绑定 `original_fingerprints`，且 `canonical_prefix` 已含 `SCOPE_DIGEST_VERSION`。新代际是否保留 `review_scope_v1:` 前缀（主规格场景「范围摘要版本化编码」写死该前缀）未表态 | types.rs:9-10,407-441 | 改写为「摘要输入含代际标识；Verification 保持既有 original 绑定」，并写明前缀处理 |
| R-13 | 次要 | TOP delta:178,250 | 「受控会话」与「受控单候选会话」未定义且措辞不一（`RunHistory` 创建点含 Legacy/非 SC 路径，如 session_state.rs:594） | workspace.rs:325 | 定义为 `flow_kind=SingleCandidate` 创建路径；Legacy 保持缺省 v2 |
| R-14 | 次要 | design D6 | `fingerprint_version` 键名/取值域/未知值如何映射到「状态损坏」未钉（`RunHistory` 是 `deny_unknown_fields`，需自定义反序列化而非 serde default）；决策 §5.3「诊断区分新 finding 与身份上下文不一致」仅半覆盖 | 决策 §5.3 末行 | design 钉键名与值域 |
| R-15 | 次要 | tasks.md | 组 1/6/7 标题无 REQ 映射；7.4 写字面命令（与首段「命令由 writing-plans 承担」自相矛盾）；design.md 行号漂移 ±1~3（workspace.rs:326→325；STABLE_ID_PREFIXES 实为 `:8`，`remove_stable_ids` 实为 `:174`）；design:131-133 称 mechanicalize 与 WIC 共享 `sc_author_teaching.rs`/fixtures/prompt_contract，但 mechanicalize 的 tasks 7.1-7.3 与 design 均未点名这些文件（仅 design:72 引用 `sc_author_teaching.rs:41`），单边登记 | mechanicalize/tasks.md:45-50 | 补 REQ 标注；对账为双边登记 |

## A~F 逐维度结论

- **A 保全（引用 controller 已复核，未重查）**：通过。REQ-TOP-04 主 28 / delta 33，REQ-TOP-05 主 13 / delta 18，原场景全在。我另用 `openspec show --json` 对账 delta 计数：WSC-30 = 7、WSC-31 = 8、TOP-04 = 33、TOP-05 = 18。TOP-05 内「运行中变更策略被拒」标题重复两次，来自主规格既有内容，非本 change 引入。
- **B 与决策 §5.1~§5.6**：
  - 已落契约：双前缀 `["WIC-","AC-"]` 且仅 criterion 分支（WSC-30 / D1 / task 2.1）；词法不放宽，`WAC-`、大小写、空后缀、`WIC-001a` 皆拒（WSC-30 场景 3）；一次 seen 插入而非两次 validator（D1）；存量保 ID、新生强制 WIC、拒「仅前缀升级」（WSC-31 / D4）；server-only `WicV1`/`LegacyPreserved`、客户端不得绕过、重入/接管不覆盖（WSC-31 / D3）；指纹代际按会话固定、缺省 v2、新建显式新代际、接管沿用、未知代际按状态损坏、缺证据不清 history 不充预算（TOP-04/05）；compiler 不 bump（D7）；「不得断言 WAC 必然完整 hash 碰撞」（D5 末段、proposal Why）。
  - 缺口：R-02（`forged_done_when` 删除与教学面）、R-03（handler 预检）、R-08（不 bump 无场景）、R-09（token 边界无场景）、R-10（授权例外被硬化）。
- **C 与主规格字面冲突**：未发现字面冲突。已核：主 spec:480 的 WSC-09 教学资格（见 R-04 的衔接问题）；`review_scope_v1:` 前缀场景（R-12 待表态）；`RunHistory` 的 `deny_unknown_fields`/缺省场景与新代际字段兼容，但需自定义反序列化（R-14）。`openspec/specs` 中 `AC-\d`/`criterion_id`/`invalid_work_item_id` 仅 process-evidence-acceptance-boundary 与 WSC-09 命中，均不冲突。
- **D 跨 change 共享面**：见 R-01、R-02、R-05。
  - 对称性问题：WIC 表只登记 mechanicalize 与决策六；effort 登记 9 行含 bound 却声称与 WIC 无共享写面；bound 登记 8 行未提 effort 对同文件的依赖（主 controller 另核结论，本报告引用不重述）；close-contract 的串行表不含 WIC。
  - **models/workspace.rs SC 字段区**：WIC 不写，无冲突。
  - **lifecycle_store/workspace.rs**：WIC 仅需改创建点一行，接管 `:485` 的 clone 已覆盖。登记缺失，见 R-01。
  - **run/single_candidate.rs**：WIC 决策要求改而 design 未登记，见 R-02。与 effort（env 装配）、bound（教学驱动）、mechanicalize（typed 资格判据）、决策六（拆分先落）同写。
- **E 代码接地**（逐项实测）：
  - `parse.rs:443` criterion 分支为 `("AC-", ...)` 单前缀，准确；`validate_identifier:460`、`is_valid_identifier:488` 准确。`validate_identifier` 现取单个 `prefix: &str`，须改为切片，D1 的「一次 seen 插入」可行。
  - `fingerprint.rs:8` `STABLE_ID_PREFIXES: [&str; 3]` 定长数组，加 `WIC-` 须改类型（design 写 `:7`，偏 1 行）；`stable_ids:143`（find，无边界）与 `remove_stable_ids:174`（starts_with）确为分别实现，无 token 边界，design「共用逻辑」表述不精确，v3 收为一份是有意义的变更。
  - `types.rs:188-214` `RunHistory` 带 `#[serde(default, deny_unknown_fields)]`，准确；`types.rs:9-10` `review_scope_v1:` 与 `review-invocation-scope/v1`，`:407-441` 摘要配方，准确。
  - `grammar.rs:148` compiler 版本常量准确。
  - 代际缺省落点：`models/workspace.rs` 测试断言 `RunHistory::default()` 与旧默认相等，须保持 Default 为旧代际。
- **F tasks 与验收**：tasks 均为高层工作包且多数带「验证」，可执行性好；问题见 R-15（缺 REQ 映射、7.4 字面命令）与 R-08（7.3 无场景承载）。

## strict 校验与 deltas 真实输出

命令一：`openspec validate isolate-plan-criterion-namespace --strict`（exit 0）

```
Change 'isolate-plan-criterion-namespace' is valid
```

命令二：`openspec show isolate-plan-criterion-namespace --deltas-only`（exit 0）。**文本模式实际打印的是 proposal 全文**（以 `# Proposal` 开头），并未只列 delta，此为 CLI 行为，非 change 缺陷。

命令三：`openspec show isolate-plan-criterion-namespace --json --deltas-only` 经 python 压缩后的摘要（**压缩摘要，非原始 JSON**）：

```
deltaCount 4
work-item-plan-single-candidate ADDED plan 域 criterion 命名域与双前缀只读兼容（REQ-WSC-30） scenarios= 7
work-item-plan-single-candidate ADDED 服务端 lineage 命名策略与旧实体 ID 保持（REQ-WSC-31） scenarios= 8
work-item-typed-outcome-policy MODIFIED review 两阶段与 scope 契约（REQ-TOP-04） scenarios= 33
work-item-typed-outcome-policy MODIFIED 运行状态持久化、CAS 与兼容（REQ-TOP-05） scenarios= 18
```

## 已核验通过项

- REQ 编号无重号（WSC-30/31 归本 change；28~28E 归 bound；29 归 effort；26/27 已归档）；四个 change 无同一 requirement 被重复 MODIFIED（controller 复核，引用）。
- 指纹代际已入 TOP-04/05 契约，spec 使用「旧代际/新代际」抽象，design 使用 v2/v3 字面，分工合理（controller 复核，引用）。
- 全 change 无 32 位以上十六进制串：我重跑 `grep -rnE '[0-9a-f]{32,}|sha256:[0-9a-f]+|review_scope_v1:[0-9a-f]+'` 零输出，符合「不预写哈希数值」。
- 被审目录文件未改：`git status --short -- openspec` 无输出。
- 双前缀不扩引用面：`validation.rs` 成员校验为闭集字符串比较，D2 论证成立。

## 未核实缺口

- `revision_turn.inc.rs:1158-1216` 与 `workspace_engine/single_candidate.rs:470-518` 的守卫落点：只读了 `single_candidate.rs` 约 465-520 的编译/落盘段，未逐行确认提升边界；revision_turn 未读。
- `plan.rs:36-125,132-187` 创建/ensure 的锁与写入细节只 grep 未精读，R-06 的 CAS 载体结论依赖「无 revision 字段」的字段级观察。
- R-07 的「旧二进制静默丢字段」为 serde 语义推断，未实测。
- v1.3 主方案 §8.0 行 E 提到的 `MODIFIED REQ-WSC-06` 是否由某 change 承担：grep 活动 change 仅 mechanicalize 命中 `REQ-WSC-06`，归属本 change 与否未能确认；v1.3 `:113-115` 仍保留「WAC 必然碰撞」旧措辞（`:1092` 已纠正），属主方案文本，不在本 change 范围，转 Main。
- 上一轮转录中 I-系列的其余细项，已在本表合并；若 Main 需要原始细分条目可再取。
- 未运行任何测试或构建（本任务仅评审）。
