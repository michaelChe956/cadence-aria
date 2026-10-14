# Proposal: fix-local-delivery-and-chain-truth

## Why

owner 在无 `origin` 的多仓验收现场发现三类互相叠加的偏差：

1. 无远端仓库的交付被当作推送失败。`git ls-remote origin` 返回非零，runner 以错误退出，attempt 转 `awaiting_manual_recovery`（AMR）并卡在推送中途；重试会原样再死。现行 REQ-COD-06 与 REQ-MTG-04 把「已推送」写成唯一完成级别，没有「本地交付」这一合法终态。
2. 页面对链路状态说谎。AMR 的 attempt 下 unit 仍是 Running，plan 聚合进度因此给出 `execution_state="running"`，仪表盘也画成执行中；runner 死亡后遗留的 Running role run 永久显示「运行中」；多仓 plan 的 coding 页顶部只显示当前 attempt 一个分支和推送状态，看不到其他仓是否已交付。
3. Coder 的交付总结被环境差异叙事污染，且仓库 `AGENTS.md` / `CLAUDE.md` 的「不自动提交」类规则压过平台投影的提交责任，导致漏交。

三类偏差同一来源：契约只覆盖「有远端、一路顺利」的路径，失败与例外路径没有被规格化，页面于是按局部事实猜状态。本 change 把已由 owner 裁定、并经设计终稿确认的语义写成可验收契约。

## What Changes

- **完成级别增加本地交付。** 新增推送状态 `skipped_not_configured`。仅当目标仓库**未配置**交付远端时才跳过推送，持久化 ReviewRequest 并按完成收口；远端已配置但不可达、被拒、不可判定时行为不变，绝不伪装成跳过。完成级别改为「目标仓分支已推送，或未配置远端而本地交付，且 ReviewRequest 已生成」（修订 REQ-COD-06）。
- **已交付谓词统一。** `pushed` 或 `skipped_not_configured` 才算已交付，plan 级聚合与 issue 级交付聚合共用同一判定；本地交付必须与已推送在展示上可区分（修订 REQ-MTG-04）。
- **多仓逐仓交付面板。** work_item_group 且 plan 绑定 target-attempt ≥2 时，coding 页按 target 逐行呈现仓库、分支、worktree、attempt 状态、stage、提交、推送状态、ReviewRequest 与失败/阻塞原因；单仓与非 group 页面零变化。冻结 plan 聚合进度到前端的九字段 additive 契约（新增 REQ-MTG-06、REQ-MTG-07）。
- **投影真实性。** plan 聚合进度的 `execution_state` 以 owning attempt 的状态为守卫：只有 attempt 为 Running 才投影 `running`；AMR、需人工、失败、中止各有真实状态值，仪表盘把 AMR 画成失败。只改投影层，不改写 unit、`start_claim`、`active_unit_id` 等 durable 事实（新增 REQ-MTG-08、REQ-MTG-09、REQ-MTG-10）。
- **运行事实收尾与显式恢复入口。** runner 死亡转 AMR 时同批收尾遗留 Running role run；显式恢复时清理残留；AMR 在驾驶舱有可见的恢复操作，否则 AMR 是死胡同（新增 REQ-CRO-07、REQ-CRO-08）。
- **prompt 汇报契约与提交责任声明。** 平台在最终 prompt 末尾无条件追加：提交责任归 Coder 且优先于仓库「不自动提交」类规则、禁止手动 `git push`、交付摘要只含四类事实、环境异常只许写末尾一行「环境备注」（新增于 `coder-owned-work-item-commit`）。
- **平台确定性交付摘要。** Work Item 完成时由平台依据 durable 事实生成「交付事实（平台生成）」：完成提交、起止区间变更文件、已记录的验证证据、handoff 标识；与 Coder 自述分离，不能被叙事改写（新增于 `coder-owned-work-item-commit`，属可选后置包）。

## Capabilities

### New Capabilities

无；所有新增行为均并入既有 capability，不拆新 capability。

### Modified Capabilities

- `per-repo-coding-execution`：修订 REQ-COD-06 的完成级别与交付语义，加入 `skipped_not_configured` 本地交付。
- `multi-target-group-coding`：修订 REQ-MTG-04 的已交付谓词；新增 REQ-MTG-06（逐仓交付面板）、REQ-MTG-07（target 项九字段数据契约）、REQ-MTG-08（执行状态以 attempt 状态为守卫）、REQ-MTG-09（attempt 级执行状态取值）与 REQ-MTG-10（多 target 仪表盘按 attempt 状态覆盖）。
- `coding-run-operability`：新增 REQ-CRO-07（转入人工恢复时的运行事实收尾）与 REQ-CRO-08（显式恢复入口与残留清理）。
- `coder-owned-work-item-commit`：新增提交责任声明优先、交付汇报契约、平台确定性交付摘要三项 requirement。

## Non-Goals

- 不做自动创建 GitHub/GitLab PR，不做跨 Issue 或跨 plan 的交付编排。
- 不做「补配远端后自动补推」。`skipped_not_configured` 是终态，不提供重推操作；`failed` 的既有重推路径不变。
- 不改变「远端已配置」时的任何推送、重试、`Indeterminate` 停等语义，不把网络或鉴权失败降级为本地交付。
- 不改变 FinalConfirm 等完成门：已核实这些门不读取推送状态，本 change 只统一两处「已交付」判定。
- 不改写 unit、attempt 的底层状态机；投影真实性只发生在读取与展示层。
- 不改 `commit_responsibility` 的投影渲染本体、`renderer_version` 与执行上下文哈希；声明段在哈希计算之后追加。
- 不重做仪表盘或 coding 页整体布局；不新增聚合视图的操作面（REQ-MTG-04 红线保持）。
- 跨 attempt handoff 解析（WI-002 依赖另一 target-attempt 的 WI-001）是对现行 REQ-GCE-01 与 REQ-CS-03 契约的缺陷修复，不产生新契约，不在本 change 的 delta 内，走缺陷修复路径；本 change 的恢复路径假定它已先行合入。
- 不处理 `pointer_publication_coordinator` 在无远端时的推送行为：指针发布只认 `pushed`，其无远端语义另行立项。
- 不改远端名写死为 `origin` 的既有调用点；「未配置」按同一名称判定。

## Spec 时序与基线说明

- REQ-CS-06（plan 聚合进度）定义在未归档的 `fix-multi-repo-group-coding-start` change 的 `coding-start` capability，当前主规格尚无该 capability。因此 `execution_state` 的 attempt 状态守卫以 `multi-target-group-coding` 内独立的 ADDED requirement 表达，不对 REQ-CS-06 做 MODIFIED，避免依赖未归档基线。该 change 归档后，两者并存：REQ-CS-06 管等待原因文案与聚合结构，REQ-MTG-08、REQ-MTG-09 管 attempt 状态对 `execution_state` 的守卫与新增状态值。
- 实施已在本 change 之前按设计终稿并行启动（本地交付后端、投影与 DTO、role run 收尾、prompt 声明、前端类型与标签）。本 change 把已定语义落成契约，tasks 不预勾选；以契约为准复核已落地的未提交改动，偏差按失败关闭原则回改实现，不回改契约。

## Impact

- 领域模型与存储：推送状态枚举新增变体；issue 级与 plan 级交付聚合；ReviewRequest 的本地交付形态；attempt 转入 AMR 与显式恢复时的 role run 收尾。
- 服务端：评审请求阶段的推送决策；plan 聚合进度端点的 target 与 unit 投影；coding prompt 的最终组装；Work Item 完成处的平台摘要。
- 前端：推送状态与 attempt 状态类型及统一标签、逐仓交付面板、仪表盘六态映射、AMR 恢复操作、`DeliveryStatusPanel` 与 `PlanGroupProjectionPanel` 的交付判定。
- 兼容性：线上字段均为 additive；旧客户端遇到未知 `push_status` 或 `execution_state` 值须回落显示原文。**回滚风险**：一旦写入 `skipped_not_configured` 的 ReviewRequest，旧二进制反序列化该记录会失败，因此该能力按只进不退部署。
- 现场验收锚：两个 AMR 现场（WI-001 停在推送中途、WI-002 因跨 attempt handoff 死亡）在修复合入后经显式恢复可收敛到 FinalConfirm；恢复步骤见 `design.md`。
