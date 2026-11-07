# 提案：让 Kimi Code 也能承担聚合初始化准备阶段（root recipe）

> 状态：**草案 · 已记录，未实施**。owner 2026-11-05 裁定「Kimi 暂时不支持聚合初始化准备阶段，先记录问题，后续另开 change 解决」——本 change 就是那个「后续的 change」。
> 技术方案与决策记录见同目录 `design.md`；精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`。

## 背景

`root-recipe-provider-agnostic` change 已把「root recipe 相位对 provider 无关」写进契约（REQ-BOOT-15），并已在 Pi / Codex 两家落地（commit `82658d58`）。

**Kimi 是唯一缺口**，两处拦路：

| 拦路处 | 现状 | 理由（已溯源） |
|---|---|---|
启动层 `src/cross_cutting/kimi_code_provider/mod.rs:624-662`（`start_validated`） | 只要带了「通用策略」且 intent 不是 `ReviewerVerify` ⇒ **一律拒**（稳定码 `provider_generic_tool_policy_forbidden`）；marker 也在这里被拒 | 「Kimi 拒绝非空通用策略，继续既有 ClientServicePolicy」——**产品约束，非能力上限** |
策略层 `src/cross_cutting/kimi_code_provider/client_services/policy.rs:108-137`（`evaluate_bootstrap_executor_marker`） | 带 marker ⇒ **一律 Deny**（理由「root recipe provider 固定为 Claude Code」） | 引用的是 **2026-11-02 `lc-root-initialization` change 的旧契约**（当时 recipe provider 固定 Claude Code），该旧措辞已被 owner 2026-11-05 的 delta 全文替换 ⇒ **历史遗留**；且该函数目前是 `#[cfg(test)]` only，生产无调用点 |

## 为什么 Kimi 与其它三家不同

其它三家的「通用策略」有一个**物理面**（CLI 开关），Kimi 没有：

| provider | 通用策略物理面 | 位置 |
|---|---|---|
Claude | `--disallowedTools Edit,Write,NotebookEdit` | `claude_code_provider/mod.rs:51` |
Codex | `sandbox=danger-full-access` + `approvalPolicy=on-request` | `codex_provider/session.rs:46` |
Pi | `--exclude-tools edit,write` | `pi_provider/mod.rs:50-52` |
**Kimi** | **无**（CLI 只有 `-y/--yolo`、`--auto`、`--plan` 三个审批档；`acp` 子命令只有 `--login/--region`；config.toml 无工具名单节；官方无 OS 沙箱/隔离边界） | `kimi_code_provider/mod.rs:261-263`（argv 冻结为 `["acp"]`，有测试锁死「不混入 --exclude-tools/--disallowedTools」）；`cadence/notes/2026-10-16_调研-四CLI沙箱机制与业界实践.md:28-31` |

⇒ Kimi 只能用**宿主侧控制面**表达「自举执行器（有写权限）」——它**已经有这套机制**（`TrustedDutyGrant` / `ReviewerVerify` 窗口引用就是先例）。

## 目标

| # | 目标 |
|---|---|
1 | **Kimi 能在 root recipe 相位承载 marker**（与 Pi / Codex / Claude 同构） |
2 | **隔离强度不弱于已被接受的现状**：放行后的实际强度 =「Pi 那个没有黑名单的 Coding 会话」级别；真实兜底是 receipt / filesystem auditor（前后快照 + allowlist，对四家同等生效） |
3 | **普通会话零变化**：无 marker 的普通会话（None / deny 意图）行为不变，仍走既有 ClientServicePolicy |

## 非目标

- 不改 Kimi 的审批模型（`ProviderPermissionMode`：Auto 直决 / Supervised 走 ApprovalBridge）
- 不给 Kimi 上 Aria 自建 OS 隔离（bwrap / ProviderBoundaryLauncher）——`ProviderBoundaryLauncher` 管不了 Kimi 自起的 ACP terminal，成本高且不在本改动面
- 不改 provider 优先级顺序（顺序调整在 `root-recipe-provider-agnostic` 里做）
- 不改失败面文案表

## 方案选项与代价

| | 方案 | 隔离强度 | 代价 |
|---|---|---|---|
**a（owner 已选）** | **marker 通道放行 + 宿主侧授权等价表达**：启动层在 root recipe 相位放行 marker 到策略层；策略层由「一律 Deny」改为「相位门：root recipe 相位 + marker 合法 ⇒ 按 Coder/Executor 档允许写聚合根；否则 Deny」 | 与「Pi 无黑名单的 Coding 会话」同级；**不弱于现状** | 4~5 个文件（见 design.md） |
b | Kimi 只做非自举 provider（root recipe 排除 Kimi） | —— | 零代码，但**与 REQ-BOOT-15「任何 Confirmed provider SHALL 能进入 root-recipe 相位」冲突** ⇒ 需改契约，与 owner「实现侧对齐 spec」的方向相反 |
c | 给 Kimi 上 OS 级隔离 | 最强 | 成本高、且管不了 Kimi 自起的 ACP terminal ⇒ 不建议 |

## 范围

| 能力 | 变更 |
|---|---|
`non-interrupt-repository-bootstrap` | MODIFIED：REQ-BOOT-15 去掉「Kimi 例外」措辞，恢复「provider 无关」的完整语义 |

## 依赖与顺序

- MUST 在 `root-recipe-provider-agnostic` **之后**归档（后者引入 Kimi 例外，本 change 移除它）
- 依赖 `82658d58` 引入的共享 marker digest（`src/cross_cutting/streaming_provider/bootstrap_marker.rs`）

## owner 决策记录（2026-11-05）

| # | 决策 | 结论 |
|---|---|---|
1 | 排期 | **暂时不支持**，先记录问题，后续另开 change 解决 ⇒ 本 change 即该 change |
2 | 实施路线 | **方案 a**（marker 通道放行 + 宿主侧授权等价表达） |
3 | 隔离强度 | 接受「与 Pi 无黑名单会话同级」；真实兜底是 receipt / filesystem auditor |
