# 设计：让 Kimi Code 也能承担聚合初始化准备阶段（root recipe）

> 本文件是 OpenSpec 层的**决策记录**：事实 / 方案 / 理由 / 代价 / 最小改动集合。全部落点已在本仓核对过行号与语义。
> **本 change 暂不实施**（owner 2026-11-05：先记录问题，后续另开 change）——本文与 `tasks.md` 是后续实施的完整依据。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`。

## 0. 事实（全部已核对）

| # | 事实 | 位置 |
|---|---|---|
K1 | Kimi 有三条启动路径：`start`（普通会话）/ `start_owned`（进程归属）/ `start_validated`（LC 策略会话）——**四家 adapter 都有这三条** | `kimi_code_provider/mod.rs:586/600/624`；对照 `claude_code_provider/mod.rs:842/853/867`、`pi_provider/mod.rs:985/1003/1037`、`codex_provider/mod.rs:758/772/1209` |
K2 | **root recipe 走 `start_validated`**（现场报错原文 `pi lc validated start: …` 即证） | `src/product/logical_codebase/provider_gateway.rs:178`（`is_root_recipe_phase`） |
K3 | Kimi `start_validated` 首步拒一切非 `ReviewerVerify` 的通用策略（含 marker），稳定码 `provider_generic_tool_policy_forbidden` | `kimi_code_provider/mod.rs:646-662` |
K4 | Kimi 的 marker 判定 `evaluate_bootstrap_executor_marker`：带 marker ⇒ 一律 Deny（理由「root recipe provider 固定为 Claude Code」）；**`#[cfg(test)]` only，生产无调用点** | `kimi_code_provider/client_services/policy.rs:108-137` |
K5 | 「root recipe provider 固定为 Claude Code」的原始出处是 **2026-11-02 `lc-root-initialization` change** 的旧契约（REQ-BOOT-03 原文「recipe provider SHALL 固定为 Claude Code」）；owner 2026-11-05 的 delta 已全文替换 ⇒ **K4 的理由是历史遗留** | `openspec/changes/archive/2026-11-02-lc-root-initialization/specs/non-interrupt-repository-bootstrap/spec.md:43` |
K6 | Kimi **无通用策略物理面**：argv 冻结 `["acp"]`，有测试锁死「不混入 `--exclude-tools`/`--disallowedTools`」；`kimi --help` 只有 `-y/--yolo`、`--auto`、`--plan`（审批档，非工具名单）；`kimi acp --help` 只有 `--login/--region`；`config.toml` 无工具 deny/allow 节；官方无 OS 沙箱/隔离边界（安全模型=审批门） | `kimi_code_provider/mod.rs:261-263`；`tests/tool_policy_zero_change.rs:1-6,75-79`；`cadence/notes/2026-10-16_调研-四CLI沙箱机制与业界实践.md:28-31` |
K7 | 三家的物理面：Claude `--disallowedTools Edit,Write,NotebookEdit`；Codex `sandbox=danger-full-access` + `approvalPolicy=on-request`；Pi `--exclude-tools edit,write` | `claude_code_provider/mod.rs:51`；`codex_provider/session.rs:46`；`pi_provider/mod.rs:50-52` |
K8 | **Pi 的黑名单也挡不住 bash 重定向 / sed 改文件**（残余风险，仓内笔记已承认）⇒ Kimi 放行后的强度与「Pi 无黑名单的 Coding 会话」同级，**不弱于已被接受的现状** | `cadence/notes/2026-10-16_分析-pi与kimi执行环境差异.md:13,65` |
K9 | root recipe 的真实兜底是 receipt / filesystem auditor（前后快照 + allowlist），**对四家同等生效** | REQ-BOOT-03 的 receipt auditor 合同 |
K10 | Kimi 已有「intent → grant 等价消费」先例（`ReviewerVerify`）：窗口引用进 grant，**不造通用策略物理面** | `client_services/policy.rs:43-70`（`from_reviewer_wire_contract` / `from_frozen_execution` / `duty_of_grant`） |
K11 | `ProviderPermissionMode` 只有 Auto（直决）/ Supervised（走 ApprovalBridge），是**审批档**不是工具能力面 | `client_services/policy.rs:283-288` |
K12 | 共享 marker digest 已就位（`82658d58`）：`src/cross_cutting/streaming_provider/bootstrap_marker.rs`；三家各有一份前缀 `lc-claude-…` / `lc-pi-…` / `lc-codex-…` | 同左 |
K13 | 三家的相位门形态（**Kimi 要照抄的样板**）：投影结构体带 `root_recipe_phase: bool` + `with_root_recipe_phase()`；`lc_tool_policy_canonical_digest(policy, root_recipe_phase)` 的 marker 分支「相位为真 ⇒ 共享 digest 放行；否则 fail-closed（文案同形）」；调用点接 `launch.is_root_recipe_phase()` | `pi_provider/projection.rs`；`codex_provider/projection.rs`；接线点 `pi_provider/mod.rs:1115/1300/1475`、`codex_provider/mod.rs:369/522/1297` |

## 决策 1：走「marker 通道放行 + 宿主侧授权等价表达」（方案 a）

**理由**：

1. K3/K4 两处拦路的**理由都是旧契约**（K5）⇒ 按现行契约改实现即可，**不改契约**。
2. Kimi 没有物理面（K6/K7）⇒ 通用策略无法用 CLI 开关表达 ⇒ 只能走它已有的授权通道（K10）。
3. 隔离强度不弱于现状（K8/K9）⇒ owner 接受。
4. K13 给出**逐字可抄的样板**（三家刚做完）⇒ 实现路径确定，无设计选型。

**代价**：Kimi 当自举执行器时**没有「禁用内置写工具」的物理面**，隔离来自宿主 fs / terminal handler 的 boundary plan + receipt auditor（与「Pi 无黑名单的 Coding 会话」同级）。

## 决策 2：普通会话零变化

无 marker 的普通会话（None / deny 意图）行为不变，仍走既有 ClientServicePolicy；`tests/tool_policy_zero_change.rs` 的「kimi 不读 tool_policy」锁需**定向放宽**（marker 意图被消费、其余仍零读），**不是**整体删除。

## 决策 3：不做 OS 级隔离（方案 c）

`ProviderBoundaryLauncher` 管不了 Kimi 自起的 ACP terminal（`cadence/notes/2026-10-16_分析-四Provider沙箱现状与路线.md:11`）⇒ 成本高、收益不确定 ⇒ 不绑进本 change。

## 最小改动集合（**不写实现代码**，实施时照此展开）

| # | 文件 | 改什么 |
|---|---|---|
1 | `src/cross_cutting/kimi_code_provider/mod.rs`（`start_validated`，:624-662） | 豁免集合从 `ReviewerVerify` 扩为 `ReviewerVerify \| BootstrapExecutorMarker(_)`；**marker 仅在 `launch.is_root_recipe_phase()` 为真时豁免** |
2 | `src/cross_cutting/kimi_code_provider/client_services/policy.rs`（:108-137） | `evaluate_bootstrap_executor_marker` 去掉 `#[cfg(test)]`；语义由「一律 Deny」改为**相位门**（root recipe 相位 + marker 合法 ⇒ 按 Coder/Executor 档允许写聚合根；否则 Deny）；并接入**生产**会话装配路径 |
3 | `src/cross_cutting/kimi_code_provider/projection.rs`（`lc_tool_policy_canonical_digest`，:194-228） | 给 `BootstrapExecutorMarker` 加分支，走共享 marker digest（Kimi 自己一份前缀 `lc-kimi-…`） |
4 | `src/cross_cutting/streaming_provider/bootstrap_marker.rs` | 确认共享 digest 支持 Kimi 前缀（与 `lc-claude-…`/`lc-pi-…`/`lc-codex-…` 同构） |
5 | `src/cross_cutting/kimi_code_provider/mod.rs`（投影构造处） | 接 `launch.is_root_recipe_phase()`（对照 `pi_provider/mod.rs:1115/1300/1475`） |
6 | `tests/tool_policy_zero_change.rs` | 「kimi 不读 tool_policy」锁**定向放宽**：marker 意图被消费，其余仍零读 |

## 验证要求（实施时）

- Pi / Codex / Claude 三家的既有断言**一条不许变**（尤其 Claude 的 marker digest 值逐字不变）
- Kimi 新增：root recipe 相位 + marker ⇒ 投影不拒且 digest 确定性/漂移敏感；**普通会话 + marker ⇒ 仍 fail-closed**（双层防线：投影层 + 策略层）
- `cargo test --locked --lib -- kimi_code_provider pi_provider codex_provider claude_code_provider` 全绿
- `cargo test --locked --lib -- provider_admission_preflight provider_gateway production_policy_resolvers` 全绿
- 行数门禁：`kimi_code_provider/**` 与 `streaming_provider/mod.rs` 逼近登记上限 ⇒ 新增行须先拆分或 owner 裁决
