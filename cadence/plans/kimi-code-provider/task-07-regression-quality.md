# Task 7: 回归验证与质量门禁（tasks 7.1, 7.2, 7.3）

> 自包含任务文件。执行前请先读 `00-overview.md`。依赖 Task 1-6 全部完成。

**Goal:** 集中补齐跨任务的回归测试矩阵（health/registry/gate/DTO/task-run 拒绝/凭证缺失/审批/提问/resume/取消/超时），前端测试，并执行完整质量门禁。

**对应 spec requirement:** 全部 Requirement 的 Scenario 回归覆盖。

**Files:**
- Verify/Augment: `src/cross_cutting/provider_health.rs`、`provider_registry.rs`、`provider_availability_gate.rs`、`src/web/handlers/providers.rs`、`src/task_run/*`、`src/cross_cutting/kimi_code_provider/tests.rs`（前序 task 已写大部分，此 task 补漏与交叉）
- Verify/Augment: 前端各 `.test.tsx`
- Run: 全套质量门禁命令

**Interfaces:** 无新产出；回归与门禁。

---

## Step 1: 后端回归矩阵（查漏补缺）

逐项确认有测试覆盖（前序 task 已写则验证，缺则补）：
- provider_health：Kimi 可用 / 缺失 / 超时 / 版本过低(<0.34.0) / snapshot 持久化 / 并行 probe 含 Kimi。
- provider_registry：stable order 含 Kimi（Pi 后 Fake 前）/ available_names / executable_names。
- provider_availability_gate：Kimi health entry 存在→放行 / 缺失→阻断。
- handlers/providers：DTO 返回 kimi_code / display_name="Kimi Code" / install_hint 正确。
- task-run 四入口：Kimi 返回 incompatible（factory）/ 稳定文本（step_runner）/ None（runtime provider）/ 稳定空（utils），**不 panic**。
- **凭证缺失运行错误映射**：模拟 ACP 认证错误（401/unauthorized）/ stderr 未登录 → 清晰运行错误（提示 `kimi login`）；脱敏不回显 token/config。
- kimi_code_provider：审批（approve/reject）、提问（AskUserQuestion select/free_text/多问题串行）、resume（load 成功/失败）、取消（session/cancel+Aborted）、超时、终态不双发、协议降级（未知 method/notification/option kind）。

- [ ] Run: `cargo test --locked --lib provider_health provider_registry provider_availability handlers::providers task_run kimi_code_provider`
- Expected: 全 PASS

## Step 2: 前端回归矩阵

- provider-options：catalog 含 kimi_code / order 正确 / 可用/不可用/禁用。
- ProviderConfigPanel + CodingProviderConfigPanel：Kimi 显示 Auto+Supervised。
- CreateRepositoryDialog：Kimi 被过滤（不出现）。
- WebSocket parser、Chat fallback、image-create dropdown、fixture union 含 kimi_code。

- [ ] Run: `cd web && pnpm test`
- Expected: 全 PASS

## Step 3: 完整质量门禁

- [ ] Run: `cargo fmt --check`
- Expected: 无 diff
- [ ] Run: `cargo clippy --all-targets --all-features --locked -- -D warnings`
- Expected: 无 warning
- [ ] Run: `cargo test --locked`
- Expected: 全 PASS
- [ ] Run: `cd web && pnpm tsc -b`
- Expected: 无类型错误
- [ ] Run: `cd web && pnpm test`
- Expected: 全 PASS

## Step 4: spec 覆盖自检 + 提交

逐条对照 `openspec/changes/add-kimi-code-provider/specs/kimi-code-provider-integration/spec.md` 的 9 个 Requirement × 各 Scenario，确认每个都有对应实现/测试。

- [ ] Commit:
```bash
git add -A
git commit -m "test(kimi): Task 7 回归矩阵与质量门禁（health/registry/gate/DTO/task-run拒绝/凭证/审批/提问/resume/取消/超时/前端）"
```

## 完工后

- 所有 task 完成后，按 OpenSpec 流程：契约（proposal/design/spec）已是获批状态，可进入 `requesting-code-review` → 审查通过后 archive change + sync 主 spec（见 `finishing-a-development-branch`）。
- 单独任务（非本 change）：统一 Claude/Pi 的 ChoiceResponse 优先级（B2 副产品发现）。
