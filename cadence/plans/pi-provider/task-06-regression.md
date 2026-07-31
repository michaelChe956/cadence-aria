# Task 6: 回归验证 + 边界验证 + 前后端质量检查（tasks 6.1, 6.2, 6.3）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints。依赖 Task 1~5 全部完成。

**Goal:** 补齐回归测试，验证仓库初始化与 Task Runner 未被 Pi 扩张，执行前后端质量检查。

**对应 spec requirement:** 全部（回归覆盖所有 requirement 与 scenario）。

**Files:**
- Test: `src/cross_cutting/pi_provider/tests.rs`（健康/目录/会话协议/取消/恢复/Auto 运行）
- Test: `src/product/workspace_engine/tests/`、`tests/it_product/product_coding_workspace_engine/`（Provider/权限/fail-fast）
- Test: `src/task_run/provider_factory.rs`、`src/web/provider_availability.rs`（Task Runner 拒绝 Pi）

---

## Step 1: Pi 后端协议回归（tasks 6.1）

`src/cross_cutting/pi_provider/tests.rs` 用 Task 2 的 fixture 覆盖：
- 健康检查（`pi --version` 解析，Task 1）
- 目录展示（Task 1）
- 会话协议：文本流、工具事件、完成、错误映射（Task 2）
- 取消：`abort` 命令 → 会话终止 → 前端呈已取消状态
- 恢复：`--session-id` 续接（Task 2）
- Auto 运行：工具调用直接执行，运行事件照常记录审计（无逐项确认）

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: PASS

## Step 2: Workspace/Coding 角色回归（tasks 6.2）

为 Story/Design/Work Item 三入口（共享 `workspace_engine`）及 Coding 角色补：
- Provider 选择（含 Pi）
- 权限模式（Auto 读配置；Claude/Codex 的 Supervised 保留）
- **fail-fast**：启动失败用 `start()` 返回 `Err` 构造（参照 `streaming_provider/mod.rs:278-295`）；运行中失败用 `ProviderEvent::Failed` 构造（参照 `src/web/test_controls/provider.rs:343-349`）。断言：失败即终态、`pi_start_count == 1`、其他 provider `start_count == 0`。

参照 reviewer 验证的注入点：`provider_registry.rs:21-44` 注册测试 provider；`tests/it_product/product_coding_workspace_engine/part_10.rs:371-383` 已有启动失败替身先例。

- [ ] Run: `cargo test -p cadence-aria workspace_engine product_coding_workspace_engine`
- Expected: PASS

## Step 3: 边界验证 —— 仓库初始化与 Task Runner 未被扩张（tasks 6.3）

- **仓库初始化**：确认添加代码库/初始化只用 Claude Code 专用选项，不含 Pi（测试锁定）。
- **Task Runner 拒绝 Pi（四层）**：
  - HTTP 入口：`parse_provider_type("pi")` 返回 `web_runtime_provider_type` 错误且文本含 `pi`（`provider_availability.rs:185-193`）
  - Router：`RoutingProviderAdapter` 拒 `ProviderType::Pi` 且 adapter 不调用（Task 1 Step 8）
  - 兼容性矩阵：`default_compatibility_matrix().entry_for(ProviderType::Pi)` 为 `None`
  - 节点契约：遍历 `default_node_contracts()`（或等价静态契约集合），断言每个 `provider_type != ProviderType::Pi`（**不是**源码文字扫描）

- [ ] Run: `cargo test -p cadence-aria provider_factory task_run provider_availability`
- Expected: PASS

## Step 4: 前后端质量检查 + 契约同步

```bash
# 后端
cargo test -p cadence-aria
cargo clippy -p cadence-aria --all-targets
cargo fmt --check
# 前端
cd web && npm test && npm run build && cd ..
```

按 `cadence/project-rules/build-test-commands.md` 标准命令执行（🔴 禁止 `-j 1`）。全部通过后勾选 `openspec/changes/add-pi-provider/tasks.md` 对应工作包。

- [ ] 全部质量检查通过。

## Step 5: Final Commit

- [ ] Run:

```bash
git add -A
git commit -m "test(pi): regression coverage for Pi protocol, workspace/coding roles, and Task Runner boundary"
```

---

## 完成检查（对应 tasks 6.1/6.2/6.3）

- [ ] 6.1：Pi 健康检查、目录展示、会话协议、取消、恢复和 Auto 运行的后端测试。
- [ ] 6.2：Story/Design/Work Item 三入口及 Coding 角色的 Provider、权限、fail-fast（启动/运行失败直接报告不切换）回归测试。
- [ ] 6.3：仓库初始化与 Task Runner 未被 Pi 扩张（含 HTTP 入口、router、兼容性矩阵、节点契约拒绝 Pi 的回归断言），前后端质量检查通过。
