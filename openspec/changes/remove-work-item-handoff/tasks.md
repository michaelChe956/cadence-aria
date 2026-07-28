## 1. 移除后行为回归测试

- [ ] 1.1 为 unit 完成编写测试，断言不产生交接摘要产物、不发生用于生成摘要的 provider 调用。
- [ ] 1.2 为 unit 完成编写测试，断言不因交接摘要缺失而失败或降级。
- [ ] 1.3 为下游依赖解析编写测试，断言仍获得上游 `provided_contracts` 与 `provided_capabilities`，内容与移除前一致。
- [ ] 1.4 为运行时权威校验编写回归测试，断言 commit / revision / status 不一致时仍失败关闭，判定口径不变。
- [ ] 1.5 为交接发布编写测试，断言 `HandoffRevision` 只承载契约与能力语义，不含测试与产物清单。
- [ ] 1.6 为启动 coding 编写测试，断言上游无交接摘要引用时不再被拒绝。
- [ ] 1.7 为 schema v2 契约编译编写回归测试，断言 `handoff_contract` 与 `handoff_field` 仍可用、编译产出契约与能力不变。
- [ ] 1.8 为组完成门禁编写写入范围越界测试：某已完成 unit 的实际变更命中 `forbidden_scopes` 时必须拒绝，且不依赖任何交接摘要字段构造输入。此测试替代 `tests/it_product/product_coding_workspace_engine/part_13.rs:569-605`（原测试靠覆写 `WorkItemHandoff.files_changed` 触发，模型移除后无法构造），MUST 先失败。
- [ ] 1.9 为组完成门禁编写合规放行测试：各 unit 变更均在 `exclusive_scopes` 内时门禁放行，防止改数据源后误拒。
- [ ] 1.10 为 reviewer 提示词编写断言：Code Review 与 GroupFinalReview 协议均不含「确认 handoff 承诺闭环」类要求、不点名已移除字段，且跨 unit 审查对象为 `HandoffRevision` 契约与能力语义。
- [ ] 1.11 为 reviewer 提示词编写回归断言：verdict 取值口径与除交接摘要外的否决依据未被削弱。
- [ ] 1.12 为 work item 完成 commit 编写回归测试，断言 `completion_commit` 仍被写入且可经接口读取。

## 2. 生产实现

- [ ] 2.1 移除 `WorkItemHandoff` 模型及其存取 API、路径解析与文件产物。
- [ ] 2.2 移除交接摘要生成入口、provider 调用与占位降级路径。
- [ ] 2.3 从 `HandoffRevision` 移除 `tests` 与 `artifacts` 字段，不添加历史数据兼容层。
- [ ] 2.4 调整 `build_group_handoff_revision` 不再读取交接摘要。
- [ ] 2.5 把组完成门禁（`gates.rs:281-305` 两条路径）的 changed_files 数据源改为 git 事实：优先按 unit completion commit 取 per-unit 清单；若无法保住 per-unit 粒度，退为 `changed_files_for_attempt` 并在实施说明中记录该退化。MUST NOT 保留空 changed_files 导致校验空转。
- [ ] 2.6 改写 `code_review_material_protocol` 与 `group_final_review_material_protocol` 中以交接摘要为审查对象的指令，切换为 `HandoffRevision` 的契约与能力语义；不改 verdict 取值口径与其他否决依据。
- [ ] 2.7 移除 lifecycle 层 `handoff_summary_ref`、`required_handoff_from`、`planned_handoff_summary`；`update_work_item_handoff_summary` MUST NOT 整体删除——摘掉 `handoff_summary_ref` 参数与赋值、保留 `completion_commit` 写入并按新职责重命名。
- [ ] 2.8 移除启动 coding 的 `work_item_handoff_missing` 前置校验。
- [ ] 2.9 移除 split engine 中 legacy 的 `required_handoff_from`、`max_handoff_chars`、`max_dependency_handoffs`（含 `models/lifecycle.rs`、`work_item_split_validator/plan.rs`、`web/types.rs`、`web/handlers/dto.rs`、`streaming_provider/fake.rs`、`web/src/api/types/common.ts`、`LifecycleCardDrawer.tsx` 的全部落点）；确认保留 `handoff_contract`、`handoff_field`、`handoff_notes`、`handoff_strategy`。
- [ ] 2.10 移除 WebSocket 协议字段与前端类型、状态对交接摘要的消费；前端 `WorkItemHandoff` 类型与后端不同构且无组件消费，仅需清理 store 引用。
- [ ] 2.11 处置 `WorkItemHandoffMissing`：移除 `group_completion.rs:101`、`gates.rs:241`、`reports.rs:51`、`handoffs.rs:244` 四个摘要触发点；为 `runtime_impact.rs:479`、`plan_defect.rs:474`、`plan_defect.rs:487`、`group.rs:20` 四个 `HandoffRevision` 体系触发点保留可用变体，其中 `group.rs:20` 换用语义正确的变体。`web/error.rs:97` 的 API 错误码字符串单独判断。
- [ ] 2.12 移除 `coding_evaluation_context` 的 `handoff_tests_run` / `handoff_test_result_summary`（`builder.rs:383-389`、`mod.rs:59-60`）；此项归本 change，`remove-testing-stage` 不重复处理。
- [ ] 2.13 处置 `CodingWorkspaceEngine.provider` 字段与 `with_provider` 构造器：唯一读取点是 `handoffs.rs:328`，移除摘要生成后必为死字段，一并移除并改 `web/coding_ws_handler/runner/task.rs` 的构造调用。
- [ ] 2.14 处置随字段移除失效的测试与夹具：`tests/runtime_handoff_delta.rs:35-36`（断言 `tests`/`artifacts` 变化不影响 delta，字段移除后失去意义）、`web/test_controls/plan_repair/seed.rs:406-407`、`web/test_controls/plan_repair/recovery.rs:542-543`（`test_controls` 不在 `#[cfg(test)]` 下，属正常编译目标）。
- [ ] 2.15 确认未改动 `HandoffRevision` 契约语义、运行时权威校验、group completion 完成判定与 commit 绑定；确认 `completion_commit` 写入路径完好；确认只消除 `handoffs.rs:360` 一个 `list_testing_reports` 消费者，其余五处未触碰。

## 3. 验证与交付

- [ ] 3.1 运行本 change 相关定向测试与 group completion、lineage 存储、runtime handoff、split engine 既有回归，并区分既有失败基线。
- [ ] 3.2 运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 与各测试目标（`--lib`、`it_core`、`it_web`、`it_provider`、`it_product`、`it_task_run`）。
- [ ] 3.3 运行前端检查与测试：`cd web && pnpm tsc -b`、`cd web && pnpm test`。
- [ ] 3.4 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.5 经用户确认后重启后端，由用户验证 group final review 不再因交接摘要缺失判要求修改。
