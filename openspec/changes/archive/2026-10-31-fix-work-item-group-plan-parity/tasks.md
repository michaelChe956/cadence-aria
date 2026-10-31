# Tasks: fix-work-item-group-plan-parity

> 本 change 只含 2026-07-31 本轮**已兑现**的部分（A 路线 + A+ + owner 面）。延后部分（路线 B / 平台派生四键 / 仓序边平台持有 / prompt 瘦身 / 现场回放用例）留在 `unify-work-item-group-structure`，经 owner 2026-10-31 裁定「复杂内容先不做」。

## 1. 归属反查单一来源（反查三态 + 三处实现收敛）

- [x] 1.1 在 `src/product/workspace_engine/member_attribution.rs` 新增 `MemberReverse`（`Unique(LogicalRepositoryId) | Absent | Ambiguous`）与 `reverse_member_logical(physical, &physical_by_logical)`，注释标明「唯一来源」。
- [x] 1.1b 本树里同一个反查原有**三份**实现，第三处 `compile/ir_adapter.rs` 的 `logical_target_for_ir_item` 也改为调用 1.1（经 A3 同意后改，两条错误文案逐字保留，含 Ambiguous 分支不带稳定码前缀的现状）。
- [x] 1.2 `src/product/workspace_engine/compile/finalizer.rs` 的本地 `reverse_logical` 闭包改为调用 1.1（`:929-939`）；`Absent` / `Ambiguous` 复用现有 `work_item_target_missing` 文案（逐字不变，由新增用例逐字断言）。
- [x] 1.3 单测：`src/product/workspace_engine/tests/part_03/park1_work_item_records.rs` 既有三分支用例全绿；新增 `attribution_ambiguous_physical_member_fails_closed_with_existing_wording` 断言 `Ambiguous` 报错文案与现状逐字一致。

## 2. 候选门与 Final Compile 判定一致（parity 测试）

- [x] 2.1 `single_repository` 声明物理仓 → `ReverseMember::Unique` 时 `logical_target = Some`；`Absent` / `Ambiguous` 保持 `None`（不猜、不新增失败种类）。集合 intent 分支语义原样保留。
- [x] 2.2 调用点同步：**无需改动**（签名未变；`plan_projection.rs` 以同名 re-export 承接）。
- [x] 2.3 保持 `plan_projection.rs` 的 legacy 清空块不动（成员表为空时零变化）。
- [x] 2.4 parity 用例（先红后绿）：`src/product/workspace_engine/tests/single_candidate/structure_parity.rs`（经 `include!` 内联）——同一 fixture 真跑候选门与终编译 `prepare_initial_plan_compile`，断言有效边集合逐条一致、成环结论一致、witness 首跳与链延续逐字一致。
- [x] 2.5 补充：**未声明 Plan Intent** 的 WI 也按同一反查从 IR 已定的回落物理仓解析归属（否则这一类仍在候选门落 `(None, None)`，REQ-WG-01 的「两处结论一致」不成立）。

## 3. owner 面：可执行选项而非归因断言

- [x] 3.1 `src/product/workspace_engine/compile_failure_surface.rs` `owner_hint`：删除「根因指向 design / 需要 owner 修改 design」断言；witness 完整时渲染两跳来源 + 两个仓序位次 + 两个选项及后果。（A3：`owner_hint` 改收 `&CandidateCompileFailure`，witness 事实由 `cycle_witness_facts` 从 producer 固定模板 fail-closed 解析。）
- [x] 3.2 `rewrite_guidance` 删除「根因可能在 design」条目；`author_teaching_block` 改为携带算出的位次与选项 + 保护项。（A3：两者均改 witness-aware；witness 不完整时退化为无归因通用要点。）
- [x] 3.3 单测：witness 完整 → 不含「修改 design」、含两个位次与两个选项；witness 不完整 → 不含任何位次/归因断言。（A3：`tests/candidate_compile_failure_surface.rs` 与 `compile_failure_surface.rs` 内嵌 tests。）

## 5. prompt 取值来源按会话类型唯一（A+）

- [x] 5.0 **A+（0.3 人日，可先单独发）**：删掉 `sc_author_teaching.rs:41` 与 `:253` 互斥取值来源中的**一句**（保留另一句并标注唯一来源），消除形态一在 prompt 侧的唯一成因；不依赖其他改动。
- [x] 5.4 既有 prompt 契约测试同步（`tests/prompt_contract*.rs` 中引用被删条款的断言）。

## 4. 观测（跨 change）

- [x] 4.1 尝试流水与通过率读数在 `openspec/changes/mechanicalize-plan-compile-repair/` 的 3.1 / 3.2 落地（追加写 `candidate-attempts.jsonl`；读数：首次通过率 / 同 issue 连续失败次数 P50·P90 / 失败码 Top N / lowering 类终态失败计数）。

## 6. 未做（owner 裁定延后，见 `unify-work-item-group-structure`）

- [ ] 6.1 Plan Intent 四键由平台派生（需成员 checkout 前缀表进 lowering 上下文）→ 路线 B
- [ ] 6.2 仓序边平台持有 / Inputs 方向冲突检出 → 路线 B
- [ ] 6.3 平台接管后移除 prompt 结构契约 → 依赖 6.1
- [ ] 6.4 固定教学块字符数 / 硬性措辞数的**全量**计量锁（本次只对 Plan Intent 归属块做了计量锁）
- [ ] 6.5 清理失真注释 / 抽仓序边纯函数 / 与 `mechanicalize-plan-compile-repair` 的 `REQ-WSC-23/24/25` 去重
- [ ] 6.6 现场回放用例（`e5f6b290` 形态：review 连报三轮 + `validation_reject`）与自动重写在多仓 plan 上的收敛率观测
