## 1. 移除后行为回归测试

- [ ] 1.1 为 unit 完成编写测试，断言不产生交接摘要产物、不发生用于生成摘要的 provider 调用。
- [ ] 1.2 为 unit 完成编写测试，断言不因交接摘要缺失而失败或降级。
- [ ] 1.3 为下游依赖解析编写测试，断言仍获得上游 `provided_contracts` 与 `provided_capabilities`，内容与移除前一致。
- [ ] 1.4 为运行时权威校验编写回归测试，断言 commit / revision / status 不一致时仍失败关闭，判定口径不变。
- [ ] 1.5 为既有 lineage 记录编写兼容测试，断言含 `tests` 与 `artifacts` 字段的历史 `HandoffRevision` 仍可反序列化且契约字段完整。
- [ ] 1.6 为启动 coding 编写测试，断言上游无交接摘要引用时不再被拒绝。
- [ ] 1.7 为 schema v2 契约编译编写回归测试，断言 `handoff_contract` 与 `handoff_field` 仍可用、编译产出契约与能力不变。

## 2. 生产实现

- [ ] 2.1 移除 `WorkItemHandoff` 模型及其存取 API、路径解析与文件产物。
- [ ] 2.2 移除交接摘要生成入口、provider 调用与占位降级路径。
- [ ] 2.3 从 `HandoffRevision` 移除 `tests` 与 `artifacts` 字段，并保证既有记录反序列化忽略多余字段。
- [ ] 2.4 调整 `build_group_handoff_revision` 不再读取交接摘要。
- [ ] 2.5 移除 lifecycle 层 `handoff_summary_ref`、`required_handoff_from`、`planned_handoff_summary` 与相关更新入口。
- [ ] 2.6 移除启动 coding 的 `work_item_handoff_missing` 前置校验。
- [ ] 2.7 移除 split engine 中 legacy 的 `required_handoff_from`、`max_handoff_chars`、`max_dependency_handoffs`；确认保留 `handoff_contract` 与 `handoff_field`。
- [ ] 2.8 移除 WebSocket 协议字段与前端类型、状态、组件对交接摘要的消费。
- [ ] 2.9 移除 `WorkItemHandoffMissing` 错误类型及其触发点与 web 错误映射。
- [ ] 2.10 确认未改动 `HandoffRevision` 契约语义、运行时权威校验、group completion 完成判定与 commit 绑定。

## 3. 验证与交付

- [ ] 3.1 运行本 change 相关定向测试与 group completion、lineage 存储、runtime handoff、split engine 既有回归，并区分既有失败基线。
- [ ] 3.2 运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 与各测试目标（`--lib`、`it_core`、`it_web`、`it_provider`、`it_product`、`it_task_run`）。
- [ ] 3.3 运行前端检查与测试：`cd web && pnpm tsc -b`、`cd web && pnpm test`。
- [ ] 3.4 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.5 经用户确认后重启后端，由用户验证 group final review 不再因交接摘要缺失判要求修改。
