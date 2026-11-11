# 评审 · WIG 四个 OpenSpec Change · explicit-sc-author-effort v1.0

> 审阅对象：`openspec/changes/explicit-sc-author-effort/`（proposal 63 / design 152 / tasks 67 / 两份 delta 52+42 行）。
> 根：`.worktrees/feat-wig-optimize-11-08/`；代码行号均取自 worktree `src/`。只读审查，未改任何被审文件，未 git add。
> 权威：`cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §3.1–§3.5、§9。

## 1. 状态

**PASS_WITH_ISSUES** —— 无阻断项：`openspec validate --strict` 通过，MODIFIED 块逐场景保全，REQ 编号无重号，四个 change 之间无同一 requirement 被两处 MODIFIED，决策三 B 维度的硬约束基本落成 MUST 级条款。但存在 **9 项「重要」**：与主规格 `REQ-ENV-HOST-01` 的字面张力、归档后 `validate --specs --strict` 会失败、接管/冻结点/「原运行」边界缺 delta 场景、跨 change 摘要依赖只写在一侧、共享面声明不互认。

## 2. 问题清单

| 编号 | 严重度 | 位置 | 问题 | 证据 | 建议修法 |
|---|---|---|---|---|---|
| I-01 | 重要 | `specs/work-item-plan-single-candidate/spec.md:7`、`design.md:94-99` vs 主规格 `session-policy-envelope/spec.md:504-514` | REQ-ENV-HOST-01 规定「平台不构造、不覆盖子进程环境（调用方显式 env map 仅作覆盖层）」「子进程环境继承宿主，平台不注入」，且「不将环境具体值写入摘要或日志」；本 change 由平台注入 `CLAUDE_CODE_EFFORT_LEVEL=high` 并要求日志记录 requested。字面上互相冲突，delta 与 design 均未提及该条 | 主规格 `:506` 原文「平台不构造、不覆盖子进程环境（调用方显式 env map 仅作覆盖层）」；`:514`「平台不注入」。`gateway_start.rs:648-651`、`provider_gateway.rs:1998-2001` 注释同口径（A1-b1 环境回退） | 需 owner 裁决：(a) 在 REQ-WSC-29 正文显式声明「经 caller 显式 env 覆盖层注入，属 REQ-ENV-HOST-01 允许的覆盖层，仅 SC author 出口」并说明 requested 档位记录是策略版本字段而非环境值日志；或 (b) 另加 MODIFIED REQ-ENV-HOST-01。不要放着不写 |
| I-02 | 重要 | `specs/workspace-session-resume-recovery/spec.md:7-11`（MODIFIED 正文） | MODIFIED 豁免 500 字符仅在 change 级校验；归档后主规格该 requirement 正文 649 字符（原 218），`openspec validate --specs --strict` 由 79/0 变 78/1 失败（非 strict 仍 79/0）。ADDED 的 REQ-WSC-29 为 472 字符，通过 | 本次 /tmp 副本归档模拟，见 §3-A | 把第 2、3 段的细则下沉到场景（已有对应场景），正文压回 ≤500；或由 owner 明确接受仓库只跑非 strict 的 specs 校验 |
| I-03 | 重要 | `design.md:78`、`tasks.md:34`；delta 无对应场景 | 自动→交互接管「继承原策略」只在 design/tasks，两份 delta 无任何场景。且接管子会话经**同一**创建路径 `create_workspace_session_with_id` 新建（`lifecycle_store/workspace.rs:455-470`）再逐字段复制父字段（`:485-499`）；若「创建时写新默认」，None 父会话会得到 `high` 子会话，违反「旧 None 不追溯改档」 | `lifecycle_store/workspace.rs:258-332`（创建）、`:406-499`（接管）；design.md:76 写「创建 SC 会话时写入 high」，未写 None→None | delta 增加「接管沿用父冻结事实（含缺省）」场景；design 明写接管必须原样复制父字段（None 保持 None），不得走创建默认 |
| I-04 | 重要 | `design.md:76`（冻结点=创建）vs `lifecycle.rs:1786-1789`、`controls.rs:257-264` | 冻结事实含 `provider`，且判据要求 provider==Claude；但 author_provider 在创建后仍可被 `set_provider`（PrepareContext 阶段）改写，并在 `start_generation` 由客户端快照整体覆盖。创建时写入的 `provider` 可能与实际锁定的 author 不一致，delta 与 design 均未规定 | `workspace_engine/controls.rs:257-264`；`workspace_engine/lifecycle.rs:1786-1789`；创建点 `web/handlers/lifecycle/plan_preparation.rs:223-235` 用 `workspace_config.author_provider` | 需 owner/设计裁决冻结点：建议与 isolate `REQ-WSC-31` 同口径，改为「真实 SC 首次生成准入时 CAS 冻结」，并规定 provider 以锁定快照为准；否则补充 provider 变更时的处置场景 |
| I-05 | 重要 | `spec.md:52`、resume delta `:11`、`design.md:109`「下一次真实新生成」 | 「原运行」与「真实新生成」边界未定义：Failed 的 SC 会话经 `start_generation` 在**同一 session record** 上重臂（`lifecycle.rs:1757-1763`）算同一原运行还是新生成？冻结载体是 session 记录，None 会话除非新建 session 永远无法获得新默认，则「下一次真实新生成才用新默认」不可验证 | `lifecycle.rs:1757-1763`；决策 §3.1(4)「同一 generation/会话」本身即含糊 | 未能裁决，见 §6：需 owner 明确「新生成授权」是否=新 session / 首次准入 / 重臂 |
| I-06 | 重要 | `tasks.md:17`(1.6)、`:35`(3.3)；`design.md:80,88-91`；delta 无场景 | 「未识别 policy_version 零启动+typed 错误 fail-closed」「服务端核验策略引用/摘要不一致→typed 协议错误」只存在于 design/tasks；tasks 将其映射到 REQ-WSC-29，但该 requirement 不含此行为 → 追溯映射不成立、不可按 spec 验收。另：fail-closed 是 design 自行推导（design.md:91 自述），决策 §3.4 未给 | 两份 delta 全文无 `policy_version` / `typed` / `fail` 字样 | 要么补一个场景（版本不可识别→零 provider 启动、保留原记录、不回落当前默认），要么在 tasks 中标注为实现级验收并去掉 requirement 映射；并标注超出决策原文 |
| I-07 | 重要 | `design.md:132`、`:135-139`、`tasks.md:46`；对侧 `bound-sc-generation-teaching/specs/.../spec.md:30`(REQ-WSC-28B)、`design.md:41`(D1) | 「窗口冻结输入摘要须纳入 effort 策略摘要」只在本 change 声明，且明写「由集成 owner 接上、本 change 不改其文件」。bound 侧 REQ-WSC-28B 的绑定字段清单、D1 窗口字段、tasks 1.1 均**无**该项；grep 全 bound 目录 `effort` 仅命中 design.md:34 的 Non-Goal「effort/model 不在本 change 范围」。决策 §3.4 第 4 行把此项列为硬依赖 | grep 结果见 §3-D | 建议由 controller 在 bound 的 28B 字段表追加「冻结执行策略摘要」，或在 REQ-WSC-29 增加场景「教学窗口的冻结输入包含该原运行冻结策略，不得被当前默认覆盖」；两侧都要写 |
| I-08 | 重要 | `design.md:125-132`(8 行共享面表) vs bound `design.md:139-170`、`proposal.md:63`、isolate `design.md:125-139` | 共享面声明不互认：本 change 声称与 bound 串行 `models/mod.rs`、`models/workspace.rs`、`lifecycle_store/workspace.rs`、`workspace_engine/types.rs`、`run/single_candidate.rs`、`prompts.rs`，bound 的「共享面与排序」只登记 mechanicalize/quota-pool/归档 SG，无一字提及 effort、`sc_launch_policy`；isolate 的表也未列 `prompts.rs`、`run/single_candidate.rs`，而主方案 §8 第 1 行把 `prompts.rs:248`、`single_candidate.rs:480-484` 派给 A。proposal.md:63「与 mechanicalize 无共享写面」与 mechanicalize `proposal.md:27` 的代码面含 `run/single_candidate.rs` 相矛盾；design.md:131 第 7 行称 socket.rs 与「mechanicalize/恢复相关改动」共享，mechanicalize 全部制品无 `socket.rs` | 见 §3-D grep | controller 统一在四个 change 的共享面表里互相登记；删除/更正无依据的 socket.rs 共享声明与 proposal.md:63 |
| I-09 | 重要 | resume delta `:29-31`、work-item delta `:49-52` | 「MUST NOT 增删 attempt 事实」与真实 fresh 启动语义冲突：fresh 的真实启动会按 REQ-WSC-25 追加候选尝试流水（bound 的 MODIFIED WSC-25）并占用既有预留对应的 ledger 条目；「attempt」同名异义（gate attempt_no / 候选尝试流水 / provider_start_ledger）。字面读会让合规实现判违约 | 主规格 resume `:82`「原 attempt/budget/gate 不因 native 更换重臂」；`gateway_start_parts/lc_fresh_payload_verify.inc.rs:270-285` 校验 payload 预留键已存在于 ledger | 改写为「fresh 不新增授权来源、不重置预算/预留；其真实启动照常按 WSC-25 记流水」，并限定 attempt 指 gate/授权层 attempt |
| I-10 | 次要 | `design.md:8,62,147`、`tasks.md:15` | 「基线教学」不是共用 builder 的生产调用方：`baseline_teaching.rs:379-492` 全在 `#[cfg(test)] mod author_baseline_tests`（`:337` 起）。生产的基线教学注入是 builder 内的 `append_author_baseline_teaching`（`prompts.rs:513`） | `baseline_teaching.rs:337,349,379,408,468,492` | 从作用域矩阵移除「基线教学」，改为「builder 内基线注入不受影响」 |
| I-11 | 次要 | `design.md:13` | 引 `web/handlers/workspace_session.rs:668` 作生产创建恒 SC 的证据；该行在 `mod tests`（`:461`）内且 provider=Fake。真实生产点是 `web/handlers/lifecycle/plan_preparation.rs:134`（`let flow_kind = SingleCandidate`）、`:210,235` | 同左 | 改引 plan_preparation.rs:134 |
| I-12 | 次要 | `design.md:70` | 「仅靠 flow_kind 无法把调用点分开」说过头：分流在 `ProviderRunKind`（`provider_run.rs:286-293`、`types.rs:305-316`），legacy 臂由 Legacy flow 派发（`inbound.rs:719-758`、`gateway_start.rs:798`）。结论（显式 SC 出口 + 冻结事实 Some 双判据）仍安全，但理由应改成「共用 builder 无法按 provider 区分」 | 同左 | 改写理由；不影响 delta |
| I-13 | 次要 | `design.md` 决策 2 / `tasks.md:50`(4.5) | 空 env 的作者类出口除 `prompts.rs:419` 外还有 `prompts/revision.rs:96`（Story/Design 修订）；`artifact_retry.rs:248-263` 克隆 base_input 继承其出口。未列入「本期不改」清单 | `prompts/revision.rs:96`；`provider_drive/artifact_retry.rs:248-263`；`inbound.rs:781-796` 注释表明该修订臂为非 SC 流 [INFERENCE，未跑测试] | 4.5 补列 `revision.rs:96`，并写明 retry 继承原出口 |
| I-14 | 次要 | work-item delta `:7` vs `design.md:72` | SC 人工修订（`sc_manual_revision.rs:356-366`）按 design 纳入同一冻结策略，但 REQ-WSC-29 正文/场景只写「教学、resume、fresh」，该路径靠推论 | `sc_manual_revision.rs:356-366` | 正文加「含人工修订」或在「三路复用」场景增列 |
| I-15 | 次要 | resume delta `:41-43` vs 主规格 `:60-62` 场景「恢复不换配置或身份」 | 主规格对「更换 provider 配置」的 fresh **拒绝**，delta 对「表达 effort」**忽略**；「等价执行策略」措辞无界。effort 不在 `ProviderConfigSnapshot`（`common.rs:113-119`，无 `deny_unknown_fields`），未知字段被 serde 静默丢弃，故「忽略」可测但近乎空转 | `common.rs:112-119`；`in_.rs:26-30` | 明写「仅 effort/执行策略表达被忽略；provider/权限变更仍按既有场景拒绝」，删「等价」 |
| I-16 | 次要 | `tasks.md:59-60`(5.4/5.5)、`:46-50`(4.x)、`:64-67`(6.x)、`:47`(4.2)、`:57`(5.2) | 5.4/5.5 需 owner 另授权才能做，change 内永远无法勾完（archive 模拟：`33 incomplete task(s)`）；4.x/6.x 是协调/自检条目、无独立 requirement 产出；4.2 列文件清单、5.2 点名 `large_file_guard` 偏向 plan 级细节 | archive 模拟输出 | 5.4/5.5 移到「Workflow follow-up / 授权后」小节并说明不阻归档；4.x/6.x 合并或降为说明 |
| I-17 | 次要 | `design.md` 全文 | 无 Migration/回退段。决策 §3.5 要求「新生成策略可经受控版本改回；新旧运行各按原冻结策略结束；不热切在途 provider、不清预算」 | 决策报告 §3.5「回退」 | 补一段回退语义；顺带说明新增字段对 `WorkspaceSessionRecord`（`models/workspace.rs:806`，无 deny_unknown_fields）的旧二进制回滚影响 |
| I-18 | 次要 | `design.md:51` | 「边界 1 的推论」一句主语/归属混乱（bound 的「mechanicalize 侧 MODIFIED」），与 DTO 边界无逻辑推导关系 | 同左 | 删除或移到跨 change 一节 |
| I-19 | 重要（横向，主方案） | 主方案 `…技术方案…v1.0.md:795,796,797,800,87,951,1032` | v1.3 残留：G-1 仍写「加 argv（`claude_code_provider/mod.rs`）」和「44 分钟→预计 10-15 分钟」「需 A1 裁决」，与已裁决的「不在 adapter 硬编码」「不承诺 44→10-15」（决策 §3.2、§3.3）冲突；G-2/G-3/G-6 仍指向 `prompts.rs:545` 注入，与本 change「共用 builder 保持空 env、仅 SC 出口注入」冲突；§8.0 E 行（`:922`）称 isolate 带 WSC-06 delta，实际 isolate 全目录无 WSC-06 | `sed -n 795p`；isolate grep `WSC-06` 零命中 | 改写 G-1 行；G-2/3/6 注入点改为 SC author 出口模块；核对 E-1/E-4 的 MODIFIED WSC-06 究竟由谁承载 |

## 3. 逐维度结论

### A. Delta 机械正确性 —— 通过（附 I-02）

**A1 MODIFIED 场景保全（`workspace-session-resume-recovery`「明确 fresh 只换 native 会话」）**：该 capability 无 REQ 编号体系，delta 标题 `### Requirement: 明确 fresh 只换 native 会话` 与主规格 `spec.md:48` 逐字相同（归档器按名匹配，合规）。脚本逐场景比对：

```
main scen  ['真 fresh 保留修订上下文', 'admission 不通过仍等待', '恢复不换配置或身份']
delta scen ['真 fresh 保留修订上下文', 'admission 不通过仍等待', '恢复不换配置或身份', 'fresh 不换内部执行策略', 'fresh 不重置授权与预算', '旧记录缺冻结事实不追溯改档', '兼容例外不得被宣称已冻结', '客户端不得借恢复消息改策略']
真 fresh 保留修订上下文 IDENTICAL
admission 不通过仍等待 IDENTICAL
恢复不换配置或身份 IDENTICAL
main head para1 in delta head: True
```

主 3 场景在 delta 中逐字相同，首段正文逐字保留，新增 2 段 + 5 场景。**A2** 每个 requirement ≥1 `#### Scenario:`，均 WHEN/THEN：ADDED 9 个、MODIFIED 8 个。

**A3 真实命令输出**：

```
$ openspec validate "explicit-sc-author-effort" --strict
Change 'explicit-sc-author-effort' is valid          (exit=0)

$ openspec show "explicit-sc-author-effort" --deltas-only --json   (节选)
"deltaCount": 2,
 {"spec":"work-item-plan-single-candidate","operation":"ADDED","nreq":1,"nscen":[9]},
 {"spec":"workspace-session-resume-recovery","operation":"MODIFIED","nreq":1,"nscen":[8]}
```

**A4** REQ-WSC-29 正文长度脚本实测 **472** 字符（常量 `MAX_REQUIREMENT_TEXT_LENGTH = 500`，`…/openspec/dist/core/validation/constants.js:9`），通过（Main 引用的 460 不精确，实测 472）。MODIFIED 在 change 级豁免，但**归档后主规格**不豁免：

```
# /tmp 副本（已删除），仅归档本 change：
Applying changes to openspec/specs/work-item-plan-single-candidate/spec.md:  + 1 added
Applying changes to openspec/specs/workspace-session-resume-recovery/spec.md: ~ 1 modified
Change 'explicit-sc-author-effort' archived as '2026-11-09-explicit-sc-author-effort'.   （另有 "33 incomplete task(s)" 警告，--yes 继续）
基线(主规格原样) validate --specs --strict : Totals: 79 passed, 0 failed
归档后        validate --specs            : Totals: 79 passed, 0 failed
归档后        validate --specs --strict   : ✗ spec/workspace-session-resume-recovery  Totals: 78 passed, 1 failed
  该 requirement 正文 218 → 649 字符（三段 218/241/186）
```

同一机制下，bound（SG-05）与 close（WSC-17）单独归档同样令 strict 失败，isolate 不会 → I-02 是系统性问题。仓库参考归档报告用的是非 strict `openspec validate --specs`（`.superpowers/sdd/…archive-report.md:77`），故判「重要」非「阻断」。

### B. 与决策报告 §3.1–§3.5 的语义一致性 —— 基本一致，缺口见 I-03/04/05/06/09

| 决策条目 | 落点 | 结论 |
|---|---|---|
| §3.1(1) 默认 `high` 非 `low`、不改 reviewer/Story·Design/coding/其它 provider | REQ-WSC-29 正文+场景「新的 Claude SC 原运行冻结 high」「作用域外…不变」；proposal.md:33 明写不预设 low | 一致，MUST 级 |
| §3.1(2) 内部 `ProviderEffortConfig{policy_version,provider,author_effort}`、不扩公共快照、不 adapter 硬编码 | REQ-WSC-29 正文「含策略版本、provider、author effort 的独立冻结配置…MUST NOT 暴露为公共 provider 配置项」；design.md:35-49 边界 1 | 一致。**注意**：「不 adapter 硬编码」只在 design/proposal，delta 无条款（决策本身也未要求入 spec，可接受）；但「不扩 ProviderConfigSnapshot」已是 MUST NOT（对应 Main 点名的三条之一） |
| §3.1(3) 显式设档、旧 None 不得补写成「原本 high」 | 场景「存量运行保留旧继承模式并披露未知」「旧记录缺冻结事实不追溯改档」，均含 MUST NOT 补写 | 一致 |
| §3.1(4) 同 generation/会话冻结，改档只对新的合法生成授权生效，不借改档重置预算 | 场景「恢复不得改档也不得重置授权与预算」只覆盖 fresh；「新的合法生成授权」边界未定义 | **缺口 I-05**；预算部分仅在 fresh 场景，未覆盖「策略版本改变」 |
| §3.4 行 2 创建/恢复/接管逐字段贯穿，接管继承原策略 | 仅 design.md:78、tasks 3.2 | **缺口 I-03**（delta 无场景） |
| §3.4 行 4 服务端核验 policy 引用/digest；摘要进入决策一窗口冻结输入 | 仅 design.md:80、132；对侧 bound 未接 | **缺口 I-06/I-07** |
| §3.5 兼容例外「存量 effective 不得宣称已追溯冻结」须写入恢复 delta | resume delta 正文第 3 段 + 场景「兼容例外不得被宣称已冻结」；work-item delta 对应场景 | 一致，MUST NOT 级（Main 点名第二条） |
| §3.5 真实调用另行授权（当前未授权） | **仅** proposal.md:26、design.md:112-117、tasks.md:59；两份 delta 无任何条款 | 决策 §3.5 未要求入 spec，属流程约束，可接受；但 Main 点名的第三条「落成 MUST 级条款」**未满足**，需 owner 确认是否要求入 delta（未能裁决，见 §6） |
| §3.5 恢复 payload 不能借机改 provider/权限/effort 或新增 attempt | 场景「客户端不得借恢复消息改策略」覆盖 effort；「新增 attempt」措辞见 I-09 | 部分一致 |
| §9 未决边界（low 能否默认、high 提速、可编辑 effort、review_rounds） | proposal.md 非目标显式排除；delta 未出现 | 未越界 |
| 软/硬强度 | 全部使用 SHALL/MUST NOT；MAY 仅用于兼容例外场景（effective 可能随宿主变化），与决策「不得宣称」相符 | 未把软约束写硬或反之 |

### C. 与主规格冲突 —— 一处字面张力 + 一处措辞冲突

- **I-01（REQ-ENV-HOST-01）**：见问题表。该冲突不在 `workspace-session-resume-recovery`，而在 `session-policy-envelope`；决策报告 §3.2 引用的 `gateway_start.rs:648-678` 注释正是该条的实现，说明「caller 显式 env 为覆盖层」被允许，但 delta 没有用规格语言说明。
- 与 resume 主规格：「明确 fresh 只换 native 会话」+「恢复不换配置或身份」边界。delta 新增段不与原 3 场景矛盾（fresh 只换 native，策略不变；provider 配置变更仍拒绝）。唯一措辞张力：拒绝 vs 忽略，见 I-15；attempt 字样见 I-09。
- `work-item-plan-single-candidate` 主规格 grep `effort|env_vars|CLAUDE_CODE`：零命中，无直接矛盾。

### D. 跨 change —— REQ 无重号、无双重 MODIFIED；共享面声明不互认

- **REQ 编号**：归档模拟四个 change 全序（两种顺序）均成功，合并后 `work-item-plan-single-candidate` 无重复 `REQ-WSC-*`：`25,26,27,29,30,31,28,28A..28E` 各一次。
- **同一 requirement 双重 MODIFIED**：枚举所有非 archive change 的 delta 头：effort=resume-recovery「明确 fresh…」；bound=WSC-25+`REQ-SG-05`；isolate=`REQ-TOP-04/05`；close=`REQ-WSC-17`（+ADDED TOP-07）；mechanicalize=`REQ-WSC-09`；quota-pool=`REQ-WSC-21`。**无重叠**；`workspace-session-resume-recovery` 仅本 change 触及。
- **WSC-09 归属**：全仓 delta 头中 `REQ-WSC-09` 只在 mechanicalize 出现；本 change 仅在 design.md:137/tasks 4.4 声明不改，守住。
- **共享面排序一致性**：见 I-07、I-08。已验证事实：bound 的 `design.md:103` 与本 change `design.md:78` 均要求接管路径（`lifecycle_store/workspace.rs:481-499`）处理「复制 vs 清空」，两侧均未交叉引用；本 change 称「先合冻结继承再合窗口」，bound 无对应排序声明。
- 跨 change grep（实测）：`grep -rn "effort|sc_launch_policy|WSC-29|ProviderEffortConfig"` 于 bound/isolate/close/mechanicalize/quota-pool：仅命中 bound `design.md:34`（Non-Goal）、`tasks.md:61`（词 best-effort）、isolate `proposal.md:46,57`（登记 WSC-29 归 effort）。bound 不含 `sc_launch_policy`/`sc_generation_attempt` 与 `models/mod.rs` 的任何互认。

### E. 代码接地 —— 坐标基本成立，作用域分流判断成立（理由需改），三处引用需更正

逐条核验（均 worktree `src/`）：

| 断言 | 实测 | 结论 |
|---|---|---|
| `ProviderConfigSnapshot` 无 effort | `web/workspace_ws_types/common.rs:113-119`：仅 author / reviewer / review_rounds / permission_modes（`#[serde(default)]`），无 `deny_unknown_fields` | ✔ |
| `StartGeneration.provider_config` | `workspace_ws_types/in_.rs:26-27` | ✔ |
| 由 WIG session 重建快照 | `session_state/timeline.rs:493-499` `provider_config_snapshot()` | ✔ |
| `prompts.rs:545` 空 env / `:419` 其他 author | `prompts.rs:545`、`:419` 均 `env_vars: BTreeMap::new()`；builder 实为 `:464`(`build_work_item_plan_streaming_input`)、`:490`(`_fresh`)、`:508`(`_with_session`)，「:464-548」覆盖三函数 | ✔（`:545` 属 `_with_session`，非「单个 builder」） |
| reviewer 出口 | `review_parts/single_candidate_input.inc.rs:315` 空 env | ✔ |
| coding 转换 | `coding_models/provider_config.rs:90-106`（`impl From<&ProviderConfigSnapshot>` 自 `:96`） | ✔ |
| env 透传可达 | `gateway_start.rs:648-651` 注释；`provider_gateway.rs:1998-2001`；`claude_code_provider/direct_start.rs:221`、`validated_start.rs:192` 均 `&input.env_vars` | ✔ |
| fresh 比较逐字段 | `socket.rs:1413-1427`（`lc_resume_fresh_admission` 自 `:1335`）比较 author/reviewer/permission_modes，无 review_rounds | ✔ |
| durable/runtime 坐标 | `models/workspace.rs:844-846`(ledger/phase)、`workspace_engine/types.rs:84-102,149-168`、`lifecycle_store/workspace.rs:324-335`(创建)、`:481-499`(接管) | ✔ |
| `WorkItemPlanFlowKind` 仅 Legacy/SingleCandidate | `work_item_plan_policy/types.rs:243-246` | ✔ |
| 生产创建恒 SC | `inputs.rs:233-238` 注释 ✔；`workspace_session.rs:668` ✘（测试模块内，见 I-11），真实在 `plan_preparation.rs:134` | 部分 |

**共用 builder 共享面（Main 点名核实项）**：对 `build_work_item_plan_streaming_input*` 的生产调用方实测：SC 初轮 `run/single_candidate.rs:1097,1105`；SC 教学重驱 `:510`；SC 人工修订 `provider_run/sc_manual_revision.rs:356,366`；legacy outline `provider_run/legacy_outline.rs:295,303,497`；PlanRevision `run/followups.rs:331,339,554,1038`；Draft/Batch `workspace_engine/draft_batch/runs.rs:154,162,248,256`。**共享成立**，故按 provider 一刀切会误伤 legacy/Draft/Batch/PlanRevision，「显式 SC 出口分流」判断成立。`baseline_teaching.rs:379-492` 为测试代码，**不是**生产共享方（I-10）。分流本身可由 `ProviderRunKind`（`provider_run.rs:286-293`）区分，flow_kind 说法过头（I-12）。生产 `StreamingProviderInput {` 构造点另有 `prompts/revision.rs:80-96`（I-13）。

### F. 门禁与可执行性 —— 命中阈值已落 delta；tasks 总体合规，局部越界

- 阈值：决策 §3.4「命中」→ 已有 ADDED + MODIFIED delta，proposal「OpenSpec 阈值判定」一节与之相符，通过。
- tasks 为高层工作包，每节首行有 `映射 requirement`（§1–§5）；§4、§6 为协调/自检、非 requirement 产出；4.2/5.2 点名文件/门禁测试偏细（I-16）。无命令、提交步骤，未越界进 `writing-plans`。
- 可测性：场景「宿主更高档位不覆盖」「教学/resume/fresh 三路一致」可由离线 env 装配矩阵测；「effective 记未知」可测；「客户端不得修改」因 wire 无字段近乎空转（I-15）；「下一次真实新生成」不可测（I-05）；「不增删 attempt」自相矛盾（I-09）。
- 自相矛盾核查：REQ-WSC-29 内「SHALL 记录 requested」与主规格 REQ-ENV-HOST-01「不将环境具体值写入日志」未调和（I-01）。

## 4. 已核验通过项

1. `openspec validate explicit-sc-author-effort --strict` 通过；`show --deltas-only --json` 为 ADDED 1(9 场景)+MODIFIED 1(8 场景)。
2. MODIFIED「明确 fresh 只换 native 会话」：标题逐字、首段逐字、原 3 场景逐字（脚本比对 IDENTICAL×3），无漏场景；归档后合并输出 `~ 1 modified`、无拒绝。
3. REQ-WSC-29 正文 472 ≤ 500；归档后主规格无重复 REQ 号（四 change 全序、倒序两种顺序归档均成功）。
4. 兼容例外（存量 effective 不得宣称已追溯冻结）、客户端不得改策略、不扩公共 `ProviderConfigSnapshot`、默认 `high` 非 `low`、作用域收窄、requested/effective 分离，均为 SHALL/MUST NOT 级条款（REQ-WSC-29 与 resume delta）。
5. 决策 §9 未决项（low 默认、提速承诺、可编辑 effort、review_rounds）未被本 change 带入 delta；proposal 非目标明确排除。
6. 本 change 未修改 mechanicalize / quota-pool 目录（`git status --short` 全空）；未重复写 REQ-WSC-09。
7. 共用 builder 共享、env 透传链、Claude 两处 spawn 消费 `input.env_vars`、`ProviderConfigSnapshot` 无 effort 等 E 维度坐标属实（见 §3-E 表）。
8. 归档 change `2026-11-08-gate-card-resolved-and-inflight-state` 确实占用 WSC-26/27，主规格 `:955,:998` 可见。

## 5. 横向核对（其余 3 个 change 与主方案，仅与本 change 相关部分）

- bound / isolate / close：本次 `--strict` 均通过；全序归档均成功；无 REQ 重号、无双重 MODIFIED。
- bound 单独归档令 `workspace-structural-governance`、close 单独归档令 `work-item-plan-single-candidate` 的 strict 规格校验失败（与 I-02 同机制），建议其审阅者复核；isolate 不受影响。
- 主方案 v1.3 残留见 I-19；`§8.0 E` 行的 isolate「WSC-06 delta」未找到承载（grep 零命中），提请 controller 核对。
- 注：mechanicalize 的 `work-item-plan-single-candidate/spec.md` 在 `:35` 有 `## ADDED Requirements` 小节，`:33` 前后仅见注释、`validate --strict` 通过，未深查其内容（不在范围）。

## 6. 未能裁决的缺口（需 owner 裁决，我不补全）

1. **I-01**：REQ-ENV-HOST-01（平台不注入环境、日志不含环境值）与「平台注入 `CLAUDE_CODE_EFFORT_LEVEL`」的关系：作覆盖层例外写进 REQ-WSC-29，还是 MODIFIED 该条？
2. **I-04/I-05**：冻结点是「SC 会话创建」「首次生成准入（对齐 isolate REQ-WSC-31）」还是 `start_generation`？「真实新生成」是否含 Failed 会话在同一 session 记录上的重臂？None 会话在什么事件上获得新默认？
3. **I-06**：fail-closed（未知 policy_version）与服务端 digest 核验是 delta 级契约还是实现级验收？（决策 §3.4 原文未要求 fail-closed。）
4. **I-07/I-08**：「窗口冻结输入含 effort 策略摘要」由谁落到 bound 的 28B/D1，以及共享文件串行顺序由哪个 change 的制品承载。
5. 决策 §3.5「真实调用另行授权」是否需要作为 delta MUST 条款（当前仅 proposal/design/tasks）。
6. **I-02**：是否接受「change 级 strict 通过、归档后 `validate --specs --strict` 失败」，还是压缩 MODIFIED 正文。

## 7. 未核实项

- 未运行任何 cargo 构建/测试（只读审查，未授权）；所有代码论断仅基于读源码；I-13 的「修订臂为非 SC 流」属 [INFERENCE]。
- 未核实 `openspec/config.yaml` 是否有 CI 强制 `validate --specs --strict`（仅查到参考归档报告用非 strict）。
- 其余 3 个 change 的 MODIFIED 块未逐场景重做（Main 已声明核过）；D 维度只读了 bound / isolate 的 design 共享面节与 close 的共享面标题。
- 未做真实 provider 调用（亦未授权）。
