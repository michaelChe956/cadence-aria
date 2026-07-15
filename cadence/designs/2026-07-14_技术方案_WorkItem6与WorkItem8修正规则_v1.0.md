# Work Item 6 与 Work Item 8 修正规则

## 文档信息

- 日期：2026-07-14
- 类型：技术方案
- 当前 Issue：`project_0001 / issue_0001`
- 当前 Coding Attempt：`coding_attempt_0001`
- 当前修正目标：Work Item 6 `work_item_compile_20260712024139064_006`
- 生成缺陷分析：[Work Item Workspace 生成缺陷](../analysis-docs/2026-07-14_分析报告_WorkItemWorkspace生成缺陷_v1.0.md)

## 修正原则

1. Work Item 3 和 Work Item 5 已完成，不重新调度。
2. 当前计划无法在 Work Item 5 与 Work Item 6 之间插入修复项，因此 Work Item 6 在 Web/API 边界自闭环。
3. Work Item 6 不修改 `src/product/**` 或 `src/cross_cutting/**`。
4. API 契约实现与受影响的既有测试由同一个 Work Item 负责。
5. Source Draft、Compiled Work Item 和 Verification Plan 必须同步修改。
6. 不使用一次性人工范围例外替代正式范围修订。

## Work Item 6 写入范围

### 保留

- `src/web/handlers/product_resources.rs`
- `src/web/handlers/dto.rs`
- `src/web/error.rs`

### 新增

- `tests/it_web/web_product_api.rs`

### 不增加

- `src/product/**`
- `src/cross_cutting/**`
- `ProviderAvailabilityGate::new_test_mode`

`tests/**` 不能继续作为 Work Item 6 的整体 Forbidden Scope，因为它会与新增的精确测试文件冲突。Exclusive Write Scopes 继续作为严格 allowlist，其他测试文件仍不允许修改。

## HOME 处理

`product_resources.rs` 可以提供最薄的、请求级可注入 HOME resolver：

- 优先读取 `HOME`。
- `HOME` 缺失或为空时回退 `USERPROFILE`。
- 拒绝空值和相对路径。
- 单元测试通过注入闭包提供临时 HOME，不读写开发者真实 HOME。
- resolver 只负责取得用户根，不复制 CadenceSkillsPaths 的五个固定路径规则。

## 错误摘要处理

`src/web/error.rs` 在 API 出口执行最终安全清理：

- 保留已经安全的可诊断摘要。
- 截断超长文本。
- 清理控制字符。
- 脱敏 KEY、TOKEN、SECRET、PASSWORD 等敏感 token。
- 清理 HOME/USERPROFILE 绝对路径和完整命令路径。
- 不返回原始 Provider/Git 输出或 Debug 文本。
- 不通过正则推断业务错误码；`reason_code`、stage、command、retryable、action 等仍直接读取结构化字段。

该边界防御只保证当前公开 API 响应安全，不宣称已修改 Work Item 3/5 的内部错误模型。

## Fake runtime 处理

- 不修改全局 `ProviderAvailabilityGate`。
- 不增加允许真实 Provider 名称无条件通过的 test mode。
- 在 `product_resources.rs` 的请求级 factory/test seam 中注入确定性可用健康源、Fake registry 或记录型 registrar。
- 产品真实 runtime 仍使用 WebAppState 中的共享 gate、runner 和 registry。

## API 与旧测试迁移

Work Item 6 同步迁移 `tests/it_web/web_product_api.rs`：

- HTTP 200 改为 HTTP 201。
- `repository_id` 改为 `repository.repository_id`。
- `project_id` 改为 `repository.project_id`。

该文件与 API 契约实现必须在同一 Work Item 中完成，确保全量 Cargo 测试可通过。

## Work Item 8 调整

Work Item 8 不重复拥有 `tests/it_web/web_product_api.rs`。其 Source Draft 和 Compiled Work Item 应记录：

- 该文件由 Work Item 6 完成契约迁移。
- Work Item 8 只验证迁移后的契约和新增贯通场景。
- 开始 Work Item 8 前，必须重新检索其他旧 Repository POST 测试。
- 如果其他旧测试仍受影响，应在 Work Item 8 开始 Coding 前把精确文件加入合法范围，不能等 Coding 后再报告无人处理的 blocker。

## Work Item 6 Verification Plan

保留：

- `cargo test --locked --lib create_repository`
- `cargo test --locked --lib api_error`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`
- `cargo test --locked`

增加：

- `cargo test --locked --test it_web manages_workspace_repositories_and_keeps_issue_on_lifecycle_flow`

禁止要求或运行：

- Playwright
- Chrome/浏览器自动化
- Web E2E
- 安装浏览器

## 需要同步修改的数据

1. Work Item 6 Source Draft：`draft_008`
2. Work Item 6 Compiled：`work_item_compile_20260712024139064_006`
3. Work Item 6 Verification Plan：`verification_plan_compile_20260712024139064_006`
4. Work Item 8 Source Draft：`draft_010`
5. Work Item 8 Compiled：`work_item_compile_20260712024139064_008`

同步完成后必须确认：

- Work Item 6 Unit 仍为 `pending`。
- Coding Attempt 仍为 `running / prepare_context`。
- Work Item 1–5 的完成状态和提交不变。
- Work Item 6 Source Draft 与 Compiled Work Item 的范围、上下文和 Verification Plan 一致。
- Work Item 8 Source Draft 与 Compiled Work Item 对旧测试迁移归属的说明一致。
