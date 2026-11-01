# Tasks: plan-compile-gate-visibility

**工作包性质**：本文件只登记可跟踪的高层工作包与验收口径；精确文件、命令、测试和提交步骤由后续 `superpowers:writing-plans` 展开到 `cadence/plans/`，不得在实施计划中重定义本 change 的契约。

**全局边界**：不改变 compile/batch/recovery 业务状态机、action 枚举语义、REQ-RET-02 typed 三命令、F-21 相位纪律或 F-30 终态守卫；不连接 4317；F-34 已正式纳入，provider 创建快照与共享默认应用必须和门可见性一起验收。

## 1. Cockpit 批次与 recovery 投影

- [x] 1.1 扩展统一 gate projection/inbox 派生，使成功整组确认和 compile 失败转 `work_item_batch_confirm` 都产生稳定、去重、可诊断的批次门条目（REQ-PCG-01、REQ-WSC-01）。
- [x] 1.2 为 `work_item_plan_compile_recovery` 派生 recovery 门摘要、可用 action 和处理中/拒绝/终态反馈；缺少 durable 身份或状态不一致时只读 fail-closed（REQ-PCG-02/03）。
- [x] 1.3 接通 Cockpit recovery 动作面与现有 action facade/driver lease/错误处理；保证 recovery action 不映射为 confirm、feedback、abandon 或隐式 advance（REQ-PCG-02、REQ-CG-02）。
- [x] 1.4 补齐投影/组件 `vitest` 覆盖：成功/失败批次门、recovery 三种 action、稳定 key 去重、重连乱序、driver 不可写、终态晚到与既有 SC/Story/Design 门回归。

## 2. WS 协议与核心状态守卫

- [x] 2.1 将既有 `WorkItemPlanCompileRecoveryAction` 纳入合法 SC recovery 状态的 WS 放行矩阵，并以 stage、flow、durable recovery 事实做最小增量校验；不扩大 HumanConfirm 三命令边界（REQ-CG-02、REQ-RET-02）。
- [x] 2.2 加固 recovery action 的非法阶段、错误 flow、缺失/不匹配 recovery 事实、phase mismatch、confirmed/terminated 终态零副作用拒绝，并保留可诊断协议错误（REQ-PCG-02/03、F-21、F-30）。
- [x] 2.3 补齐 `it_web` WS/handler 矩阵覆盖：SC 合法 recovery action、普通 HumanConfirm/AuthorConfirm/generate 拒绝、legacy 消息拒绝、重放/重连与终态守卫。
- [x] 2.4 补齐 `it_core` compile/batch/recovery 状态覆盖：成功批次门、compile 失败可恢复转批次门、三种 recovery 结果、重复动作幂等、恢复后不重复 compile、终态收口。

## 3. F-34 创建入口与默认 provider

- [x] 3.1 将 provider 选择/默认快照加入 Workbench plan/story/design 创建请求及其服务端兼容解析；用户已选或 localStorage 默认不得先被服务端环境默认替换，provider 不可用时返回明确可诊断结果（REQ-PPS-01）。
- [x] 3.2 抽取共享 provider 默认应用 hook/适配层，覆盖 Cockpit 与 legacy workspace；确保仅在 provider 未锁定且合法 PrepareContext 阶段幂等发送，已锁定会话不被默认覆盖（REQ-PPS-02）。
- [x] 3.3 在创建/开始生成入口展示实际 provider 和可用状态；provider 未确定或不可用时启动 fail-closed，不出现“先按服务端默认运行、再补发纠正”的竞态（REQ-PPS-03）。
- [x] 3.4 补齐 provider 的 `vitest` 与 `it_web` 覆盖：创建载荷携带显式/默认 provider、不可用不静默回退、Cockpit/legacy 默认一致、锁定态不覆盖、开始生成前可见；必要的 `it_core` 覆盖服务端缺省兼容与可用性错误。

## 4. 集成验收与文档关闸

- [x] 4.1 对照本 change 四个 capability delta 逐项核查 requirement/scenario 与实现、测试证据一致；确认没有恢复 legacy 逐段协议或扩大三命令。
- [x] 4.2 运行 change 级 OpenSpec strict 校验与受影响的 `vitest`、`it_web`、`it_core` 目标测试；记录 recovery 可达性、F-21/F-30 守卫和 provider 创建竞态消除的证据。
- [x] 4.3 完成风险/回滚检查：无 schema/durable 迁移、无 4317 连接、前后端协议原子切换；将未覆盖 provider 跨设备偏好明确登记为后续演进，不把 localStorage 宣称为跨设备一致。
