## 1. Draft 生成契约与有界修复

- [x] 1.1 建立可信验证命令目录，并将其与 ID 注册表、自检规则一起投影到 Work Item Draft Prompt 和结构化输出约束中。（需求：work-item-draft-generation-reliability）
- [x] 1.2 实现解析后语义校验失败的一次自动修复、诊断持久化及服务端统一的重写反馈合并，保持无效 Draft 不可接受。（需求：work-item-draft-generation-reliability、work-item-draft-validation-feedback）

## 2. Draft 失败反馈界面

- [x] 2.1 在 Work Item Draft 确认区实现可访问的校验失败摘要、完整 findings 展开和无 findings 的降级提示。（需求：work-item-draft-validation-feedback）
- [x] 2.2 接入“根据校验错误重写”与暂停行为，确保有效 Draft、Story Spec、Design Spec 和 Artifact 深度查看不回归。（需求：work-item-draft-validation-feedback）

## 3. 质量测试与 Prompt 调优基线

- [x] 3.1 为跨字段引用、可信命令、一次自动修复和失败反馈补充确定性 Rust、WebSocket 与前端回归测试；将本次失败的全部错误码类别固化为回归样本。（需求：work-item-draft-generation-reliability、work-item-draft-validation-feedback）
- [x] 3.2 回退超出范围的运行时评估模块、CLI、30 场景语料和评估报告代码，保持 Prompt 试运行不进入产品代码或 CI。（需求：work-item-draft-generation-reliability）
- [ ] 3.3 在操作者授权的 Claude Code 上，以两个脱敏案例各 10 个有效首次输出执行校验；每个案例必须 10/10 成功后转入人工验证。未达标时仅进行单变量 Prompt 文案调优并重跑同一案例。（需求：work-item-draft-generation-reliability）
- [x] 3.4 将两个脱敏 Case、首次输出判定、Provider 中断处理与人工授权提醒沉淀为已启用项目规则；Prompt 或结构化契约变更时必须提醒操作者授权执行基线，且不得自动调用 Provider 或加入 CI。（需求：work-item-draft-generation-reliability）

## 4. 完整验证

- [ ] 4.1 执行规定的 Rust 格式化、静态检查、单元/集成测试、前端测试和类型检查，并验证 OpenSpec 场景覆盖与真实评估的显式触发边界。（需求：work-item-draft-generation-reliability、work-item-draft-validation-feedback）
