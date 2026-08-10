# Task 5: image-create 支持与边界排除（tasks 5.1, 5.2）

> 自包含任务文件。执行前请先读 `00-overview.md`。依赖 Task 1（枚举映射）、Task 2（streaming adapter）。

**Goal:** image-create 支持 Kimi（复用 streaming provider 会话，Task 1 已加 `From<ProviderName>` 映射，此 task 补前端 dropdown + 回归）；显式排除 Kimi 于 artifact retry / review repair。

**对应 spec requirement:**
- 「image-create 支持 Kimi」（含不可用禁用 Scenario）
- 「Kimi 复用既有授权与失败边界」（排除 structured-output repair / review repair）

**Files:**
- Verify/Modify: `src/product/image_create/models.rs:115`（Task 1 已加 `KimiCode => ProviderType::KimiCode`，此 task 验证）
- Modify: `src/product/workspace_engine/provider_drive.rs:113-118`（artifact retry 排除 Kimi，加注释）
- Modify: `src/product/workspace_engine/review/drive.rs:52-56`（review repair 排除 Kimi，加注释）
- Modify: `web/src/api/types/image-create.ts`（image-create dropdown 加 Kimi 选项）
- Test: `src/product/image_create/engine.rs`（脚本化 provider 跑通 Kimi 路径）

**Interfaces:**
- Consumes: `image_create` 复用 `StreamingProviderAdapter`（Task 2 已注册 Kimi）；`ProviderName::KimiCode`。
- Produces: image-create 可选/可执行 Kimi；artifact retry/review repair 跳过 Kimi。**Task 6-7 依赖。**

**参照：** Pi 在 `provider_drive.rs`/`review/drive.rs` 的排除逻辑（Kimi 同模式）；`image_create/engine.rs:415-934`（脚本化 provider 测试范式）。

---

## Step 1: image-create dropdown + 回归（失败测试先行）

`web/src/api/types/image-create.ts`：provider 选项加 Kimi（与 Claude/Codex/Pi 并列）。

`src/product/image_create/engine.rs` 测试加：
```rust
#[tokio::test]
async fn image_create_runs_with_kimi_provider() {
    // 用 ScriptedIterationProvider（provider_name: ProviderName::KimiCode）跑 image-create 路径
    // 断言：会话经 streaming provider 执行，provider_session_id 持久化
}
```

- [ ] Run: `cd web && pnpm test -- image-create` 与 `cargo test --locked --lib image_create`
- Expected: dropdown 含 Kimi；engine 跑通 Kimi；先 FAIL 后 PASS。

## Step 2: artifact retry / review repair 排除 Kimi

`src/product/workspace_engine/provider_drive.rs:113-118`：确认 artifact retry 的排除集合含 Kimi（与 Pi 同），加注释 `// 第一阶段不实证 Kimi resume 稳定性，排除 artifact retry（同 Pi）`。

`src/product/workspace_engine/review/drive.rs:52-56`：review repair 同样排除 Kimi，加注释。

测试：断言 Kimi 不触发 artifact retry / review repair（失败即终态）。

- [ ] Run: `cargo test --locked --lib provider_drive` 与 `cargo test --locked --lib review::drive`
- Expected: Kimi 被排除；先 FAIL 后 PASS。

## Step 3: 质量检查与提交

- [ ] Run: `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked` / `cd web && pnpm tsc -b && pnpm test`
- Expected: 全绿
- [ ] Commit:
```bash
git add -A
git commit -m "feat(kimi): Task 5 image-create 支持(前端dropdown+回归) + 排除 artifact retry/review repair(同 Pi)"
```
