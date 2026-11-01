# Spec Delta

> 修订基线：`openspec/specs/per-repo-coding-execution/spec.md` 现行 REQ-COD-06（2026-08-05 归档版本）。以下为完整替换文本，保留原有两个场景并扩展。

## MODIFIED Requirements

### Requirement: 每仓独立交付（推分支 + 人工评审）（REQ-COD-06）
系统 SHALL 使每仓独立 branch/commit 并持久化 `ReviewRequest`（GitBranchOnly，含分支/推送状态，无外部 PR URL）。目标仓库已配置交付远端时 SHALL 推送该分支；目标仓库**未配置**交付远端时，系统 SHALL 跳过推送并以本地交付收口，推送状态记为 `skipped_not_configured`。「未配置」的唯一判据是该仓库不存在名为交付远端的远端配置，MUST NOT 以远端不可达、鉴权失败、被拒或无法判定远端状态代替；这些情形 SHALL 保持既有语义（推送被拒记为失败，无法判定则停等人工恢复），MUST NOT 降级为本地交付。

Issue 完成 = 所有必需 Work Item 完成，完成级别为「目标仓分支已推送，或目标仓未配置远端而本地交付，且 ReviewRequest 已生成」；partial failure 显式呈现，不伪装全局成功；自动创建 GitHub/GitLab PR 不在本 change 范围。本地交付 SHALL 在 API 与界面上与「已推送」可区分，MUST NOT 呈现为已推送。Issue 级交付聚合的既有取值 `all_pushed` 名称保持不变，语义为全部必需 Work Item 已交付（含本地交付），不新增取值。

#### Scenario: 多仓 Issue 各仓交付
- **WHEN** 多仓 Issue 各仓 coding 完成
- **THEN** 各仓 SHALL 产出独立分支/提交/推送（或本地交付）与 ReviewRequest 状态；某仓失败时显式呈现 partial failure，不标记 Issue 为已完成

#### Scenario: 交付完成级别
- **WHEN** 判定 Work Item/Issue 完成
- **THEN** 以「目标仓分支已推送，或未配置远端而本地交付，且 ReviewRequest 已生成」为完成级别；未推送、推送失败的 item 不得视为已交付

#### Scenario: 未配置远端的仓库按本地交付收口
- **WHEN** 某目标仓未配置交付远端，其 Work Item 的 coding 与评审通过后进入评审请求阶段
- **THEN** 系统 SHALL 不尝试推送、持久化 ReviewRequest（推送状态 `skipped_not_configured`，无推送错误），其提示 SHALL 说明为本地分支与提交且未配置远端，MUST NOT 声称基于远端发起审查；评审请求阶段 SHALL 标记为完成而非失败，attempt 继续进入后续阶段门

#### Scenario: 已配置远端的失败不降级为本地交付
- **WHEN** 目标仓已配置交付远端，但推送被拒、网络或鉴权失败，或远端状态无法判定
- **THEN** 系统 SHALL 保持既有语义：被拒记为推送失败并保留重推路径，无法判定则停等人工恢复；MUST NOT 记为 `skipped_not_configured`，该 item 不得视为已交付

#### Scenario: 推送中途中断的无远端仓库可收敛
- **WHEN** 某未配置远端的目标仓其评审请求操作已记录「推送已开始」后因进程中断而中止，随后经显式恢复重新进入该阶段
- **THEN** 系统 SHALL 在探测远端状态或推送之前先判定远端未配置，并收敛为本地交付，MUST NOT 因探测一个不存在的远端而再次失败

#### Scenario: 本地交付不自动补推
- **WHEN** 某 Work Item 已按本地交付收口，之后该仓库补配了远端
- **THEN** 系统 MUST NOT 自动推送，也 MUST NOT 重开该 ReviewRequest；既有重推操作仅对推送状态为失败的记录开放

#### Scenario: 本地交付计入 Issue 完成
- **WHEN** Issue 下所有必需 Work Item 均为 Completed，且各自最新 ReviewRequest 的推送状态为 `pushed` 或 `skipped_not_configured`
- **THEN** 交付聚合 SHALL 为全部已交付（`all_pushed`），Issue 可标记完成；其中本地交付的 Work Item 在交付面板中 SHALL 以「本地交付(无远端)」区别于「已推送」
