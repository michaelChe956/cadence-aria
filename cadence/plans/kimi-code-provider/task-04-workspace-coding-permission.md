# Task 4: Workspace/Coding 接入与 Renderer（tasks 3.1, 3.2, 4.1, 4.2）

> 自包含任务文件。执行前请先读 `00-overview.md`。依赖 Task 1（枚举）与 Task 2/3（provider 能力）。

**Goal:** 普通 Workspace（Author/Reviewer）与 Coding Workspace（Coder/Code Reviewer/Internal Reviewer）接入 Kimi：默认 Auto、可切 Supervised（**不**像 Pi 强制 Auto-only）；新增 `render/kimi_code.rs` renderer；前后端 interaction guidance 一致。

**对应 spec requirement:**
- 「Kimi 权限模式默认 Auto 且支持 Supervised」
- 「Kimi 支持 AskUserQuestion 结构化提问」（guidance 声明 Kimi 支持 structured permission/choice）

**Files:**
- Modify: `src/product/coding_models/provider_config.rs:24-29`（`CodingRolePermissionModes::default` 全 Auto 既有；确认 Kimi 不被强制 Auto-only）
- Modify: `src/product/workspace_engine/mappings.rs`（Kimi **不**像 Pi 强制 Auto；保留用户选择的 Supervised）
- Modify: `src/web/workspace_context/prompts.rs`（Kimi interaction guidance：支持 structured permission request + AskUserQuestion choice；与前端一致）
- Modify: `web/src/state/workspace-ws-store-guidance.ts`（同上，前端 guidance，**与 Rust 端声明一致**——避免 Pi 那种 Rust/前端自相矛盾）
- Modify: `src/product/work_item_projection/render/kimi_code.rs`（补全 Task 1 占位 `KimiCodeProjectionRenderer` 的完整 profile：Supervised tool hint、structured-output wrapper、renderer version；仿 `render/pi.rs`）
- Verify: `src/product/work_item_projection/render.rs`（Task 1 已接入 `renderer_for` + mod 声明，此 task 仅验证）

**Interfaces:**
- Consumes（来自 Task 1/2/3）：`ProviderName::KimiCode`、Kimi provider 能力（Auto+Supervised、AskUserQuestion）。
- Produces: Kimi 在普通/Coding Workspace 可选可执行；`renderer_for(&ProviderName::KimiCode)` 返回 `KimiCodeRenderer`；前后端 guidance 含 Kimi 一致声明。**Task 5-7 依赖。**

**参照：** `render/pi.rs`（renderer 模板）、`workspace_engine/mappings.rs`（Pi 强制 Auto 的位置——Kimi 此处**不改**强制逻辑）、`workspace_context/prompts.rs` 与 `workspace-ws-store-guidance.ts`（Pi 的 guidance——注意其自相矛盾，Kimi 要避免）。

---

## Step 1: 权限配置——Kimi 不强制 Auto（失败测试先行）

`src/product/workspace_engine/mappings.rs` 测试（若 Pi 有"强制 Auto"测试，Kimi 加对反测试）：
```rust
#[test]
fn kimi_supervised_mode_is_preserved_not_forced_auto() {
    // 用户为 Kimi 选 Supervised → 映射结果保留 Supervised（不强制 Auto）
    // 对照：Pi 选 Supervised 会被强制 Auto（既有逻辑不动）
}
```

- [ ] Run: `cargo test --locked --lib kimi_supervised_mode`
- Expected: FAIL（若 Kimi 被误纳入 Pi 的强制分支）或 PASS（若已正确）

实现：确认 `mappings.rs` 的"强制 Auto"分支只匹配 `ProviderName::Pi`，**不含** KimiCode。Kimi 保留用户选择的权限模式。Coding 侧 `coding_models/provider_config.rs` 默认 Auto 不变，Kimi 角色可切 Supervised。

- [ ] Run: `cargo test --locked --lib kimi_supervised_mode`
- Expected: PASS

## Step 2: 前后端 interaction guidance 一致

`src/web/workspace_context/prompts.rs`：为 Kimi 加 guidance 声明——
- Kimi 支持 structured permission request（Supervised 工具审批）。
- Kimi 支持 AskUserQuestion 结构化提问（用户可选项或自由输入）。
- 与前端 `workspace-ws-store-guidance.ts` **逐字对齐**（避免 Pi 那种 Rust 说"可用 ask_user 继续"、前端说"只能 text fallback"的矛盾）。

`web/src/state/workspace-ws-store-guidance.ts`：加 Kimi 条目，声明与 Rust 端一致的能力（支持 permission + choice）。

- [ ] Run: `cargo test --locked --lib workspace_context` 与 `cd web && pnpm test -- workspace-ws-store-guidance`（若有测试）
- Expected: guidance 含 Kimi；两端一致。

## Step 3: 补全 `render/kimi_code.rs` renderer profile（失败测试先行）

`src/product/work_item_projection/render/kimi_code.rs`，把 Task 1 的占位 `KimiCodeProjectionRenderer` 补全为完整 profile（仿 `render/pi.rs`）：
```rust
// 补全 impl ProviderProjectionRenderer for KimiCodeProjectionRenderer
// label "Kimi Code"、renderer version、Supervised tool hint（支持逐工具审批）、
// structured-output wrapper
```

测试：golden output——KimiCode renderer 产出的 profile 含正确 label/version/tool hint。

- [ ] Run: `cargo test --locked --lib work_item_projection::render`
- Expected: KimiCodeRenderer golden 正确；先 FAIL 后 PASS。

## Step 4: 质量检查与提交

- [ ] Run: `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked` / `cd web && pnpm tsc -b`
- Expected: 全绿
- [ ] Commit:
```bash
git add -A
git commit -m "feat(kimi): Task 4 Workspace/Coding 接入(默认Auto可切Supervised/不强制Auto-only) + render/kimi_code.rs + 前后端 guidance 一致"
```
