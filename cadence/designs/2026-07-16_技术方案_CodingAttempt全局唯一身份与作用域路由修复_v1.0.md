# Coding Attempt 全局唯一身份与作用域路由修复技术方案

## 文档信息

- 日期：2026-07-16
- 版本：v1.0
- 类型：技术方案
- 适用范围：Coding Attempt 创建、Coding Workspace 路由、REST API、WebSocket、历史 `.aria` 数据兼容
- 目标分支：`feat-b-0715`

## 背景

当前 `.aria` 数据中同时存在以下两条合法 Coding Attempt：

- `project_0001 / issue_0001 / coding_attempt_0001`
- `project_0001 / issue_0002 / coding_attempt_0001`

Coding Attempt ID 由 Issue 目录内的序列分配，因此不同 Issue 可以产生相同 ID。Coding Workspace 页面、REST API 和 WebSocket 却只传递 `attempt_id`，后端需要扫描全部 Project 与 Issue 才能定位记录。当扫描到第二条同名记录时，`find_attempt_by_id` 返回 `coding_attempt_ambiguous`。

WebSocket 当前把所有读取错误统一包装为 `coding_attempt_not_found`，最终导致页面能够进入但无法恢复任何内容，并显示：

`coding_attempt_not_found: coding attempt not found: product_store_io: coding_attempt_ambiguous: coding_attempt_0001`

该问题不是 `.aria` 迁移损坏。迁移只是让历史 Issue 与新 Issue 的同名 Attempt 同时出现在一个状态目录中，从而触发既有身份模型缺陷。

## 目标

1. 新创建的 Coding Attempt ID 在整个 Aria 状态目录中全局唯一。
2. Coding Workspace、REST API 和 WebSocket 使用 Project、Issue、Attempt 组成的明确地址，不依赖全局扫描。
3. 不迁移、不重命名、不批量重写现有 `.aria` 历史数据。
4. 当前两个同名 `coding_attempt_0001` 都能从各自 Issue 正常打开。
5. 旧式单 ID 地址继续提供受控兼容，不静默选择错误记录。
6. 错误码准确区分未找到、历史 ID 歧义和作用域不一致。

## 非目标

- 不调整 `.aria/projects/<project>/issues/<issue>/coding-attempts/` 目录结构。
- 不迁移历史 Attempt、Role Run、Timeline、Chat Entry、Artifact 或 JSONL 事件引用。
- 不改变 Coding Workspace 的执行状态机、Provider 流程、门禁或恢复逻辑。
- 不修改 Story Spec、Design Spec 的 Workspace Session 身份模型。
- 不为 UUID 增加新的第三方依赖；项目已启用 `uuid` 的 `v4` 功能。

## 方案比较

### 方案一：UUID 新 ID、作用域地址、历史数据原地兼容

新 Attempt 使用 UUID；正式路由携带 Project、Issue、Attempt；旧数据不迁移。历史同名记录通过作用域精确访问，旧式单 ID 查询仅作为兼容入口。

优点：无全局计数器竞争，不破坏历史数据，能直接修复当前问题，长期身份清晰。

代价：需要同步修改前端路由、API 客户端、REST 路由和 WebSocket 地址。

### 方案二：全局顺序号并迁移历史数据

保留 `coding_attempt_0001` 格式，但把序列提升为全局共享，并重写全部历史记录及引用。

优点：ID 可读且连续。

代价：需要全局锁；历史 JSON、JSONL、Artifact 和 URL 重写范围大，当前样本包含数万条事件，迁移和中断恢复风险高。

### 方案三：只增加作用域路由

继续按 Issue 分配顺序 ID，仅让访问地址携带 Project 和 Issue。

优点：实现最小。

代价：不满足新 Attempt 全系统唯一的目标，单 ID 仍不能作为稳定身份。

## 设计决策

采用方案一：新 ID 使用 UUID，全链路使用作用域地址，历史数据原地兼容。

## 身份模型

### 新 ID 格式

新 Coding Attempt ID 使用以下格式：

`coding_attempt_<32位UUID十六进制>`

生成逻辑等价于 `Uuid::new_v4().simple()`。ID 只包含字母、数字和下划线，继续满足现有 `validate_relative_id` 约束。

UUID 不依赖共享序列文件，不会因不同 Project、Issue 并发创建而分配同一个 ID。删除 Attempt 后也不会复用 ID。

### 规范地址

即使新 ID 已全局唯一，正式访问仍使用以下复合地址：

- `project_id`
- `issue_id`
- `attempt_id`

复合地址用于表达资源归属、避免全局扫描，并对历史局部 ID 保持兼容。`attempt_id` 是资源身份，Project 与 Issue 是资源作用域和归属校验信息。

## 后端组件设计

### CodingAttemptStore

`CodingAttemptStore::get_attempt(project_id, issue_id, attempt_id)` 成为业务链路的规范读取方法。该方法读取固定路径，并校验记录中的三个身份字段与请求地址一致。

`get_attempt_by_id(attempt_id)` 和 `find_attempt_by_id(attempt_id)` 降级为旧式兼容查询，只允许旧路由使用：

- 匹配零条：返回 NotFound。
- 匹配一条：返回该记录。
- 匹配多条：返回结构化 Ambiguous 错误。

创建单 Work Item Attempt 和 Work Item Group Attempt 时，共享一个 UUID ID 生成辅助函数，删除 Issue 级序列分配依赖。历史 `.meta/coding-attempt-sequence.json` 文件保留但不再用于新建记录，也不主动删除。

### ProductStoreError

增加可结构化匹配的歧义和身份不一致错误，避免依赖 `Io(String)` 文本前缀判断：

- Ambiguous：资源 ID 匹配多个记录。
- IdentityMismatch：固定路径中的记录身份与请求作用域不一致。

现有 Gate 与 Workspace Session 的错误模型不在本次重构范围内；新错误变体先用于 Coding Attempt。

### REST 路由

创建接口已经携带 Project 与 Issue，保持不变：

- `POST /api/projects/{project_id}/issues/{issue_id}/work-items/{work_item_id}/coding-attempts`
- `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-attempts`

现有单 ID 操作增加作用域版本，并作为新前端的规范接口：

- `GET|DELETE /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}`
- `GET /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/diff`
- `POST /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/abort`
- `POST /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/execution-plan/confirm`
- `POST /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/execution-plan/change-request`
- `GET /api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/artifacts/{artifact_id}`

旧 `/api/coding-attempts/{attempt_id}` 路由暂时保留，通过兼容查询处理。它不再被新前端调用。

### WebSocket 路由

新规范地址为：

`/ws/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}`

握手后使用精确作用域读取 Attempt。后续 Runner、Gate、Context Note 和恢复操作沿用已加载 Attempt 中的 Project 与 Issue，不再按单 ID 重新扫描。

旧 `/ws/coding-attempts/{attempt_id}` 暂时保留：唯一匹配时兼容连接；多条匹配时发送真实 `coding_attempt_ambiguous` 协议错误并关闭连接。

## 前端组件设计

### CodingAttemptAddress

新增共享前端类型：

```typescript
type CodingAttemptAddress = {
  projectId: string;
  issueId: string;
  attemptId: string;
};
```

`IssueLifecycleWorkbench`、`AppShell`、Router、`CodingWorkspacePage`、API Client 与 `useCodingWorkspaceWs` 使用该对象传递完整身份，避免多个字符串参数顺序错误。

### Workbench 跳转

Lifecycle 卡片已经持有 `selectedProjectId`、`card.issueId` 和 Attempt DTO，因此创建或复用 Attempt 后跳转到：

`/workbench/projects/{projectId}/issues/{issueId}/coding/{attemptId}`

Work Item 和 Work Item Group 使用同一跳转辅助函数。

### 旧页面地址

保留 `/workbench/coding/{attemptId}`：

- 唯一匹配时跳转到新规范地址。
- 多条匹配时显示历史 Attempt ID 冲突提示，并提供返回 Workbench 的入口。
- 不按最近更新时间、状态或当前选中 Issue 猜测目标记录。

## 数据流

### 新建并进入

1. Workbench 使用 Project、Issue、Work Item 或 Plan 创建 Attempt。
2. 后端生成 UUID ID，并写入现有 Issue 目录。
3. 创建响应返回 Attempt DTO。
4. 前端使用当前 Project、Issue 和响应中的 Attempt ID 构建规范页面地址。
5. Coding Workspace 使用相同地址建立 REST 与 WebSocket 连接。
6. 后端固定路径读取并校验记录身份，然后发送 Session State。

### 打开历史 Attempt

1. Lifecycle 响应在对应 Issue 范围内返回历史 Attempt。
2. Workbench 使用 Lifecycle 所属 Project、Issue 和历史 Attempt ID 构建规范地址。
3. 后端直接读取该 Issue 下的记录，不扫描其他 Issue。
4. 即使另一个 Issue 存在同名 Attempt，也不会产生歧义。

### 旧单 ID 地址

1. 兼容入口执行全局查询。
2. 唯一匹配时获取记录中的 Project 与 Issue，并引导到规范地址。
3. 多条匹配时返回冲突，不选择任意记录。

## 错误处理

| 场景 | 错误码 | HTTP/协议行为 |
|---|---|---|
| 精确作用域内没有记录 | `coding_attempt_not_found` | HTTP 404；WebSocket 发送错误后关闭 |
| 旧单 ID 匹配多条 | `coding_attempt_ambiguous` | HTTP 409；WebSocket 发送冲突后关闭 |
| 路径与记录身份不一致 | `coding_attempt_scope_mismatch` | HTTP 409；WebSocket 发送冲突后关闭 |
| ID 非法或存在路径逃逸 | 现有 validation 错误 | HTTP 400 |

WebSocket 不再把任意 Product Store 错误统一包装成 `coding_attempt_not_found`。未知存储错误继续使用 `product_store_error`，避免掩盖磁盘或 JSON 损坏。

前端在歧义和作用域冲突时显示可操作提示：返回 Workbench，从目标 Issue 重新进入。页面不得自动选择候选 Attempt。

## 历史数据兼容

- 不重命名现有 `coding_attempt_0001.json`。
- 不修改 Attempt 目录、Role Run、Timeline、Chat Entry、Gate、Report、Artifact 或 JSONL 内容。
- 不删除 Issue 级序列元数据。
- 新版本能够读取旧顺序 ID 和新 UUID ID。
- 当前 `issue_0001/coding_attempt_0001` 与 `issue_0002/coding_attempt_0001` 通过规范地址分别恢复。
- 若用户手工访问歧义旧地址，系统明确提示冲突。

## 测试设计

### Product Store 单元与集成测试

1. 在不同 Issue 创建新 Attempt，断言 UUID ID 均符合格式且互不相同。
2. 单 Work Item 与 Work Item Group 使用同一全局唯一 ID 生成策略。
3. 删除 Attempt 后新 ID 不复用。
4. 构造两个同名历史 Attempt，断言 `get_attempt(project, issue, id)` 分别读取正确记录。
5. 旧式单 ID 唯一匹配时成功，多条匹配时返回结构化 Ambiguous。
6. 固定路径内 JSON 的 Project、Issue 或 Attempt 不一致时返回 IdentityMismatch。

### REST API 测试

作用域路由覆盖以下行为：

- Snapshot 获取。
- Diff 获取。
- Abort。
- Delete。
- Execution Plan 确认与返修请求。
- Artifact 内容读取。

每类接口至少包含一个双 Issue、同名历史 Attempt 夹具，证明操作不会落到另一个 Issue。

旧式接口覆盖唯一匹配与歧义冲突两条兼容路径。

### WebSocket 测试

1. 两个 Issue 各包含 `coding_attempt_0001`，使用新作用域地址连接时分别收到正确 Session State。
2. 旧单 ID WebSocket 对歧义记录发送 `coding_attempt_ambiguous`，不得发送 `coding_attempt_not_found`。
3. 精确作用域不存在时仍返回 `coding_attempt_not_found`。
4. Scope mismatch 返回明确冲突，不启动 Coding Runner。

### 前端测试

1. Router 注册包含 Project、Issue、Attempt 的新页面地址。
2. Work Item 已有 Attempt、新建 Attempt 两条路径均传递完整地址。
3. Work Item Group 已有 Attempt、新建 Attempt 两条路径均传递完整地址。
4. API Client 的 Snapshot、Diff、Abort、Delete、Artifact 和 Execution Plan 请求包含完整作用域。
5. WebSocket Hook 使用新作用域地址。
6. 旧地址唯一时兼容跳转，歧义时显示冲突与返回入口。

### Workspace 三模块影响检查

- Story Spec：继续使用 Workspace Session 路由，不依赖 Coding Attempt，补路由回归断言。
- Design Spec：继续使用 Workspace Session 路由，不依赖 Coding Attempt，补路由回归断言。
- Work Item：Coding Workspace 使用新作用域身份链路，覆盖 Work Item 与 Work Item Group。

本次不修改 Story、Design、Work Item Artifact Workspace 共用的 Workspace Engine、Timeline 或 Chat 重建逻辑。

## 验证命令

实现阶段遵循 TDD，先写失败测试，再修改实现。最终运行：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
cd web && pnpm tsc -b
cd web && pnpm test
```

不得给 Cargo 命令添加 `-j 1`。

## 验收标准

1. 当前 `project_0001/issue_0001/coding_attempt_0001` 能恢复完整历史内容。
2. 当前 `project_0001/issue_0002/coding_attempt_0001` 能进入其新建状态页面。
3. 新创建的 Attempt 使用 `coding_attempt_<UUID>`，不同 Issue 并发创建不会重复。
4. 新前端、REST 和 WebSocket 不再调用单 ID 全局扫描链路。
5. 旧单 ID 唯一时兼容，多条时返回准确冲突。
6. `coding_attempt_not_found` 只用于真正不存在的记录。
7. 真实 `.aria` 历史文件内容、文件数量和目录结构不被实现或启动流程自动修改。
8. Rust 与前端标准验证全部通过。

## 实施边界

本方案可以作为一个独立实施计划完成，建议按以下顺序拆分：

1. Product Store UUID 与精确身份读取。
2. 结构化错误与作用域 REST API。
3. 作用域 WebSocket。
4. 前端地址对象、Router、API Client 与 Hook。
5. 旧地址兼容及完整回归验证。

除上述身份链路外，不捆绑 Coding Workspace 状态机、Provider、存储压缩或其它重构。
