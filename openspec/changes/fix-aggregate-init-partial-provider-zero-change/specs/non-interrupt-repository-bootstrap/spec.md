# Spec Delta

## MODIFIED Requirements

### Requirement: root recipe turn 零变更必须显式失败（REQ-BOOT-17）

root recipe 的四个 step（`PreCheck` / `RuleAndMcpConfig` / `OpenspecAndExamples`）中，任一 turn 结束时若**未产生任何工作区变更**（receipt 判为零动作），系统 SHALL 判该 step **失败**并保留可诊断失败事实；SHALL NOT 把该 turn 记为成功。

零变更失败的诊断 SHALL 同时携带**失败事实**（provider、step、零变更）与 **provider 的真实输出**（该 turn 的输出尾部，含 Cadence-skills 脚本的报告原文）。理由：零变更本身不说明原因——「命令未被识别」「命令执行了但失败」「命令按契约幂等无变更」三种情形的 provider 输出各不相同；诊断只有 `zero_change` 一句时，用户无法分辨是哪一种（本仓 2026-11-12 真机：pre-check 脚本某个 phase 失败 → `--no-interrupt` 中止 → 聚合根零落盘 → 界面只显示 `root recipe turn produced no workspace change (provider=pi, step=pre_check, zero_change=true)`，脚本的 JSON 报告整段丢失）。

provider 没有产出任何输出时，诊断 SHALL 只保留失败事实，MUST NOT 用无信息量的兜底串冒充真实输出。

provider turn 的输出超预算时，系统 SHALL 保留输出的**头部与尾部**（MUST NOT 只留头部）：脚本报告与失败原文都在收尾处，只留头部会把最有诊断价值的事实整段截掉。

**幂等重跑例外**：Cadence-skills 四条命令的契约就是幂等（脚本按契约报告 `all-skipped` / 0 created / 0 updated、产物无变化）。因此当工作区零变更**且**该 turn 的**工具结果**里出现脚本的成功报告（`overall=success`）时，系统 SHALL 判该 step **成功**——命令确实执行了、按契约没有产物变化。命令没被执行（没有脚本报告）或脚本报告失败（`overall=failed`）时，零变更 SHALL 仍判失败。脚本报告结论 MUST 只从**工具结果原文**读，MUST NOT 采信模型自述（模型可能复述或编造报告内容）。

本 requirement SHALL 复用既有 receipt auditor 的零变更判据，SHALL NOT 新增第二套「有无变更」的判定逻辑。

#### Scenario: 零变更的 recipe turn 判失败

- **WHEN** root recipe 的某个 step 结束时 receipt 判为零动作
- **THEN** 该 step SHALL 判为失败，operation SHALL 记录可诊断失败事实（provider、step、零变更）；SHALL NOT 把该 step 记为成功、SHALL NOT 继续推进后续 step

#### Scenario: 零变更但脚本报告成功判成功

- **WHEN** 某个 root recipe step 的 turn 结束时 receipt 判为零动作，且该 turn 的工具结果里出现脚本的成功报告（`overall=success`）
- **THEN** 该 step SHALL 判为成功（命令确实执行且按契约无产物变化），SHALL NOT 以零变更判失败

#### Scenario: 脚本报告失败仍判失败

- **WHEN** 某个 root recipe step 的 turn 结束时 receipt 判为零动作，且该 turn 的工具结果里出现脚本的失败报告（`overall=failed`）或没有脚本报告
- **THEN** 该 step SHALL 判为失败，诊断 SHALL 携带脚本报告原文

#### Scenario: 零变更诊断携带 provider 的真实输出

- **WHEN** 零变更失败的 turn 产出了输出（例如 Cadence-skills 脚本报告某个 phase 失败并给出原文）
- **THEN** 失败诊断 SHALL 同时包含零变更事实与该输出尾部，用户从诊断即可判断命令是否被执行、失败原因是什么；MUST NOT 只显示 `zero_change` 一句

#### Scenario: provider 无输出时诊断只保留失败事实

- **WHEN** 零变更失败的 turn 没有任何输出
- **THEN** 诊断 SHALL 只保留失败事实，MUST NOT 出现无信息量的兜底串

#### Scenario: 超长 turn 输出保留收尾

- **WHEN** provider turn 的输出长度超过诊断预算
- **THEN** 保留下来的输出 SHALL 同时包含头部与尾部，收尾（脚本报告 / 失败原文）MUST NOT 被整段截掉
