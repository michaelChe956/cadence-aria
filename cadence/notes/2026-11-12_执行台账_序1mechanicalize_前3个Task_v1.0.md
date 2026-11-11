# SDD ledger — plan: /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-wig-optimize-11-08/cadence/plans/2026-11-11_计划文档_实施计划_WIG编译机械化与修复面_v1.0.md

- 分支：`feat-wig-optimize-11-08`（worktree `.worktrees/feat-wig-optimize-11-08`）
- 串行序：**序 1**（序 0 决策六 ✅ 已合入 main → **序 1 本 change** → 序 2 quota-pool → 序 3 bound → 序 4 effort → 序 5 isolate → 序 6 close → 序 7 rework）
- 对应 OpenSpec change：`mechanicalize-plan-compile-repair`（已用 `openspec list` 确认：11 个活跃 change，本 Plan 落此 change，**不落错**）
- 交接来源：`cadence/notes/2026-11-11_交接_另一会话_WIG优化实施前置_v1.0.md`
- 开工基线（2026-11-11 实测）：
  - `cargo clippy --all-targets --all-features --locked -- -D warnings` → **exit 0**
  - `large_file_guard`（2 项用例）→ **2 passed / 0 failed**
  - Plan 的 9 个行数锚点 + 5 个符号锚点 → **全部仍准确**（main 前进后未漂移）
- 开工动作：`git merge origin/main` → `09e34506`（origin/main 已成为 HEAD 祖先）

---

## Pre-flight 冲突扫描（dispatch Task 1 之前）

### A. 共享文件 / 共享接口的任务对

| 任务对 | 一方产出 | 另一方消费 | 扫描结论 |
|---|---|---|---|
| Task 1 ↔ Task 2 | Task 1：`types.rs` 追加 `lowering_fault: Option<LoweringFault>` + `LoweringFault` 枚举；`lower.rs` 产 typed fault | Task 2：`types.rs` 追加 `ir_fault: Option<IrFault>` + 新模块 `ir_fault.rs` | **共享 `types.rs` 与 `mod.rs`** ⇒ 严格串行；Task 2 MUST 在 Task 1 之上追加**并列**字段，不得改动 `lowering_fault` 的形态 |
| Task 2 ↔ Task 3 | Task 2：四类 typed IR 载荷 + producer（`validation.rs`） | Task 3：`contract_autorepair.rs` 移除八轮 source mutation 收敛 | **无共享文件**；接口关系 = Task 3 产机器指令交原授权 author，Task 2 的 typed 载荷是它的下游消费者 |
| Task 3 ↔ Task 4 | Task 3：改 `run/single_candidate.rs` 的 converge 调用点（符号定位）+ `workspace_engine/single_candidate.rs` 的 completion 调用点 | Task 4：改 `run/single_candidate.rs` 主循环 + `run/single_candidate_compile_preflight.rs` + `prompts/sc_compile_teaching.rs` | ⚠️ **共享 `run/single_candidate.rs`** ⇒ 严格串行；Task 4 MUST NOT 覆盖 Task 3 对该文件的改动（交接纪律第 2 条：改完当场 `git diff --stat` 复核） |
| Task 4 ↔ Task 5 | Task 4：handler 侧 typed 资格判定 | Task 5：`workspace_engine/types.rs`（新增错误枚举）、`compile.rs`、`compile/single_candidate.rs`、`draft_batch/compile_support.rs` | **无共享文件** |
| Task 5 ↔ Task 6 | Task 5：Final Compile 环失败透传 Structure | Task 6：一次答完收口核验（核验型） | **无共享文件**；Task 6 依赖 Task 5 的透传面才能做「已交付面」核验 |
| Task 6 ↔ Task 7 | Task 6：创建测试 + 勾 `tasks.md` | Task 7：`sc_author_teaching.rs` + `prompt_contract*.rs` + 新计量测试 | **无共享代码文件**；但 **两者都写 `tasks.md`** ⇒ 串行执行，各只勾自己的工作包 |
| Task 7 ↔ Task 8 | Task 7：prompt 瘦身 + 计量闸门 | Task 8：验收记录 + 取证记录 + 勾 `tasks.md` | 同上（共享 `tasks.md`），串行 |
| 全部 Task | 多个 Task 提交时都 `git add ... openspec/changes/mechanicalize-plan-compile-repair/tasks.md` | — | `tasks.md` **只由当前 Task 勾选自己的条目** ⇒ 串行执行下无冲突；**禁止 `git add -A`**（会把并发会话的 `runner.rs` 卷进去 —— 交接纪律第 1 条的真事故） |

### B. 每个任务的自洽性

| 任务 | 自洽性核验 | 结论 |
|---|---|---|
| Task 1 | 产出签名（`LoweringFault` / `EquivResolution` / `compile_work_item_plan_audited`）与 Step 1 的五个用例名一一对应；`EquivalenceCanonicalization` 复用既有 `lifecycle_store/candidate_attempts.rs:126`（已核实存在）；`diagnostic()` / `lower_plan_intent` / `WorkItemPlanSourceContext` / `PlanMemberTargets` 五个符号锚点**实测仍在 :1121 / :400 / :33 / :47**；`types.rs` 190、`lower.rs` 1132 余量充足；`tests.rs` 登记 1450 ⇒ 新测试落新文件 | 自洽 |
| Task 2 | 新模块 `ir_fault.rs` + `ir_fault_tests.rs`，`mod.rs` 加一行；与 Task 1 共享 `types.rs`/`mod.rs` ⇒ 串行 | 自洽 |
| Task 3 | 改 `contract_autorepair.rs`（**1651 = 登记上限，零余量**）且要求「移除八轮 source mutation 收敛与死代码」⇒ **净减**方向与零余量约束一致；`contract_prerevision.rs` 登记 1913；两个 caller 都点名 ⇒ 覆盖 tasks 8.3 的坑 | 自洽 |
| Task 4 | 改 `single_candidate_compile_preflight.rs`（541，决策六拆出）、`single_candidate.rs`（1146）、`prompts/sc_compile_teaching.rs`（140）；新建 `typed_teaching.rs` | 自洽 |
| Task 5 | 改 `workspace_engine/types.rs`（**1146，余量仅 54 行**）、`compile.rs`（**1651 零余量**）、`compile/single_candidate.rs`（1165）、`draft_batch/compile_support.rs`（**2308，只许净减**） | 自洽，但**余量极紧** ⇒ 见裁定 R3 |
| Task 6 | **核验型**（跑既有测试 + 补缺失负向/透传项 + 勾选 tasks.md），不产生新行为 | 自洽；**TDD 红-绿不适用**（见裁定 R4） |
| Task 7 | 改 `sc_author_teaching.rs` + 新建 `prompt_contract_volume_budget.rs` + 更新受影响的 `prompt_contract*.rs`；计量基线 15,765 → ≤8K | 自洽；红测 = 体积超限（可红） |
| Task 8 | 建两份 `cadence/notes/` + 勾 tasks.md 9.1/9.2/10.1/10.2 | 自洽 |

### C. 与评审口径 / 交接纪律的冲突

| # | 冲突 | 裁定 |
|---|---|---|
| **R1** | Plan 原写「clippy 基线 9 个 error，验收口径零新增」——**该前提已失效**（并发会话已修 9 处并合入 main，实测 exit 0） | **裁定**：验收口径改为 **0 告警**（交接纪律第 4 条），不得引入任何新增。已改 Plan。代价：若沿用「零新增」，会放过本批自己引入的新告警 |
| **R2** | Plan 原写「序 0 决策六拆分进行中，`run/single_candidate.rs` 仍 1674，守卫现红」——**已失效**（已拆至 1146、登记项已销、守卫 2 passed） | **裁定**：序 0 硬门禁**已满足**，Plan 该段改写为已完成状态。已改 Plan。代价：若不改，实施者会误以为还要等 |
| **R3** | Task 5 要在 `workspace_engine/types.rs`（余量仅 54 行）新增错误枚举，而该文件离 1200 只差 54 行，且 `compile.rs`（1651 零余量）/ `draft_batch/compile_support.rs`（2308 只许净减）同时在改 | **裁定**：Task 5 的新增错误枚举**若使 `types.rs` 超 1200 ⇒ 改落新模块**（仓内惯例 `workspace_engine/compile/` 子模块），不得抬登记上限。`compile.rs` 与 `compile_support.rs` **只许净减或持平**。代价：若不预先钉死，实施者会在零余量文件里硬塞 → 守卫转红 |
| **R4** | Task 6 是**核验型**（跑既有测试、补缺失负向项），不产生新行为 ⇒ TDD 的「先红」无对应对象 | **裁定**：Task 6 的红态定义 =「缺失的负向/透传断言在既有实现上确实失败」；若既有实现已满足，则该项**登记为已交付并核验通过**，**不得**为制造红态而改产品。代价：若机械套用 TDD 模板，会去改已正确的产品代码 |
| **R5** | 交接纪律第 2 条「改完当场 `git diff --stat` 复核」与第 5 条「跨平台必须编得过（别用 Linux 专属 libc）」在 Plan 里**缺失** | **裁定**：两条已补进 Plan 的 Global Constraints（逐字引用事故原型）。代价：Plan 是实施者读的契约，缺纪律＝实施者不会照做 |
| **R6** | 交接纪律第 6 条「起服务走 `scripts/aria-web-start.sh`」 | **裁定**：序 1 不起服务；仅 Task 8 若需真机取证时，**必须**走 `install -m755 scripts/aria-web-start.sh /tmp/aria-deploy/start.sh`（手搓 `aria web --port 4317` 直起 ⇒ `ARIA_API_TRUSTED_ORIGINS` 缺 hostname 与两个反代域名 ⇒ owner 访问报 `[request origin not allowed]`） |
| **R7** | 交接纪律第 8 条「开工前确认 Plan 落到哪个 change」 | **裁定**：已用 `openspec list` 确认 —— 本 Plan 落 `mechanicalize-plan-compile-repair`（**不是**同名的 `teach-sc-compile-reredrive-feedback`，后者是我另建的、落 `work-item-plan-single-candidate` 的新 change，序 3.5）。二者名字相近易混，已在 ledger 锁定 |
| **R8** | cargo 锁用**全局** `/tmp/aria-cargo.lock` ⇒ 实测把本链的验证**阻塞在另一个 worktree 的 cargo 上**（另一会话 `.worktrees/feat-pic-provider-11-11` 持锁跑 `cargo test --lib cross_cutting::image_client`） | **裁定**：改为 **per-worktree 锁 `/tmp/aria-cargo-wig.lock`**。依据：两个 worktree 的 `target/` 目录 **inode 不同**（本 worktree `587177473`、另一 `601073392`）⇒ **不共享 cargo 的 target 目录锁，cargo 内部本无竞争** ⇒ 全局 flock 属跨 worktree 的无谓串行化。同 worktree 内仍用它串行化（那里才有真冲突）。**不 kill 别的会话的 cargo 进程**。已改 Plan 的 Global Constraints 并通知在跑的 Task 3 实施者 |

---

### Task 1: lowering typed fault 基建 + Plan Intent 分支等价解析 —— 已完成（评审 Approved）

- **提交**：`d6a75fd9` —— `feat(compiler): lowering 诊断携带独立 typed fault 并落地 Plan Intent 分支等价解析`（11 文件，+539/−22；索引经 `git diff --cached --name-only` 核验仅含任务文件）
- **TDD 红态**：确认为「`LoweringFault` / `compile_work_item_plan_audited` 未定义」（**E0432/E0609，非语法错**）⇒ 红态合法。绿态 `equivalence` 5 passed / 0 failed。
- **回归**：`work_item_plan_compiler` 125 passed（基线 120，只增不减）；`contract_autorepair` 21 passed（基线 21，断言集合不变）；clippy `-D warnings` 全仓 **exit 0**；`cargo fmt --check` **exit 0**。
- **行数（controller + 评审双独立 `wc -l` 核验，数值一致）**：`lower.rs` **1199**、`types.rs` 225、`tests.rs` **1450（净零）**、`contract_autorepair.rs` **1650（净减 1）**、`equivalence.rs` 136、`equivalence_tests.rs` 271。
- **评审**：**Approved** —— 0 Critical / 0 Important / 2 Minor。评审逐字段核验了 Produces 契约（`LoweringFault` 两变体全部字段 / `CompilerDiagnostic.lowering_fault` / `EquivResolution` 三变体 / `resolve_equivalent_single_repository_target` 签名 / `CompiledPlanAudit` 字段 / `compile_work_item_plan_audited` 签名与返回类型）**逐字命中**；核验 9 处外部构造点全部在 diff 内（结构体字面量必须穷尽字段 ⇒ 编译通过即机械佐证无遗漏）；核验关键路由 `(Some,false)→Adopted / (Some,true)→Ambiguous / (None,_)→NotInMemberTable`、legacy 会话回落与聚合前缀 typed 拒绝、fault 仅两构造点显式写入（**零 code/message 派生**）、**source 无任何改写路径**、`compile_work_item_plan` 旧签名不变纯委托。
- **两处零余量压缩经评审逐行核验为纯构造去重、断言强度未削弱**：`contract_autorepair.rs:1486-1494` 删的是 fixture 构造值（`line: 9` / `field: "trusted_commands"` / `kind: None`），`..unknown_key_diagnostic(9, "trusted_commands")` 逐字段回填相同值，三条断言一行未动；`tests.rs:882-884` 把 `let cloned = d.clone(); assert_eq!(d, cloned)` 改为 `assert_eq!(d, d.clone())`，仍构造 clone、仍全字段 `PartialEq` 全等（含新字段）。
- **Plan 缺陷更正（我方）**：Plan 原写「`CompilerDiagnostic` 字面量构造点（含 `resolve_item_target_repository_id` 内的构造）逐一补」但只列了 `lower.rs`；实际 **9 处在 lower.rs 之外**（`parse.rs`×2、`validation.rs`×3、`tests.rs`×1、**`tests/blockers.rs`×1**（实施者发现、Plan 原漏）**、`contract_autorepair.rs`×2**）。⇒ 已更正 Plan 的 Files 与 Step 3②（9 处 + `tests/blockers.rs` + 两个零余量文件的净零要求）。
- **落点变更（已更正 Plan）**：`lower.rs` 加全部实现会到 ~1206 行**超 1200 红线** ⇒ 按 Global Constraints「新增代码一律落新文件」改落 `equivalence.rs`（136 行），`lower.rs` 只留接线（1199）。命名与签名逐字符合 Produces（评审已核）。
- **两条 Minor（已派回实施者当场修）**：① `EquivResolution` / `resolve_equivalent_single_repository_target` 的 `pub(super)` **超出 Produces 契约**（契约未标 pub）且无外部调用方 ⇒ 暴露裸 resolver 等于邀请未来 Task 绕过 wrapper 直接解析、**静默丢审计** ⇒ 改私有。② `lower.rs` 余量仅 1 行是**后续 Task 的悬崖**（Task 2 加 `ir_fault` 字段 ⇒ 1199+2 = 1201 即超线）⇒ 现在就把 `diagnostic_with_fault`（连文档注释约 14 行）下沉到 `equivalence.rs`、把 `diagnostic()` 提为 `pub(super)`，`lower.rs` 降至 ~1185 ⇒ **当下解除悬崖**，Task 2/3/4 都不必再为腾行做取舍。

Task 1: complete (commits a4757008..d6a75fd9, review clean — 2 Minor 派回当场修)

**修正波**（commit `438d68ba`，`refactor(compiler): 收紧等价解析可见性并下沉诊断工厂以腾出行数余量`，2 文件 +25/−22，索引核验仅 `equivalence.rs` + `lower.rs`）：
- **Minor 1（可见性超契约）已消除**：`EquivResolution`（现模块私有，equivalence.rs:13）与 `resolve_equivalent_single_repository_target`（现模块私有，:27）的 `pub(super)` 均已去掉，diff 确认**仅去掉可见性修饰、签名与逻辑零改动**。评审做了**全仓 grep 引用面核验**：两个符号的全部命中均在 `equivalence.rs` 内（唯二调用仍在 wrapper `equivalent_single_repository_target`（:83，`pub(super)` 按契约保留、审计 push 未动））⇒ **无任何 sibling 模块可达裸 resolver，绕过审计的通路已封死**。
- **Minor 2（行数悬崖）已解除**：`diagnostic_with_fault`（**含两行 REQ-CRP-02 文档注释**、签名、struct-update 函数体）**逐字节**迁至 `equivalence.rs:62-75`；`diagnostic()` 提为 `pub(super)`（`lower.rs:1172`，仅父模块树可见，非 crate 公开 ⇒ 最小扩面）。消费侧接线完整：`lower.rs:5` 的 `use super::equivalence::{diagnostic_with_fault, equivalent_single_repository_target}`；全仓 grep `diagnostic_with_fault` 仅 4 处（定义 :64 / 调用 :107 / 导入 lower.rs:5 / 调用 lower.rs:446），**无死代码、无悬空引用**；`use super::types::LoweringFault` 仍活跃。
- **行数**：`lower.rs` 1199 → **1189**（余量 **11 行** ⇒ **Task 2 加 `ir_fault` 的 +2 行悬崖已解除**）；`equivalence.rs` 136 → **149**。
- **复评结论：All findings addressed, no new Critical/Important breakage**（核查手段：diff 逐字比对 + 全仓 grep 两符号引用面 + `equivalence.rs` 全读 + `LoweringFault` 导入活跃性 + `wc -l`；未重跑测试，按指令）。
- **前向状态**：Task 2 开工时 `lower.rs` 余量 11 行，**无需再为腾行做取舍**。

Task 1: complete (commits a4757008..438d68ba, review clean + 修正波复评全绿)

### Task 2: 四类 typed IR 诊断载荷（ir_fault.rs） —— 已完成（评审 Approved）

- **提交**：`46898222` —— `feat(compiler): 四类 IR 诊断携带 typed 载荷（ir_fault）且行号经 AST 补全`（16 文件，+702/−17；tasks.md 仅勾 2.6）
- **TDD 红态**：11 个错误**全为 E0432/E0609**（「IrFault / ir_fault 未定义」）⇒ 红态合法。绿态 `--lib ir_fault` 7 passed；`--lib work_item_contract` 79（=基线）；`--lib work_item_plan_compiler` 132（=基线 125 + 7 新增，**双向吻合**）；`contract_autorepair` 21；fmt exit 0；clippy `-D warnings` exit 0；行数守卫 **2 passed**。
- **行数（controller + 评审双核验）**：`ir_fault.rs` 172、`ir_fault_tests.rs` 366、`types.rs` 230、`lower.rs` **1191**（≤1200）、`tests.rs` **1450 净零**、`contract_autorepair.rs` **1646 净减**、`validation.rs` 699（620→699）。
- **评审**：**Approved** —— 0 Critical / 0 Important / 3 Minor。四项重点裁决**均经 diff 外独立取证成立**：
  ① **可见性偏差成立**：两个宿主结构（`CompilerDiagnostic` / `ContractValidationFinding`）经 `pub use types::*` / `pub use validation::*` 全公开可达 ⇒ `pub(crate)` 字段类型必然触发 `private_interfaces`（`-D warnings` 下失败）⇒ 私有 `mod ir_fault` + `pub` 类型 + `pub use` 重导出是合规最小形，且与 Task 1 的 `LoweringFault` 同形先例。
  ② **10 处机械补齐零断言削弱**：`tests.rs` 的 `let cloned_ast = ast.clone(); assert_eq!(ast, cloned_ast);` → `assert_eq!(ast, ast.clone());`（clone 等值断言保留、语义逐字等价）；`contract_autorepair.rs` 的内联 `WorkItemPlanSourceContext{…}` 换成 helper `source_context()`（评审读到 helper 定义逐字段相同、无断言被触碰）。
  ③ **负向换码成立**：`overlapping_exclusive_and_forbidden_scope` 的 producer 就在 `validate_canonical_contract` 内、经 `error_finding` 构造（validation.rs:409-413，Error 级 ⇒ 真实产出 `ContractValidationFinding`），且不在 `PREFLIGHT_FINDING_CODES` ⇒ 走 Err 出口；新用例断言 `assert_eq!(overlap.ir_fault, None)`（ir_fault_tests.rs:316）**且**全体诊断 `ir_fault.is_none()`。`duplicate_task_id` 作为码在全仓不存在、重复标识符在 parse.rs:484 被 grammar 层终态拒绝 ⇒ 实施者论断属实。
  ④ **载荷-message 无关成立且强于验收形**：`scrub_message`（ir_fault_tests.rs:101-106）先抹编号再断言，**四个** payload 用例全做了（不止 brief 要求的一条）；行号确证来自 AST span（`attach_source_lines` → `parse_work_item_plan(source)` → `WorkItemPlanFieldAst.value.line`），`ir_fault.rs` 全文**无任何 message 字符串操作**；评审实读 fixture 第 35/36/43/58 行与用例断言的 `Some(35/36/43/58)` 逐一吻合；`None` **不以 0 冒充**（producer 恒 `None`，四个 payload 用例均断言 `source_line == None`）。
- **评审额外发现 brief 未料到的关键集成点**（已核实）：`WorkItemSplitFinding` **无侧信道** ⇒ brief 的「透传」不可行 ⇒ 实施者建了**重建桥**（collect 侧 validate.rs:545-565 与投影侧 work_item_split_validator/draft.rs:13-18 + utils.rs:3-8 跑**同一纯函数于同一对象** ⇒ 键/多重性/序列结构性相等，无静默丢弃路径；四码均 Error 且非 preflight ⇒ 必达 Err 出口）。评审逐行核验该论证成立。
- **Plan 缺陷更正（我方，两处）**：① **serde 契约矛盾**（原写「IrFault 不新增 serde 派生」却要加进 serde 的 `ContractValidationFinding`）⇒ 核实 `ContractValidationReport` 内嵌于 `models/work_item_revision.rs:150`（**持久化模型**，Task 4 要求审计落库）⇒ 裁定 `IrFault`/`IrFaultKind`/`IrFaultLocator` **必须加 serde 派生**，`IrFaultKind` 用 `#[serde(tag="kind", rename_all="snake_case")]`（与既有 `PlanningFailureKind`（planning_failure.rs:27-29）同形），findings 侧 `default + skip_serializing_if` ⇒ 旧记录再序列化字节不变。② **文件清单漏 8 个文件**（10 处字面量构造点分布在 8 个文件，与 Task 1 同型）⇒ 已更正 Plan。
- **三条 Minor 的处置**：① **公开面扩大**（`attach_source_lines` 被 `pub use` 带成 crate 外 API）⇒ **不现在改**（Task 4 未接线前收回会立刻 `dead_code`）⇒ **已写入 Plan 的 Task 4 前置段作为验收项**（Task 4 接线后 MUST 收回为 `pub(crate)`）。② `validate.rs:555` 的 `unwrap_or_default()` 掩盖「logical id 恒在场」不变量 ⇒ 派回实施者**加注释钉死**（**不改行为**，不改成 `.expect()`）。③ 报告「净减 2」算术不精确 ⇒ 派回实施者更正。

Task 2: complete (commits f624f8db..46898222, review clean — 3 Minor：1 条移交 Task 4 验收项，2 条派回当场修)

### Task 3: O1 落地——内容 autorepair 迁原授权 author（converge 重塑） —— 已完成（评审 Approved）

- **提交**：`fbb14cec` —— `refactor(workspace-engine): O1 落地——contract autorepair 本地 source mutation 迁移为原授权 author 指令`（11 文件，+1567/−1193）
- **TDD 红态**：`ContractConvergeOutcome` 未定义 + 旧三元组返回不匹配（**非语法错**）⇒ 红态合法。绿态 `contract_autorepair_migration` 25/0（新）、`contract_autorepair` 29/0（基线 21）、`single_candidate` 207/0、`workspace_engine` 1867/0（基线 1859）、`work_item_plan_compiler` 132/0、`it_core` 221/0；clippy exit 0、fmt exit 0。
- **行数（controller + 评审双核验）**：`contract_autorepair.rs` 1646 → **881（净减 765）**、新测试 `contract_autorepair_migration.rs` 1081、`run/single_candidate.rs` 1196、`contract_prerevision.rs` 1871/1913、`workspace_engine/single_candidate.rs` 808。
- **评审**：**Approved** —— 0 Critical / 0 Important / 3 Minor。评审**逐断言**核对了 312 行旧测试改写（`tests/single_candidate/contract_autorepair.rs` 180 / `preflight_options_loop.rs` 12 / `single_candidate_provider_run.rs` 75 / `reredrive_and_reopen.rs` 45）⇒ 结论：**全部为等强度口径反转**（每条旧断言都有反向新断言钉住；含逐字节 source 比对与终态零持久化；**零用例删除、零断言弱化**）⇒ 核心风险「旧测试改写是否掩盖问题」**已排除**。
- 评审另独立核验：Produces 契约**逐字**落地（`ContractConvergeOutcome{ir,instructions,audit}` + `converge_work_item_plan_source`）；**两条 caller 均切换且无第三点**（grep 全仓生产调用仅 `workspace_engine/single_candidate.rs:473` 与 `run/single_candidate.rs:833`）；**死代码删净**（`apply_structural_repair_operations`/`rewrite_source`/`MAX_CONTRACT_AUTOREPAIR_ROUNDS` 仅存于注释、零符号残留、无 shim）；四类收窄口径与裁决 §2.2 **逐条对应**；`StructuralRepairInstruction`/`ProtectedBusinessFacts` **复用未新造**；sg_07 两测试原地保留的取舍**成立**（断言与 mutation 语义正交，迁出需改 `finding_projection.inc.rs` 私有 fn 可见性 ⇒ 真越清单）。
- **三条 Minor 的处置**：① `tasks.md` 8.4 勾选行**措辞与事实不符**（称新文件承接了 sg_07 两测试的改写，实际那两测试原地未改）⇒ 派回实施者**只改措辞**。② `run/single_candidate.rs` 1196/1200 的**更低成本腾行**（指令事件块 `:935-962` ~28 行 + Err 臂拼装 `:841-851` 可下沉到 `single_candidate_compile_preflight.rs`）⇒ 派回实施者**纯搬移下沉** ⇒ 降至 ≤1170 ⇒ **Task 4 的「4 行余量悬崖」提前解除**。③ 指令全文进入 owner 可见消息**长度无界** ⇒ **已写入 Plan 的 Task 4 作为接线时的长度约束**（MUST 加条数上限或摘要形态 + 断言）。

Task 3: complete (commits 970a3105..fbb14cec, review clean — 2 Minor 派回当场修，1 Minor 移交 Task 4 验收项)



---

## 任务清单

- [x] Task 1: lowering typed fault 基建 + Plan Intent 分支等价解析
- [ ] Task 2: 四类 typed IR 诊断载荷（ir_fault.rs）
- [ ] Task 3: O1 落地——内容 autorepair 迁原授权 author
- [ ] Task 4: 教学资格 typed 合入 handler + 等价审计落库
- [ ] Task 5: Final Compile 环失败透传 Structure + 奇偶性与环面核验收口
- [ ] Task 6: 一次答完收口核验
- [ ] Task 7: prompt 分层瘦身与计量闸门
- [ ] Task 8: 收口验收与待验证假设取证

## 执行记录

（每条 Task 完成时追加：`Task <N>: complete (commits <base7>..<head7>, review clean)`）
