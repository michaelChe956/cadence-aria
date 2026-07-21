# Work Item Workspace 生成缺陷分析

## 文档信息

- 日期：2026-07-14
- 类型：分析报告
- 适用对象：Work Item Workspace、Work Item Draft 生成器、Compile 流程、Coding Workspace、Code Reviewer
- 当前 Issue：`project_0001 / issue_0001`
- 当前 Coding Attempt：`coding_attempt_0001`

## 结论摘要

本次 Coding Attempt 先后在 Work Item 4、Work Item 6 和 Work Item 8 暴露了一组关联生成缺陷：Work Item Draft 对前置接口、真实调用链、受影响测试和跨 Work Item 累积结构变化的判断不完整。Work Item 4、6 出现了完成任务所必需的文件被排除在 Exclusive Write Scopes 之外、甚至被放入 Forbidden Write Scopes 的直接冲突；Work Item 8 主要表现为前置依赖未验证就绪、潜在旧测试迁移无人归属，却仍被调度进入 Coding。除此之外，前序 Work Item 可能把文件推过代码拆分门禁阈值，后续 Work Item 才发现并承担失败，但没有原文件和拆分接线文件的写入权限。

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

### 结论校正：Work Item 8 没有要求 Coder 越界修改实现

Work Item 8 明确规定“只增加后端集成测试与 fixture，不修改实现”，并禁止修改 `src/**`、`web/src/**`、`web/e2e/**`。因此，不能把 Reviewer 提出的所有未完成测试场景都归类为“完成行为所需文件位于 Forbidden Scopes”。

以下内容可以在 Work Item 8 的合法测试范围内完成，不构成写入范围冲突：

- Provider 健康测试矩阵。
- Cadence online clone、online update、offline 三分支。
- 软链安全矩阵。
- Repository 初始化成功、失败、交互和并发场景。
- AC 到测试函数的追踪表及相关 fixture。

Work Item 8 中真正需要区分的是“直接写入范围冲突”和“前置依赖未就绪”。

### 行为要求与范围问题的精确对应

#### 1. 旧 Repository POST 测试迁移：条件成立后属于直接范围冲突

Work Item 8 要求先检索并运行仍使用旧 HTTP 200 或旧顶层 Repository JSON 的集成测试，最终 Verification Plan 又要求 `cargo test --locked` 全量通过。如果下列测试被实际证明仍依赖旧契约，就必须修改这些测试才能满足全量门禁：

- `tests/it_web/web_lifecycle_api/part_01.rs`
- `tests/it_web/web_coding_attempt_api/part_02.rs`
- `tests/it_web/web_coding_attempt_api/part_05.rs`
- `tests/it_web/web_work_item_generation/part_01.rs`

这些文件不在 Work Item 8 的 Exclusive Scopes 中。`tests/it_web/web_product_api.rs` 被 Draft 声明为已由 Work Item 6 完成迁移，因此不能在未核验前把它继续算作 Work Item 8 的必改文件。

该冲突只有在旧测试确实失败后才能确认；Draft 只是提前识别了风险。Draft 要求正式扩展范围或创建前置迁移项，但平台没有提供对应的合法处理路径。

#### 2. Repository API 集成注入：属于前置公开 seam 就绪性问题

Work Item 8 要求 Repository API 在构造 router/state 时能够注入：

- 临时用户根。
- 共享 `BoundedCommandRunner`。
- Provider 健康 gate。
- 脚本化 `ProviderRegistry`。

测试文件只能消费公开的 integration-safe builder 或其他公开 seam。如果前置 Work Item 只交付了单元测试私有 seam，新增公开 builder 就需要修改 `src/**`，而 `src/**` 被 Work Item 8 禁止。

但 Work Item 8 并没有要求当前 Coder 越界补接口；它明确要求在这种情况下报告 `handoff blocker`。因此其准确分类是“依赖接口未就绪仍被调度”，不是“Reviewer 要求当前 Work Item 修改 Forbidden 文件”。

#### 3. AC-011 实际 HTTP 调用链：属于前置实现正确性问题

Work Item 8 要求通过 `coding.rs` 和 `lifecycle.rs` 的实际 HTTP 请求入口验证共享 Provider gate，而不是直接调用解析函数。如果请求仍绕过 gate，修复位置会落在 `coding.rs`、`lifecycle.rs` 或 `provider_availability.rs` 等 `src/**` 文件。

Work Item 8 同样没有授权当前 Coder 修改这些文件，而是明确要求记录调用证据并退回 `outline_provider_execution_gate` 修复。因此这里暴露的是：已完成依赖缺少正式返修或 amendment 路径，平台却继续把同一 Work Item 送入 Coding。

### Work Item 8 的准确缺陷分类

1. 只有旧测试被实际证明需要迁移时，才形成当前 Work Item 的直接写入范围冲突。
2. 公开 integration-safe seam 不存在时，形成依赖就绪性 blocker。
3. 实际 HTTP 调用链仍绕过 gate 时，形成前置实现返修 blocker。
4. 普通测试矩阵或断言缺失仍属于 Work Item 8 的正常 Coding 内容，不能误判为范围冲突。
5. 平台缺陷在于 Compile/Coding 前没有验证依赖就绪性，并且 blocker 没有合法处理者或自动回退目标。

## 事件四：前序改动触发结构或质量门禁，后续 Work Item 无权修复

### 问题定义

Cadence Aria 是通用 AI 开发平台，本问题不绑定 Rust、React 或任何特定语言、框架和构建工具。这里的“format 门禁”是用户口径，平台侧应将其建模为仓库定义的结构或质量门禁，例如格式化、静态检查、复杂度、文件或模块规模、生成代码布局，以及“超长后必须拆分”等项目自定义规则。当前 Rust 项目只是本次复现案例，不应成为平台规则的适用边界。

可能出现以下跨 Work Item 链路：

1. 前序 Work Item 在自己的 Exclusive Scopes 内合法修改某个文件。
2. 累积修改使该文件超过代码拆分门禁阈值，或者使下一次新增内容必然触发拆分要求。
3. 前序 Work Item 完成时没有执行同一套结构或质量门禁，或者平台没有把门禁失败归因到引入超限的 Work Item。
4. 后续 Work Item 运行仓库全量验证或 Review 门禁时，首次发现必须拆分该文件。
5. 拆分通常需要同时修改原文件、拆出的新文件、模块或包注册入口、公共导入导出、清单或配置，以及相关测试接线文件；具体集合由目标仓库技术栈决定。
6. 这些文件可能不在后续 Work Item 的 Exclusive Scopes 中，甚至位于 Forbidden Scopes，导致后续 Coder 无法同时满足门禁和写入边界。

### 准确分类

- 如果超限由前序 Work Item 引入，但前序完成门禁没有发现，这是“完成门禁覆盖不足和违规归因错误”。该判断与实现语言无关。
- 如果后续 Work Item 自身的预期改动会把文件推过阈值，而 Draft 没有预留拆分所需文件，这是“写入范围闭包计算不足”。
- 如果门禁规则或阈值在两个 Work Item 之间发生变化，这是“门禁版本和基线未固化”。
- 后续 Reviewer 发现问题本身可能是正确的，但不能把修复责任直接交给没有权限的当前 Coder。

### 不应采用的处理方式

- 不应让后续 Coder 任意扩大 Forbidden Scopes。
- 不应为了让当前 Work Item 通过而关闭或放宽代码拆分门禁。
- 不应把前序 Work Item 引入的既有失败算作后续 Work Item 的新增缺陷。
- 不应要求后续 Coder 在未获得正式 amendment 的情况下顺手重构无归属文件。

### 正确的流程要求

1. 每个 Work Item 完成前必须执行目标仓库声明的同一版本结构与质量门禁，违规应在引入它的 Work Item 内处理。
2. 下一个 Work Item 开始前应保存并验证门禁基线，区分既有失败和当前 diff 新增失败。
3. Draft 生成器应根据目标仓库的门禁规则、当前结构和预计增量判断是否会触发拆分，并把原文件、拆分产物、注册入口、清单配置及测试接线文件纳入范围闭包。
4. 如果依赖已经完成才发现必须拆分，平台应生成独立修复 Work Item、正式 amend 当前 Work Item 或重新 Compile，而不是继续把 blocker 送回同一 Coder。
5. Reviewer 应报告门禁失败的首次引入 Work Item、当前基线状态和所需修复文件，避免把正确 finding 路由给错误的执行者。

## 事件五：Code Review 返修扩展了写入文件，但 Work Item 有效范围未同步

### 当前复现

Work Item 1“Provider 健康检查、状态路径与统一执行门禁基础”的原始 Exclusive Write Scopes 没有包含：

- `src/cross_cutting/provider_adapter.rs`
- `src/protocol/provider_errors.rs`

后续 Code Reviewer 发现 `provider_unavailable` 错误码在门禁错误转换中丢失，要求 Coder 修改这两个文件。该修复在技术上必要，文件也不属于 Forbidden Write Scopes；单项 Code Review 和 Group Final Review 均确认实现正确，Group Final Review 最终返回 `approve`。

但平台没有将 Code Review 返修引入的精确文件同步到 Work Item 的有效写入范围。Work Item 1 handoff 的 `files_changed` 已包含上述文件，而最终完成门禁仍只读取原始 `exclusive_write_scopes`，因此在 Group Final Review 通过后报错：

```text
coding_start_failed: work_item_diff_scope_violation: src/cross_cutting/provider_adapter.rs
```

`src/protocol/provider_errors.rs` 同样不在原始 Exclusive Write Scopes 中，因此第一个文件被处理后，仍可能继续出现同类范围错误。

### 问题分类

1. **Work Item 写入范围闭包不完整**：原始范围没有包含错误码契约修复的直接依赖文件。
2. **缺少 Code Review 返修范围的正式 amendment**：Reviewer 要求范围外修改后，平台没有生成可审计的范围修订记录。
3. **不同阶段消费的范围不一致**：Coder 和 Reviewer 根据返修上下文接受了额外文件，最终确定性门禁却仍使用原始范围。
4. **门禁失败暴露过晚**：范围冲突没有在当前 Work Item 返修或完成时处理，而是延迟到全部 Work Item 和 Group Final Review 通过后才阻断 Attempt 收尾。

### 后续优化方向

- Work Item Draft 生成时应计算完成目标所需的直接契约、错误类型和 adapter 文件范围闭包。
- Code Review 要求修改 Exclusive Scopes 之外的文件时，应产生精确、可审计的范围 amendment，不得仅依赖 prompt、handoff 叙述或 Reviewer 自行默认。
- Coder、Code Reviewer、Group Final Review 和最终完成门禁必须消费同一份有效写入范围。
- 范围不一致应在当前 Work Item 完成前阻断并提供合法处理路径，不应等到 Group Final Review 通过后再以 `coding_start_failed` 终止流程。
- Forbidden Write Scopes 仍必须保持绝对禁止，不能由 Reviewer 或 amendment 自动扩展。

## 生成器根因归类

### 1. 未验证依赖接口是否真实存在

Draft 生成只消费前序 handoff 文本，没有确认依赖声称交付的类型、构造器和公开方法是否真实存在。

### 2. 未验证写入范围能否覆盖真实调用链

生成器没有从目标行为反向追踪入口、状态构造、factory、registry、错误映射和既有测试。Work Item 4、6 因此出现必改文件被排除或禁止；Work Item 8 则主要表现为依赖公开 seam 未经验证、旧测试迁移风险未被正式归属。

### 3. 未把契约变更传播到既有测试

HTTP status、JSON 字段层级或错误码发生变化时，生成器没有自动检索旧断言并把精确测试文件分配给当前或前置 Work Item。

### 4. Verification Plan 与写入范围未做一致性检查

生成器允许同时出现“全量测试必须通过”和“已识别可能失败的旧测试不属于任何可修改范围”。只有运行结果证明旧测试确实失败后，才能认定为直接冲突。

### 5. Blocker 没有合法处理者

Work Item 8 已提前识别旧测试迁移、公开 integration-safe seam 和实际 HTTP gate 三类风险，却只要求 Coder 报告 blocker，没有生成迁移项、依赖返修项或正式 amendment，也没有授予任何 Work Item 对应修改权限。

### 6. 已完成依赖没有正式修订路径

当后续 Coding 发现已完成依赖缺少接口时，平台只能停住或依赖人工 context note，没有“新增修复项”“正式 amend 当前 Work Item”“重新 Compile”三种明确流程。

### 7. Source Draft 与 Compiled Work Item 缺少同步和版本审计

Work Item 4 的人工修订只落在 Compiled Work Item，Source Draft 仍为旧版，导致不同消费者读取到不同约束。

### 8. Reviewer 上下文不完整时会自行补全要求

Reviewer 没有正式 Work Item、amendment 和完整证据时，可能凭模型习惯扩展验证要求，包括错误引入 E2E/Playwright。

### 9. 跨 Work Item 结构或质量门禁缺少基线与责任归属

平台没有在每个 Work Item 边界固化目标仓库声明的结构或质量门禁、相关结构指标和失败基线，也没有把违规首次出现的位置归因到具体 Work Item。结果是前序改动产生的拆分义务被延迟到后续 Work Item，而后续 Work Item 的写入范围并未覆盖完成拆分所需的完整文件集合。该缺陷适用于任意技术栈和用户自定义门禁。

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

### 8. 通用结构与质量门禁的范围闭包和基线校验

Work Item Draft、Compile 和 Coding 门禁应共同记录：

- 目标仓库声明的门禁标识、命令、配置及版本。
- Work Item 开始前的门禁基线。
- 当前目标代码单元的结构状态、相关指标和预计增量。
- 拆分需要修改的原文件、拆分产物、模块或包注册入口、公共导入导出、清单配置及测试接线文件。
- 门禁失败首次由哪个 Work Item 引入。

如果当前计划会触发拆分，但完整拆分文件集合不属于 Exclusive Scopes，Draft validator 应在进入 Coding 前阻止接受，并要求生成修复项、正式 amendment 或重新 Compile。

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
10. 任意技术栈中，前序 Work Item 使代码触发仓库声明的拆分门禁，后续 Work Item 首次发现失败，但原文件、拆分产物、注册入口、清单配置或测试接线文件不在其 Exclusive Scopes 中。

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
