# Tasks: kimi-root-recipe-support

> **状态：未实施**（owner 2026-11-05：Kimi 暂时不支持聚合初始化准备阶段，先记录问题，后续另开 change 解决 —— 本 change 即该 change）。
> 决策记录见同目录 `design.md`（事实表 + 最小改动集合）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。

## 0. 前置（实施前必须先做）

- [ ] 0.1 确认 `root-recipe-provider-agnostic` 已归档（它引入 REQ-BOOT-15 的 **Kimi 例外**措辞；本 change 移除该例外 ⇒ MUST 在其之后归档）
- [ ] 0.2 确认 `82658d58` 引入的共享 marker digest（`src/cross_cutting/streaming_provider/bootstrap_marker.rs`）已在 HEAD

## 1. 启动层放行 marker（root recipe 相位）

映射 requirement：REQ-BOOT-15

- [ ] 1.1 `src/cross_cutting/kimi_code_provider/mod.rs` 的 `start_validated`：通用策略豁免集合从 `ReviewerVerify` 扩为 `ReviewerVerify | BootstrapExecutorMarker(_)`；**marker 仅在 `launch.is_root_recipe_phase()` 为真时豁免**（普通 coder 启动携带 marker 仍须 fail-closed）
- [ ] 1.2 失败面文案沿用既有稳定码族（`provider_generic_tool_policy_forbidden` 等），**不新造码**
- [ ] 1.3 单测：root recipe 相位 + marker ⇒ 不被启动层拒；**普通会话 + marker ⇒ 仍拒**（双层防线）

## 2. 策略层 marker 相位门（进生产路径）

映射 requirement：REQ-BOOT-15

- [ ] 2.1 `src/cross_cutting/kimi_code_provider/client_services/policy.rs` 的 `evaluate_bootstrap_executor_marker`：去掉 `#[cfg(test)]`；语义由「一律 Deny」改为**相位门** —— root recipe 相位 + marker 合法 ⇒ 按 Coder/Executor 档允许写聚合根；否则 Deny
- [ ] 2.2 接入**生产**会话装配路径（当前该函数生产无调用点）
- [ ] 2.3 单测：root recipe 相位 ⇒ 允许写聚合根；普通会话 ⇒ 仍 Deny（策略层自身的双层防线）

## 3. 投影层 marker 分支 + digest

映射 requirement：REQ-BOOT-15

- [ ] 3.1 `src/cross_cutting/kimi_code_provider/projection.rs` 的 `lc_tool_policy_canonical_digest`：加 `BootstrapExecutorMarker` 分支，走**共享** marker digest（Kimi 自己一份前缀，形态对齐 `lc-claude-…`/`lc-pi-…`/`lc-codex-…`）
- [ ] 3.2 投影结构体加 `root_recipe_phase` 相位门 + `with_root_recipe_phase()`；调用点接 `launch.is_root_recipe_phase()`（对照 `pi_provider/mod.rs:1115/1300/1475`）
- [ ] 3.3 单测：digest 确定性 + 漂移敏感 + 相位无关性（deny / None 与相位无关）
- [ ] 3.4 **Pi / Codex / Claude 的既有断言一条不许变**（尤其 Claude 的 marker digest 值逐字不变）

## 4. 审计面与既有锁的定向放宽

- [ ] 4.1 Kimi 的 LC launch audit 沿用同一 sink（`ProviderStartAudit.lc_projection`），**不新增分区**
- [ ] 4.2 `tests/tool_policy_zero_change.rs` 的「kimi 不读 tool_policy」锁**定向放宽**：marker 意图被消费、其余仍零读（**不是**整体删除该断言）

## 5. 验证矩阵

- [ ] 5.1 `flock /tmp/aria-cargo.lock cargo test --locked --lib -- kimi_code_provider pi_provider codex_provider claude_code_provider` ⇒ 全绿
- [ ] 5.2 `flock /tmp/aria-cargo.lock cargo test --locked --lib -- provider_admission_preflight provider_gateway production_policy_resolvers` ⇒ 全绿
- [ ] 5.3 `cargo clippy --locked --all-targets -- -D warnings` ⇒ exit 0；`cargo fmt --check` ⇒ 干净
- [ ] 5.4 `openspec validate --specs --strict` ⇒ **仍 78 passed / 0 failed**
- [ ] 5.5 `openspec validate kimi-root-recipe-support --strict` ⇒ valid
- [ ] 5.6 行数门禁：`kimi_code_provider/**` 与 `streaming_provider/mod.rs` 逼近登记上限 ⇒ 新增行须先拆分或 owner 裁决
- [ ] 5.7 真机验收：owner 用 Kimi 跑一次聚合初始化 ⇒ 选中 Kimi ⇒ 真产生根级材料（`AGENTS.md` / `.claude/rules/` / `cadence/project-rules/`）

## 6. 与其它 change 的关系

- 本 change 与 `root-recipe-provider-agnostic` **归档有序**（后者先）；两者都 MODIFIED `non-interrupt-repository-bootstrap` 的 REQ-BOOT-15 ⇒ 顺序错会覆盖措辞
- 与 `globalize-provider-capability-verification` 的机器级化（capability 作用域）**无冲突**（不同文件面）
