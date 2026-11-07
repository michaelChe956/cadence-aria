## MODIFIED Requirements

### Requirement: root-recipe 相位的 provider 无关准入（REQ-BOOT-15）

root-recipe 相位的 provider 准入 SHALL **不要求** provider 是 Claude Code：**任何**通过普通 capability 准入（`Confirmed` + snapshot 一致）的 provider SHALL 能进入 root-recipe 相位，**含 `KimiCode`**。

准入的其余检查 SHALL **一条不减**，且 SHALL 继续 fail-closed：

1. 凭据重核验通道 SHALL 在场（缺失 ⇒ 拒绝，不放行）；
2. 已交付 recipe 证据的版本 SHALL 与记录版本钉定一致（漂移 ⇒ 拒绝）；
3. 凭据 SHALL 每次对**该逻辑代码库的** durable Running operation 重核验（operation 终态 / step 推进 / digest 或 root 漂移 ⇒ 拒绝）。

本 requirement MUST NOT 放宽普通会话的准入强度，也 MUST NOT 让 root recipe 的能力证据被普通会话复用（隔离契约不变）。

> **本 change 的作用**：移除 `root-recipe-provider-agnostic` 引入的 **KimiCode 例外**（owner 2026-11-05 裁定「先记录、后续另开 change 解决」）。实施方式见 `design.md` 决策 1（marker 通道放行 + 宿主侧受信岗位授权等价表达）。

#### Scenario: Kimi 进入聚合初始化候选

- **WHEN** 系统构建聚合初始化的 provider 候选序列
- **THEN** 候选序列 SHALL 包含 `KimiCode`；SHALL NOT 因其「结构上无法承载 root-recipe 相位」而把它排除

#### Scenario: Kimi 通过 root-recipe 准入

- **WHEN** 所选 provider 是 `KimiCode`，其普通 capability 准入为 `Confirmed`、凭据三道检查全部通过
- **THEN** root-recipe 相位 SHALL 放行；SHALL NOT 再以「Kimi 不是 bootstrap provider」为由拒绝

#### Scenario: 普通会话携带 marker 仍 fail-closed

- **WHEN** 非 root-recipe 相位的普通 coder 启动携带 bootstrap executor marker
- **THEN** 启动 SHALL 拒绝（fail-closed）；本 requirement SHALL NOT 放宽普通会话的策略门

#### Scenario: 隔离强度不被放宽

- **WHEN** Kimi 承载 root recipe 相位
- **THEN** 其实际写入范围 SHALL 受宿主 fs / terminal handler 的 boundary plan 与 receipt / filesystem auditor 约束（前后快照 + allowlist，与其它三家同等生效）；SHALL NOT 因缺少 CLI 物理禁用面而获得更宽的写面

#### Scenario: 非 Claude provider 通过 root-recipe 准入

- **WHEN** 所选 provider 不是 Claude Code，但其普通 capability 准入为 `Confirmed`、凭据三道检查全部通过
- **THEN** root-recipe 相位 SHALL 放行；SHALL NOT 再以「provider 不是 Claude Code」为由拒绝

#### Scenario: 不再出现固定 Claude 的拒绝

- **WHEN** root-recipe 相位的准入失败
- **THEN** 失败原因 SHALL NOT 是「provider 不是 Claude Code」或「Kimi 不是 bootstrap provider」这类已废止的固定化理由；SHALL 返回与真实原因相符的稳定码
