# Spec Delta

## MODIFIED Requirements

### Requirement: Coder 与 Work Item Code Reviewer 的技术失败自动重试有固定上限

Coder 与 Work Item Code Reviewer 的一次 Provider 执行周期 MUST 在首次调用之外最多自动重试2次，因此该cycle最多3次调用；本上限不重排已存在的恰一次coder教学窗口或group-review一次repair窗口最大组合。可自动transport重试的失败仅限非用户意图的Provider技术失败，包括启动错误、流提前关闭、连接或进程中断、Provider执行超时与可识别上游临时5xx错误；L1消费既有剩余机会而不扩池。

用户主动取消、权限或选择请求等待（包括等待该交互而超时）、正常 Reviewer finding、开放验证失败与 Plan Defect MUST NOT 作为transport技术失败自动重试。Provider正常完成后的结构化输出无效 MUST NOT 伪记技术失败；但服务端证实的L1确定性结构拒收 SHALL 按 workspace-structural-governance 沿原角色授权、teaching/repair/cycle剩余机会机器指令自动修复与全量复验，不要求用户写技术prompt。coder既有一次teaching窗口、每cycle最多3及group-review既有一次repair窗口均不增机会、不叠新池，共用耐久parent identity；权限/业务判断不自动放宽。

#### Scenario: Coder 因临时 503 自动恢复

- **WHEN** Coder 的首次调用因可识别的上游 503 失败
- **THEN** 系统 MUST 自动发起下一次 Coder 调用，且该失败 MUST NOT 增加 Coder rework 计数或立即创建人工门禁

#### Scenario: Reviewer 连续技术失败后转人工

- **WHEN** Work Item Code Reviewer 的首次调用及两次自动重试均因技术失败而未完成
- **THEN** 系统 MUST 停止自动重试，并落地既有 Reviewer 中断人工处置入口

#### Scenario: 正常完成但结构化输出无效不自动重试

- **WHEN** Reviewer 已正常完成调用，但其输出不符合结构化审查契约
- **THEN** 系统 MUST 保留真实结构拒收原因，服务端证实L1且有原授权剩余机会时携机器指令自动修复；无授权/机会或未知诊断则工程故障/原协议恢复，MUST NOT 将结果计为transport失败或新增独立池

#### Scenario: 用户取消不自动重试

- **WHEN** 用户在 Coder 或 Work Item Code Reviewer 运行中取消 attempt
- **THEN** 系统 MUST 停止当前调用，MUST NOT 发起自动重试


#### Scenario: coding教学与review修复不叠新池

- **WHEN** coder一次teaching机会或group-review一次repair机会已消费后又出现同类L1
- **THEN** 不另建L1调用池，不自动新run授权，不重置cycle/teaching/repair耐久身份；保既有最大组合，不改为统一三次总限，业务结论与自动rework仍按原合同
