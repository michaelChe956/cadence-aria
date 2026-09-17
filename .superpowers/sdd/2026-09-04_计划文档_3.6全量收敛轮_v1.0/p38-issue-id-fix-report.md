# P38 Issue ID 防撞号快修报告

## 根因

`IssueStore::create` 曾以 `issues/` 目录条目数计算 `issue_XXXX`：目录删除后，剩余条目数会小于历史最大编号。例如保留 `issue_0001` 与 `issue_0003` 时，计数为 2，下一次创建错误地分配 `issue_0003`。`write_json` 随后原子重命名到同一 `issue.json`，覆盖原 issue 记录；其遗留的 design-spec、work-item 等子目录则继续挂在新 issue 名下。

## 修法

- `src/product/id.rs` 新增 `next_sequential_id_in_directory`：遍历目标目录，只接受同名前缀且数字后缀可解析的条目（`.json` 后缀可选），取最大序号加一；目录不存在或没有匹配项时返回 `prefix_0001`。
- `IssueStore::create` 改用上述扫描器，保持四位零填充和既有 ID 格式。
- 新增真实文件系统回归测试：
  1. 空目录首个 issue 为 `issue_0001`；
  2. 创建三个、删除中间项后新建为 `issue_0004`，并确认首尾旧记录未被覆盖；
  3. 保留非连续编号 `0001/0003/0004` 后新建为 `issue_0005`；
  4. ID 工具测试验证只按同族最大编号分配，忽略异族条目。

## 同族调用方审计

已用 AST 搜索审计全部 `next_sequential_id` 调用点。

### 本提交修复

| 调用方 | 分配范围 | 处理 |
| --- | --- | --- |
| `IssueStore::create` | `issues/issue_XXXX/` | 已修复：目录同族最大编号加一。 |

### 已确认属于不同分配语义、未纳入本次紧急提交

| 调用方 | 理由 |
| --- | --- |
| `LifecycleStore::append_spec_version` | 版本号来自记录的显式 `version` 递增值，不从目录条目数推导。 |
| `LifecycleStore::next_workspace_session_id` | 已调用全局 `max_workspace_session_sequence`。 |
| `CodingAttemptStore::group_initialization` | 在尚未落盘的新 attempt 内按绑定数组索引确定 unit ID，非既有目录扫描。 |
| `CodingAttemptStore::unit_run` | 已循环探测现有 run ID 直到未占用。 |
| `ExecutionRecordStore::append` | JSONL 追加记录，不会以同名路径覆盖旧记录。 |

### 需由后续专属改动覆盖的持久化同型调用方

`ProjectStore`、legacy `RepositoryStore`、`RuntimeBindingStore`、Lifecycle 的 story/design spec、work-item plan、work item、verification plan、repository profile、provider review round/provider run，以及 Coding Attempt 的 context note、choice/stage/blocked gate、quality audit、coding unit、role run、plan-amendment context、code/internal review、rework instruction、human presentation revision 等，均在本轮 AST 审计中发现使用“记录数/列表长度”生成 ID 的模式。

本报告所述紧急提交严格限于用户被阻塞的 issue 覆盖路径；这些调用方未随意批量改写，以免在无各自回归场景和并发 owner 协调下扩大快修风险。Controller 应按各存储契约分别完成 max+1 迁移和回归测试。

## 测试证据

- RED：`cargo test --locked product::issue_store::tests` 在修复前失败 2 项，实际得到 `issue_0003`，预期分别为 `issue_0004` 和 `issue_0005`。
- GREEN：`cargo test --locked product::id::tests` → 1 passed。
- GREEN：`cargo test --locked product::issue_store::tests` → 4 passed。
- GREEN：`cargo test --locked product::project_store::tests` → 2 passed（审计时的同型验证）。
- `cargo fmt` 已成功执行；修复期间 `cargo check --locked` 已成功执行。
- 本次最终 `cargo test` / `cargo clippy --all-targets --all-features --locked -- -D warnings` 受并行 worker 正在完成 `workspace_session` API 改动阻断：`src/web/workspace_session/tests.rs` 引用了尚未落盘的 `start_run`、`active_run`、`finish_run`、`abort_active_run`。该阻断与本提交无关，待其 Step 提交后由 controller 补跑全量门禁。
