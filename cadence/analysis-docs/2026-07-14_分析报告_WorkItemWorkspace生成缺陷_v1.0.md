# Work Item Workspace 生成缺陷分析

## 文档信息

- 日期：2026-07-14
- 类型：分析报告
- 适用对象：Work Item Workspace、Work Item Draft 生成器、Compile 流程、Coding Workspace、Code Reviewer
- 当前 Issue：`project_0001 / issue_0001`
- 当前 Coding Attempt：`coding_attempt_0001`

## 结论摘要

本次 Coding Attempt 先后在 Work Item 4、Work Item 6 和 Work Item 8 暴露了同一类生成缺陷：Work Item Draft 对前置接口、真实调用链和受影响测试的判断不完整，导致完成任务所必需的文件被排除在 Exclusive Write Scopes 之外，甚至被放入 Forbidden Write Scopes。

Coder 遵守范围时只能报告 blocker；人工临时扩大范围后，Reviewer 又可能因为拿不到权威修订上下文而持续报告越界。这不是 Story Spec 或 Design Spec 的主要问题，核心问题位于 Work Item Workspace 的 Draft 生成、Compile 同步、依赖 handoff 校验、受影响测试归属和 Reviewer 上下文传递。

## 事件一：Work Item 4 生成问题

### Work Item

- 标题：真实 Provider 执行路径统一门禁
- Compiled Work Item：`work_item_compile_20260712024139064_004`
- Source Draft：`draft_006`
- Outline：`outline_provider_execution_gate`

### 原始 Draft 为什么不可执行

原始 `draft_006` 要求 WebRuntime、Task Run、Workspace 和 Coding 使用同一个 `ProviderAvailabilityGate`，并要求真实 Provider 在解析时和实际执行前进行双重门禁。

但生成器没有核对真实构造顺序：

1. `src/web/app.rs` 先构造 `WebRuntime`。
2. `src/web/state.rs::WebAppState::with_events` 随后才创建共享 `ProviderAvailabilityGate`。
3. `runtime.provider_adapter()` 在没有收到共享 gate 的情况下返回未包装的同步 Provider adapter。

要完成 Design Spec 的统一门禁，必须修改 `src/web/app.rs` 或 `src/web/state.rs`，并需要修正 `src/web/provider_availability.rs` 中的 host 阻断错误码。

然而原始 Draft 同时规定：

- `src/web/app.rs`：Forbidden
- `src/web/state.rs`：Forbidden
- `src/web/provider_availability.rs`：Forbidden

因此 Work Item 4 在生成时已经不可执行。Coder 当时没有代码改动，并正确报告 dependency handoff blocker。

### 具体生成缺口

1. **共享 gate 无法注入 WebRuntime**
   - Draft 假设前置 Work Item 已交付 gate 注入接口。
   - 实际 WebRuntime 没有接收共享 gate 的构造参数或安装接口。
   - 完成任务必须修改被禁止的 `app.rs` 或 `state.rs`。

2. **错误码修正文件被禁止**
   - Design Spec 要求 `host_real_workflow_blocked`。
   - 当时代码返回 `real_workflow_blocked`。
   - 定义该行为的 `src/web/provider_availability.rs` 被 Draft 禁止修改。

3. **依赖 handoff 只有文字目标，没有实现核验**
   - Draft 根据前序 handoff 推断接口已经存在。
   - 生成阶段没有检查实际类型、构造器和调用链能否让当前 Work Item 在允许范围内完成接线。

4. **Reviewer 缺少权威 Work Item 上下文**
   - 历史 Code Review 的 `raw_markdown_or_sections` 为空。
   - Reviewer 无法核对人工范围调整、Verification Plan 和 handoff。
   - Reviewer 因上下文缺失错误要求 Web `tsc`、Web 单测和 Web E2E 证据；实际 Work Item 4 的正式 Verification Plan 只有 Rust/Cargo 命令。

5. **Coder evidence 与正式 handoff 不一致**
   - Coder completion report 中包含测试说明。
   - 当时正式 `work-item-handoff.json` 缺失，Reviewer 只能看到截断后的 completion excerpt。
   - 累积 diff 也被截断，进一步放大了误判。

### 当时的人工修订

人工修订把以下文件从 Forbidden Scopes 移入 Compiled Work Item 4 的 Exclusive Scopes：

- `src/web/app.rs`
- `src/web/state.rs`
- `src/web/provider_availability.rs`

并把修改限制为共享 gate 接线和 host 错误码修正，禁止借此改动 Work Item 2 已完成的其他 Provider 健康逻辑。

### 遗留缺陷：Source Draft 与 Compiled Work Item 漂移

当前状态中：

- Compiled Work Item 4 已包含人工追加的范围调整。
- Source Draft `draft_006` 仍保留原始 Forbidden Scopes。

这说明 Work Item Workspace 缺少正式 amendment 机制。人工修订没有同步回源 Draft，也没有独立的修订版本和审计关系。后续重新 Compile、重新 Review 或读取源 Draft 时，仍可能得到旧约束。

## 事件二：Work Item 6 生成问题

### Work Item

- 标题：代码库添加 API 成功摘要与结构化失败契约
- Compiled Work Item：`work_item_compile_20260712024139064_006`
- Source Draft：`draft_008`
- Outline：`outline_repository_registration_api`

### 原始 Draft 的错误假设

Work Item 6 只允许修改：

- `src/web/handlers/product_resources.rs`
- `src/web/handlers/dto.rs`
- `src/web/error.rs`

同时禁止 `src/product/**`、`src/cross_cutting/**` 和 `tests/**`。

生成器做出了以下未经实现核验的假设：

1. Cadence-skills 模块已经提供生产 HOME resolver。
2. Repository registration core 已经提供不会泄漏敏感信息的清理后错误摘要。
3. Fake runtime 可以直接复用与真实 runtime 相同的共享 gate 和 registry 构造。
4. API 从 HTTP 200/顶层 Repository JSON 改为 HTTP 201/envelope 后，现有全量测试仍能通过。

### 具体生成缺口

1. **生产 HOME resolver 假设错误**
   - 已完成 Work Item 3 提供的是注入 `home` 的 `CadenceSkillsManager` 和 `CadenceSkillsPaths::from_home`。
   - 它没有交付 Work Item 6 Draft 所假设的生产 HOME resolver。
   - 原 Draft 又禁止 Web handler 自行解析 HOME，导致请求级 manager 无法在允许范围内构造。

2. **安全错误摘要假设错误**
   - Repository initializer 的部分 Provider 失败路径已有清理和截断。
   - Cadence 错误仍可能通过 `error.to_string()` 携带绝对路径、Git argv 或原始原因。
   - Draft 假设 core 已完整保证安全，但未核对公开字段和实际调用路径。

3. **Fake runtime 的具体 gate 与兼容闭包语义不一致**
   - WebAppState 在 Fake runtime 下可以让兼容 availability 闭包返回可用。
   - Repository coordinator 接收的却是具体 `Arc<ProviderAvailabilityGate>`。
   - 具体 gate 仍读取真实健康快照，导致 Fake Web 测试无法直接复用。
   - 上一轮 Coder 新增 `ProviderAvailabilityGate::new_test_mode` 绕过真实 Provider 检查，但这扩大了全局门禁语义，不应保留。

4. **API 契约变化与既有测试所有权矛盾**
   - Work Item 6 要求成功返回从 HTTP 200 改为 HTTP 201。
   - 返回体从顶层 Repository 改为 `repository` envelope。
   - `tests/it_web/web_product_api.rs` 仍断言旧契约。
   - Work Item 6 禁止修改 `tests/**`，但 Verification Plan 又要求 `cargo test --locked` 全量通过。
   - 该 Work Item 在生成时无法同时满足写入范围和验证门禁。

5. **人工范围例外掩盖 Draft 缺陷**
   - 人工曾一次性授权多个 `src/product/**`、`src/cross_cutting/**` 和测试文件。
   - 授权只能消除范围证据问题，不能证明 `new_test_mode` 等实现技术上正确。
   - Reviewer 没有直接取得原始人工 context note，导致范围判断反复。

## 事件三：Work Item 8 生成问题

### Work Item

- 标题：Provider 健康与代码库初始化贯通集成测试
- Compiled Work Item：`work_item_compile_20260712024139064_008`
- Source Draft：`draft_010`
- Outline：`outline_integration_provider_repository`

Work Item 8 的 Draft 已经明确列出以下旧测试可能受到 HTTP 201/envelope 影响：

- `tests/it_web/web_product_api.rs`
- `tests/it_web/web_lifecycle_api/part_01.rs`
- `tests/it_web/web_coding_attempt_api/part_02.rs`
- `tests/it_web/web_coding_attempt_api/part_05.rs`
- `tests/it_web/web_work_item_generation/part_01.rs`

但 Work Item 8 的 Exclusive Scopes 只允许新增两个测试模块和 fixture，不允许修改任何上述既有测试。

Draft 的处理方式是让 Coder 在发现旧测试失败后报告 blocker。这个设计没有为 blocker 指定任何可以实际修改旧测试的 Work Item，只是把生成期可以发现的问题推迟到 Coding 阶段。

## 生成器根因归类

### 1. 未验证依赖接口是否真实存在

Draft 生成只消费前序 handoff 文本，没有确认依赖声称交付的类型、构造器和公开方法是否真实存在。

### 2. 未验证写入范围能否覆盖真实调用链

生成器没有从目标行为反向追踪入口、状态构造、factory、registry、错误映射和既有测试，导致必改文件被放入 Forbidden Scopes。

### 3. 未把契约变更传播到既有测试

HTTP status、JSON 字段层级或错误码发生变化时，生成器没有自动检索旧断言并把精确测试文件分配给当前或前置 Work Item。

### 4. Verification Plan 与写入范围未做一致性检查

生成器允许同时出现“全量测试必须通过”和“禁止修改必然失败的旧测试”。

### 5. Blocker 没有合法处理者

Work Item 8 已提前识别旧测试风险，却只要求 Coder 报告 blocker，没有生成迁移项或授予任何 Work Item 修改权限。

### 6. 已完成依赖没有正式修订路径

当后续 Coding 发现已完成依赖缺少接口时，平台只能停住或依赖人工 context note，没有“新增修复项”“正式 amend 当前 Work Item”“重新 Compile”三种明确流程。

### 7. Source Draft 与 Compiled Work Item 缺少同步和版本审计

Work Item 4 的人工修订只落在 Compiled Work Item，Source Draft 仍为旧版，导致不同消费者读取到不同约束。

### 8. Reviewer 上下文不完整时会自行补全要求

Reviewer 没有正式 Work Item、amendment 和完整证据时，可能凭模型习惯扩展验证要求，包括错误引入 E2E/Playwright。

## Work Item Workspace 后续优化要求

### 1. 依赖接口实证校验

Draft 生成后应检查：

- 依赖声称交付的类型、构造器和公开方法是否真实存在。
- 当前 Work Item 的允许文件是否能取得并连接这些接口。
- 关键对象的构造顺序是否使依赖可注入。
- 若必须修改 Forbidden Scope，Draft 在进入 Review 前应标记为不可执行。

### 2. 写入范围与调用链可行性校验

生成器应从目标行为反向追踪到实际入口和接线点，至少覆盖：

- 入口 handler/CLI。
- 状态或依赖构造位置。
- adapter/factory/registry。
- 错误映射位置。
- 受影响的既有测试。

若调用链上的必改文件不属于 Exclusive Scopes，Draft validator 应阻止接受。

### 3. 契约变更影响测试自动归属

当 Draft 修改 HTTP status、JSON 字段层级、错误码或公共类型时，应自动检索并归属：

- 断言旧 HTTP status 的测试。
- 读取旧 JSON 路径的测试。
- 构造旧 DTO/类型的测试。
- 消费旧错误码的测试。

### 4. Verification Plan 与范围一致性检查

当 Verification Plan 包含全量回归时，生成器应判断当前变更是否必然使 Forbidden Scope 中的现有测试失败。若是，应把精确测试文件加入当前 Work Item、生成前置迁移项或调整契约变更顺序。

### 5. 完成依赖的缺陷处理策略

当 Coding 发现已完成依赖缺少接口时，平台应提供：

1. 回到 Work Item Workspace 生成新的修复项。
2. 正式 amend 当前 Work Item，并记录为何接管依赖缺口。
3. 终止当前计划并重新 Compile。

### 6. Draft Amendment 与 Compile 同步

任何人工范围调整必须：

- 生成新的 Draft amendment/version。
- 同步更新 Compiled Work Item。
- 保留旧版、修订原因、操作者、时间和受影响字段。
- 让 Reviewer 和 Coder 始终读取同一个修订版本。

### 7. Reviewer 权威上下文

Code Reviewer 和 GroupFinalReview 必须直接取得：

- 当前正式 Compiled Work Item。
- Exclusive/Forbidden Scopes。
- Verification Plan。
- 依赖 handoff。
- 正式 amendment 和人工授权原文。
- 当前 Unit 的精确 diff 和 handoff 测试证据。

当 Work Item 上下文缺失时，Reviewer 不得自行扩展验证要求，尤其不得引入 E2E/Playwright。

## 回归案例清单

1. Work Item 4：目标要求共享 gate，但构造 gate 的 `state.rs` 被禁止。
2. Work Item 4：Design 要求修正 host 错误码，但定义文件被禁止。
3. Work Item 4：只修改 Compiled Work Item 后 Source Draft 仍为旧范围。
4. Work Item 4：Reviewer 缺少 Work Item 上下文后错误要求不存在的 Web/E2E 门禁。
5. Work Item 6：Draft 假设生产 HOME resolver 已交付，但实际只有注入式构造器。
6. Work Item 6：Draft 假设 core 摘要安全，但实际调用链仍可能携带敏感文本。
7. Work Item 6：API 改为 201/envelope，但旧测试文件被禁止且全量测试必跑。
8. Work Item 8：Draft 已识别旧测试 blocker，却没有任何 Work Item 获得修改权限。
9. 人工 context note 只传给 Coder，Reviewer 无法验证正式范围修订。

## 证据索引

- Work Item 4 Source Draft：`.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/round_002/draft_006.json`
- Work Item 4 Compiled：`.aria/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260712024139064_004.json`
- Work Item 4 历史备份：`/tmp/coding_attempt_0001-before-draft4-reset.tgz`
- Work Item 4 历史代码 patch：`/tmp/aria-issue_0001-draft4-uncommitted.patch`
- Work Item 4 blocker reviews：备份中的 `code_review_0009.json`、`code_review_0010.json`、`code_review_0011.json`
- Work Item 6 Source Draft：`.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/round_002/draft_008.json`
- Work Item 6 Compiled：`.aria/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260712024139064_006.json`
- Work Item 8 Source Draft：`.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/round_002/draft_010.json`
- Work Item 8 Compiled：`.aria/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260712024139064_008.json`
- Work Item 6 回退前备份：`/tmp/cadence-aria-coding_attempt_0001-before-work-item6-reset-20260714T023112Z`
