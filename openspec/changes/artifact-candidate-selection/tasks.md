# Tasks: artifact-candidate-selection

**工作包性质**：本文件只登记可跟踪的高层工作包与验收口径；精确文件、命令、测试和提交步骤由后续 `superpowers:writing-plans` 展开到 `cadence/plans/`，不得在实施计划中重定义本 change 的契约。

**全局边界**：不放宽 artifact gate 任何禁止规则；不解除 kimi artifact retry 排除（`kimi-code-provider-integration` SHALL 不动）；不改 AskUserQuestion 桥接；不为诊断新增 WebSocket/UI 公开语义。

## 1. 候选枚举与唯一选择

- [ ] 1.1 在 `artifact_extraction.rs` 实现顶层完整 fenced 候选枚举（含 XML marker 优先级、嵌套 fence 状态机、行/字节/hash 元数据），保留既有 fallback 兼容；单元测试覆盖 F-46 双 block 原文、同长度 fence 歧义、未闭合、多候选（REQ-ACS-01）。
- [ ] 1.2 新增 workspace-aware `select_workspace_artifact`（唯 gate-passing 选择，0/多通过 fail-closed，携带逐候选诊断），以 F-46 真实 durable 原文作为红绿验收样本（REQ-ACS-01）。
- [ ] 1.3 迁移全部共享调用方（provider drive、artifact retry、choice 检测、lifecycle 恢复、session reload 映射、coding context）到同一 selector；生产路径无旧首开—末闭残留（REQ-ACS-01）。

## 2. 失败诊断

- [ ] 2.1 候选选择/失败时写有界 kind=artifact 诊断事件到 timeline node（稳定 event id、逐候选边界与 hash、selection 结论、原文零复制）；诊断写失败可见且不放宽 gate（REQ-ACS-02）。

## 3. 弱模型 prompt 加固

- [ ] 3.1 在三个既有注入点统一负面清单（单一顶层 block/思考在 fence 外/四反引号包代码/骨架不回显），覆盖四 workspace 类型；split JSON 流不注入；prompt contract 断言同步（REQ-ACS-03）。

## 4. 契约更新与跨类型回归

- [ ] 4.1 表驱动回归覆盖 Story/Design/Work Item/legacy Work Item Plan：唯一有效候选恢复、两有效候选歧义失败、所选正文污染仍拒、reload/recovery 不重新错切、「一次成功」新判据（含前置示意 block 场景）（story-pipeline-weak-model-hardening MODIFIED + REQ-ACS 全场景）。
- [ ] 4.2 运行 change 级 strict 校验与受影响测试目标（lib/it_web/前端按触及面），记录证据。
