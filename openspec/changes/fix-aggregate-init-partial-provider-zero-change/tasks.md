# Tasks：聚合初始化零变更判据与失败诊断

## 工作包 1：provider turn 输出保留头尾

- [ ] Task 1.1：`BoundedOutput::push` 超预算时裁中段、保留头尾（字符边界安全）+ 新增
  `floor_char_boundary` / `ceil_char_boundary`
  （`src/product/logical_codebase/coordinator_provider_turn.inc.rs`）
- [ ] Task 1.2：单测 `bounded_output_keeps_head_and_tail_after_budget`（同文件测试模块）

## 工作包 2：零变更判据补「命令是否真的执行且成功」

- [ ] Task 2.1：`aggregate_initialization_coordinator.rs` 新增 `RecipeCommandReport`
  （Absent / Success / Failed）与 `AggregateProviderTurnOutcome`；trait 返回类型改为 outcome
- [ ] Task 2.2：`consume_turn` 只从**工具结果原文**读脚本报告（`recipe_report_verdict` +
  `merge_recipe_report`，失败优先）
- [ ] Task 2.3：`root_recipe_audit_windows.inc.rs`：零变更且报告成功 ⇒ 判成功；否则失败
- [ ] Task 2.4：全部 trait 实现（生产 2 处 + 测试替身 8 处）随返回类型机械更新
- [ ] Task 2.5：测试——零变更失败诊断携带 provider 输出；零变更 + 报告成功 ⇒ step completed

## 工作包 3：失败诊断携带 provider 真实输出

- [ ] Task 3.1：`coordinator_frozen_provider.inc.rs` 新增 `provider_turn_failure_output`
  （失败事实 + 输出尾部，脱敏走既有入口）
- [ ] Task 3.2：`production_dependencies.inc.rs` 把 turn summary 传入审计收口

## 工作包 4：验证与归档

- [ ] Task 4.1：`cargo fmt --check`
- [ ] Task 4.2：`cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] Task 4.3：`cargo check --locked`
- [ ] Task 4.4：`cargo test --locked`
- [ ] Task 4.5：podman 容器 E2E 重跑（只装 Pi 的场景）——聚合初始化成功
- [ ] Task 4.6：OpenSpec sync / archive
