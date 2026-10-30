# Tasks

- [x] 1. 契约：`openspec/changes/fix-write-scope-bound-repository/`（proposal/design/delta/tasks），`openspec validate fix-write-scope-bound-repository --strict` exit 0。
- [x] 2. TDD 红：新增 `work_item_plan_compiler/tests/write_scope_bound.rs` 五类用例（非成员区被拦 / 跨成员被拦 / 新建不误伤 / 前缀表缺席零回归 / 被授权仓不在表内 fail-closed + 无锚点不触发）；`PlanCandidateValidationContext` 扩 `member_checkout_prefixes` 字段并同步全部构造点——红已实录（bg_318/bg_320：字段缺失 E0560/E0432 编译红 + 存在性断言失败红）。
- [x] 3. TDD 绿：
  - [x] 3.1 `cross_cutting/worktree.rs` 新增 `scope_fixed_path_anchors`（首个 `*` 前固定段；顿号方言拆分；归一同源）+ 锚点提取单测。
  - [x] 3.2 新模块 `work_item_plan_compiler/write_scope_bound.rs`：逐 WI 归属核对 → `write_scope_outside_bound_repository` Error finding（每 WI 聚合一条、去重、声明序）。
  - [x] 3.3 `types.rs`：`WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION` 共享常量 + `PREFLIGHT_FINDING_CODES` 收录新码；`validate.rs` 接线（前缀表在场才触发）。
  - [x] 3.4 `plan_preflight.rs`：`preflight_required_action` 映射新码（与共享常量逐字同源）+ 同源断言测试。
- [x] 4. 生产装载与接线：`compile_support.rs` 新增 `plan_member_checkout_prefixes`（Legacy→None、FailClosed/越根/成员缺失/Main checkout 不唯一→Err、active 成员、strip_prefix+canonicalize 兜底）；三条校验路径（single_candidate.rs / revision_turn.inc.rs / web run single_candidate.rs）同源接入（与 validation_member_targets 同门控）。
- [x] 5. 门禁：`cargo test --locked --lib` 5696/0 绿；`--test it_web` 591/0 绿；clippy `-D warnings` 零告警；fmt 干净；`--test it_core` 189 通过 + large_file_guard 剩余超限 3 项全部在并行任务 A 写面（contract_prerevision.rs/prompts.rs/tests/single_candidate.rs，已通知 A 自行登记；我方 4 项登记已完成）。
  - **k3 审查后终态（2026-10-30）**：F1/F2/F3/F4 四缺陷全修完；`--lib` **5712/0**、`--test it_core` **190/0**（large_file_guard 绿）、`--test it_web` **591/0**、clippy 零告警、fmt 干净。**行数登记最终值（均已获 owner 批准，见下）**：`draft_batch/compile_support.rs` 2310（装载器整体迁出至新模块 `draft_batch/member_checkout_prefixes.rs`，134 行，+0）、`web ws_handler/run/single_candidate.rs` 1658→1666（+8，三调用点收敛为共享门控 `for_validation`）、`conversational_gate_recovery.rs` 1565→1567（+2，结构体字面量必增）、`conversational_gate_parts/revision_turn.inc.rs` 1488→1505（+17，24 层缩进下物理下限：已合并进既有 enrollment match、短路径 use、短变量，rustfmt 拆行不可再压）、`work_item_plan_compiler/tests.rs` 1449→1450（+1，新测试模块声明必增）。
  - **owner 裁决留痕（2026-10-30）**：上述 **+17 / +2 / +1 三条**已由 owner 明确批准（与并行任务 A 的 3 条一并裁决，A 那 3 条为 `contract_prerevision.rs` 1911→1913、`prompts.rs` 1665→1681、`tests/single_candidate.rs` 1545→1551）。登记值 = 各自当前行数，仍锁定不放松「不得继续增长」。结构性问题见 `cadence/notes/2026-10-30_待办_行数上限治理_v1.0.md`（登记 109 个文件、**89 个已顶格**、0 超限；建议改为目录级上限 + 分批拆最大的）。
- [x] 6. 报告：裁定表、接线链路、冲突排查见 design.md §3/§4 与最终报告。
