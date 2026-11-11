# 规范增量：教学重驱资格 typed 化、图奇偶性、环来源、尝试可观测

<!-- 来源：owner 2026-10-31「不可控性太高」。设计：cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v1.0.md。
     本 delta 不新增 attempt：REQ-SG-05 的 attempt 单权威与上限不变，只改「谁有资格使用那恰一次」。 -->
<!-- 2026-11-09 补：按 owner 裁决（2026-11-08 §1.4/§1.5；处置裁决 §2.2 序 1）扩充资格清单——四类 typed IR 诊断与确定性结构拒收；
     标题与主规格逐字一致（不改主规格标题）。producer 落点与载荷细则见本 change design.md「MODIFIED REQ-WSC-09 的 typed IR 资格扩充」。 -->

## MODIFIED Requirements

### Requirement: generate 段编译语法失败的教学重驱（REQ-WSC-09）

SC 候选编译（generate 段）失败时，系统 SHALL 在会话进入终态前提供恰好一次教学重驱，该机会以前置授权、合法 provider 与剩余机会为条件，混合 operational 集合 MUST NOT 获驱。资格 MUST 由 typed 判据决定：parse 语法类、事实完整的 typed lowering fault、四类 typed IR 诊断（unknown_done_when_ref、unknown_reviewer_check_ref、acceptance_criterion_without_reviewer_check、unknown_requirement_ref）与确定性结构拒收；数据库/基线/成员解析失败为 Operational 不获资格；新增类别 MUST 显式进入规格/typed 白名单。资格 MUST NOT 取决于 message 措辞；重驱 prompt MUST 渲染 typed 事实，仍失败按既有失败路径终态。每 candidate 总额外驱动 ≤1，既有上限不变。

#### Scenario: invalid_ears 触发一次教学重驱

- **WHEN** 候选编译产出 invalid_ears 且为该 candidate 首次编译失败
- **THEN** 系统以行号+错误信息回灌 author 重驱一次；重驱交付通过编译则继续既有链路，仍失败则终态（不再二次重驱）

#### Scenario: 事实完整的 lowering fault 获得一次重驱

- **WHEN** 候选因携带字段名与期望形态的 typed lowering fault 编译失败，且为首次编译失败
- **THEN** 系统 MUST 以 typed 事实回灌 author 重驱一次，MUST NOT 立即终态

#### Scenario: 无 typed fault 的 lowering 失败仍不重驱

- **WHEN** lowering 诊断不携带 typed fault，仅 message 含「缺少」等措辞
- **THEN** 系统 MUST NOT 授予重驱资格，按既有终态路径处理

#### Scenario: 重驱上限不被突破

- **WHEN** 同一 candidate 的编译失败类别先后命中两个可教学类别
- **THEN** 教学重驱总共仍至多一次，第二次失败直接按既有失败路径处理

#### Scenario: parse 语法类按码表获驱

- **WHEN** 候选编译产出 missing_section、invalid_ears、invalid_work_item_id 或 unknown_structured_key，且为该 candidate 首次编译失败
- **THEN** 系统按既有 parse 教学链路以行号与错误信息回灌 author 重驱一次

#### Scenario: 四类 typed IR 诊断获得重驱且载荷完整

- **WHEN** 候选产出 `unknown_done_when_ref`、`unknown_reviewer_check_ref`、`acceptance_criterion_without_reviewer_check` 或 `unknown_requirement_ref` 之一，且每条携带 Work Item、task/check/criterion 定位、错误 ref、本 WI 合法集合或真实 spec 集合、原行（无行即「不可定位」）与所需字段形态
- **THEN** 系统 MUST 以该 typed 载荷回灌 author 重驱一次；载荷不全或无原行时 MUST NOT 授予资格

#### Scenario: operational 失败混入不获重驱

- **WHEN** 失败集合含数据库、基线或成员解析等 Operational 失败，另含可教类别
- **THEN** 系统 MUST NOT 以 `any(可教)` 放行，MUST NOT 授予重驱，按既有 Operational 路径处理

#### Scenario: 未入白名单的类别不自动获权

- **WHEN** 某失败 code 带可教前缀或 message 含「缺少」措辞，但未显式进入规格/typed 白名单且事实不完整
- **THEN** 系统 MUST NOT 授予重驱资格，按既有失败路径处理

<!-- 2026-07-31 owner 裁定「复杂内容先不做」：**本 change 只保留未交付条款**（四层处置 / typed lowering fault / 目标分支等价规范化 / 精确缺口 / prompt 体量闸门 / 教学重驱资格改为 typed 完备）。
     本轮已交付条款已拆到 `fix-work-item-group-plan-parity` 归档：判定一致（REQ-WSC-23 ≡ REQ-WG-01，重复已删）、环诊断带来源（REQ-WSC-24）、尝试可序列观测（REQ-WSC-25）、owner 回显与一次答完（REQ-CG-08/09）、反查单一来源（REQ-WG-02）、可执行选项（REQ-WG-05）、修复指令携带选项与保护项（REQ-WSC-06 ≡ REQ-WG-06）、取值来源按会话类型唯一（REQ-WG-08）。 -->

## ADDED Requirements
