# Task 4: Coding Workspace 默认 Auto + Pi 接入 + fail-fast（tasks 4.1, 4.2）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。依赖 Task 1（`ProviderName::Pi`）与 Task 2（`PiProvider` 注册）。

**Goal:** 把 Coding Workspace 各角色（Coder/Code Reviewer/Internal Reviewer）的新建默认权限模式改为 `Auto`（保留既有独立 `Supervised` 配置），并让三角色支持 Pi（Auto，fail-fast）。

**对应 spec requirement:**
- 「Provider 权限模式默认为 Auto，Pi 仅支持 Auto」（Coding 默认 Auto；Claude/Codex 保留 Supervised）
- 「Pi 在活跃 Provider 工作流中可发现且可选择」（Coding 选 Pi）
- 「所选 Provider 的失败直接报告且不切换」（fail-fast）

**背景：** Coding Workspace 已有 per-role 权限结构 `CodingRolePermissionModes`（`src/product/coding_models/provider_config.rs:17`），默认全 `Supervised`。本任务只改新建默认值为 `Auto`，不改 `CodingProviderPermissionMode` 类型，不动 Claude/Codex 的 Supervised 实现。

**Files:**
- Modify: `src/product/coding_models/provider_config.rs:23-31`（`CodingRolePermissionModes::default()` 全改 `Auto`）
- Modify: `src/product/coding_workspace_engine/`（三角色运行接 Pi；权限硬编码改读配置）
- Test: `src/product/coding_models/provider_config.rs`、`tests/it_product/product_coding_workspace_engine/`

**Interfaces:**
- Consumes: `CodingRolePermissionModes`、`CodingProviderPermissionMode`、`CodingRoleProviderConfigSnapshot`、`PiProvider`。
- Produces: `CodingRolePermissionModes::default()` 全 `Auto`；Coding 三角色可选 Pi。**不改 `CodingProviderPermissionMode` 类型。**

**约束（Decision 3）：** 只改**新建默认值**为 `Auto`；已持久化的显式 `Supervised` 值不受影响；旧记录缺 `permission_modes` 字段按新默认 `Auto` 反序列化（符合 Decision 3 与 Risks）。

---

## Step 1: 写失败测试 —— 新建默认 Auto + 显式值保留 + 旧记录缺字段 Auto

`src/product/coding_models/provider_config.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn coding_role_permission_modes_default_is_auto() {
    let modes = CodingRolePermissionModes::default();
    assert_eq!(modes.coder, CodingProviderPermissionMode::Auto);
    assert_eq!(modes.code_reviewer, CodingProviderPermissionMode::Auto);
    assert_eq!(modes.internal_reviewer, CodingProviderPermissionMode::Auto);
}

#[test]
fn explicit_supervised_value_preserved() {
    let json = serde_json::json!({
        "coder": "supervised", "code_reviewer": "supervised", "internal_reviewer": "supervised"
    });
    let modes: CodingRolePermissionModes = serde_json::from_value(json).unwrap();
    assert_eq!(modes.coder, CodingProviderPermissionMode::Supervised);
    assert_eq!(modes.code_reviewer, CodingProviderPermissionMode::Supervised);
}

#[test]
fn old_coding_snapshot_without_permission_modes_deserializes_to_auto() {
    // CodingRoleProviderConfigSnapshot.permission_modes 用 #[serde(default)]；缺字段按新默认 Auto
    let json = serde_json::json!({
        "coder": "claude_code", "code_reviewer": "codex",
        "internal_reviewer": "claude_code", "review_rounds": 1
    });
    let snapshot: CodingRoleProviderConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(snapshot.permission_modes.coder, CodingProviderPermissionMode::Auto);
}
```

- [ ] Run: `cargo test -p cadence-aria coding_models`
- Expected: FAIL —— 默认值是 `Supervised`

## Step 2: `CodingRolePermissionModes::default()` 改 Auto

`src/product/coding_models/provider_config.rs:23-31`：

```rust
impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Auto,
            internal_reviewer: CodingProviderPermissionMode::Auto,
        }
    }
}
```

⚠️ `CodingRoleProviderConfigSnapshot.permission_modes` 用 `#[serde(default)]`，改默认后旧记录缺字段按 `Auto` 反序列化——这正是契约要求，已由 Step 1 第三个测试覆盖。

- [ ] Run: `cargo test -p cadence-aria coding_models`
- Expected: PASS

## Step 3: 写失败测试 -- Pi+Supervised 规范化为 Auto + fail-fast（真实未实现行为）

注：Task 1/2 完成后，选 Pi 走 `PiProvider` 可能已通过 registry 路径执行，故"三角色可选 Pi"不作为 red test。本步聚焦 Task 4 真正未实现的行为：(a) Pi 角色的 Supervised mode 被规范化为 Auto；(b) Pi 运行失败不触发 Codex-only fresh retry。

`tests/it_product/product_coding_workspace_engine/` 参照现有 `execute_coding_*`（如 `part_04.rs`/`part_06.rs` 的 test harness 与 registry 注入方式），加测试：

```rust
#[tokio::test]
async fn pi_role_with_supervised_mode_normalized_to_auto() {
    // 构造 CodingRoleProviderConfigSnapshot：coder 选 Pi 且 permission_modes.coder = Supervised
    // 跑运行；断言实际 StreamingProviderInput.permission_mode == Auto（Pi 强制 Auto）
}

#[tokio::test]
async fn pi_failure_does_not_trigger_fresh_retry() {
    // recording Pi provider 运行失败
    // 断言：pi start_count == 1、其他 provider start_count == 0、终态失败
    // （Codex 的 resume-stall fresh retry 不对 Pi 触发）
}
```

注：需明确 `CodingRoleProviderConfigSnapshot` 构造、registry 注入路径、recording adapter 的 start_count 计数方式（参照现有 `execute_coding_*` 测试的 harness）。

- [ ] Run: `cargo test -p cadence-aria product_coding_workspace_engine`
- Expected: FAIL -- Pi+Supervised 未规范化；Pi 失败可能触发 fresh retry


## Step 4: Coding 运行链路接 Pi + fail-fast

`src/product/coding_workspace_engine/`：三角色运行经 registry 取 provider（Task 2 已注册 Pi）。确认 `StreamingProviderInput` 把 `permission_modes`（来自 `CodingRoleProviderConfigSnapshot`）传给 provider。

用 `rg -n "CodingProviderPermissionMode::Supervised" src/product/coding_workspace_engine/ -g '*.rs'` 定位权限硬编码，改为读 `CodingRolePermissionModes` 对应角色。

**fail-fast：** 用 `codegraph explore "coding_workspace_engine resume stall fresh retry 触发条件 coding.rs provider_stream.rs"` 确认 Codex resume-stall fresh retry（`coding.rs:184-206`、`provider_stream.rs:542-555`）是同 provider 内部重试还是换 provider：同 provider 内部 retry 保留；Pi 不实现该 retry（Pi 失败即终态）。测试断言 `pi_start_count == 1`。

- [ ] Run: `cargo test -p cadence-aria product_coding_workspace_engine`
- Expected: PASS

## Step 5: 全量受影响测试 + Commit

- [ ] Run:

```bash
cargo test -p cadence-aria coding_models
cargo test -p cadence-aria product_coding_workspace_engine
git add src/product/coding_models/provider_config.rs src/product/coding_workspace_engine/
git commit -m "feat(coding): default role permission modes to Auto and support Pi with fail-fast"
```

---

## 完成检查（对应 tasks 4.1/4.2）

- [ ] 4.1：Coding Workspace 各角色新建默认权限模式改为 `Auto`，保留独立 `Supervised` 配置。
- [ ] 4.2：Coder/Code Reviewer/Internal Reviewer 支持 Pi（Auto；失败直接报错，不做运行期降级）。
