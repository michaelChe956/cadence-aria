## ADDED Requirements

### Requirement: 语义闭合的 Draft 生成契约
系统 SHALL 向 Work Item Draft author 提供可生成语义闭合 Canonical Contract 的 Prompt 契约。Prompt MUST 约束任务完成条件、需求追踪、交接审查、blocker 目标和验证项之间的引用关系；必需验证项 MUST 使用可信验证命令目录中的非空命令。

#### Scenario: Provider 输出闭合引用的 Draft
- **WHEN** Provider 为具有验收标准、追踪需求和输出 contract 的 Outline 生成 Draft
- **THEN** Draft 的任务、交接和 blocker 引用 MUST 分别只引用对应已声明的 criterion、traceability 和 input/output contract ID，且完整本地校验通过

#### Scenario: 缺少可信必需验证命令
- **WHEN** 当前 Outline 没有足够的可信命令支持必需验证
- **THEN** 系统 MUST 生成明确的 context 或 blocker 路径，且不得接受含有 `required=true` 和空 command 的 Draft

### Requirement: 有界自动修复
系统 SHALL 在 Draft 已解析但本地语义校验失败时，自动执行恰好一次携带全部校验 findings 的修复生成，并对修复结果重新执行完整校验。

#### Scenario: 自动修复生成有效 Draft
- **WHEN** 初次 Draft 因本地语义校验失败且自动修复输出通过完整校验
- **THEN** 系统 MUST 将修复后的有效 Draft 作为当前候选，并保留自动修复诊断

#### Scenario: 自动修复仍失败
- **WHEN** 初次 Draft 和唯一一次自动修复都未通过完整校验
- **THEN** 系统 MUST 保存失败 Draft 与全部 findings，进入人工重写或暂停状态，且不得继续自动重试

### Requirement: Claude Code Prompt 试运行验证
系统 SHALL 在操作者明确授权后，允许使用一至两个脱敏 Draft 案例对 Claude Code 进行临时 Prompt 试运行。试运行 MUST 不创建运行时评估模块、CLI、持久化报告或默认 CI 步骤。

#### Scenario: 单案例达到首次通过率门槛
- **WHEN** 一个脱敏案例在 Claude Code 上取得 10 个有效首次输出
- **THEN** 系统 MUST 以首次输出通过既有本地 Validator 作为成功，并要求 10 次全部成功后才转入人工验证

#### Scenario: 双案例达到首次通过率门槛
- **WHEN** 两个脱敏案例各在 Claude Code 上取得 10 个有效首次输出
- **THEN** 系统 MUST 以首次输出通过既有本地 Validator 作为成功，并要求两个案例均全数成功后才转入人工验证

#### Scenario: Prompt 试运行未达标
- **WHEN** 临时试运行未达到 95% 首次通过率
- **THEN** 系统 MUST 只调整 Prompt 文案并重跑同一组案例；不得放宽 Validator、Schema 或接受门禁
