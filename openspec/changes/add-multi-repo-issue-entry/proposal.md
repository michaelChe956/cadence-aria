# Proposal: add-multi-repo-issue-entry

## Why

LC(Logical Codebase)多成员场景下,一个业务功能天然横跨多个成员仓(如「前端+网关+API+业务」四层)。当前产品在该形态下不可用:

1. **建 Issue 入口单选**:前端 `CreateLifecycleIssueDialog` 只能选一个仓;后端 `IssueSelection.focus_repository_ids` 复数字段与校验(focus⊆include)早已存在,但前端无写入点——多仓 issue 没有点击路径(用户 2026-09-30 C7 裁决登记的缺口①)。
2. **plan preflight 单仓闸**:work item plan 的确定性 preflight 要求 design involved **恰好一仓**,多仓 design 确定性失败(SC spec REQ-WSC-08 场景「多仓 preflight 失败收敛」)。
3. 而下游**多 target 组编码引擎已归档验收**(REQ-MTG-01..05:按 target 分流/异仓并行/聚合终态/审计),plan IR 天然多仓(`items: Vec<PlanCandidateItemIr>` 每项自带 `target_repository_id`)——**只差入口的闸**。

本 change 补齐入口两块,使「一个 issue、一份跨仓 plan、若干单仓 work item、人工逐 target 启动(MTG 一期合规路径)」的完整链路可用;它是页面 E2E(四层四仓案例)的产品前置。

## What Changes

- **建 Issue UI 多选**:成员列表改复选(单仓仓库路径行为不变);LC 路径将勾选集写入 `focus_repository_ids`(单选仍兼容:勾 1 个=现行为)。
- **钉定语义(上界)**:勾选集=硬上界;design involved 由 AI 在界内收敛(非空真子集或全集),sentinel 不得输出界外仓;越界诉求只能走修订反馈人工裁决;change_order/plan items target ⊆ involved。
- **preflight 放开**:`恰好一仓` → `involved ⊆ focus 且非空`;SC 事务状态机零改动;spec delta 修订「多仓必拒」场景语义。
- **非目标**:自动 fan-out(MTG 二期 defer 维持);issue↔plan 基数不变(仍一个 plan);单仓流为零回归。

## Impact

- 受影响 spec:`work-item-plan-single-candidate`(REQ-WSC-08 多仓场景语义修订)、issue 入口/lifecycle 相关 spec(若单选语义有 spec 面)。
- 代码面:前端对话框+提交链、`preflight_single_repository_candidate`、design involved 钉定注入(镜像 e78a97c6 单仓钉定的多仓版)、相关测试。
- 下游零改动:split 引擎/组编码(MTG)/cross_target 门按 items 的 target 分流,现状即支持。
- 验收:单测+集成钉新语义;**页面 E2E 四仓流程即终验**(另行执行,不在本 change 内)。
