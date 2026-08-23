# 订单结算抽象需求（上游 Story Spec fixture）

source id: issue_design_0004#计算策略
source id: issue_design_0004#舍入规则
source id: issue_design_0004#抽象

## 用户故事

- [REQ-001] 作为财务系统对接方，我希望订单结算有明确的计算策略抽象，使常规结算与部分退款结算可插拔替换。
  - source id: issue_design_0004#计算策略

## 功能需求

- [REQ-002] 系统 SHALL 定义结算计算策略抽象，至少覆盖常规结算与部分退款结算两种实现口径。
  - source id: issue_design_0004#计算策略
- [REQ-003] 金额计算 SHALL 使用分单位整数并采用银行家舍入，中间过程不得出现浮点累计误差。
  - source id: issue_design_0004#舍入规则
- [REQ-004] 结算设计中的每条决策 SHALL 追溯到本 Story 的 REQ/AC 引用，dec_req_links 完整可查。
  - source id: issue_design_0004#抽象

## 非功能需求

- [NFR-001] 结算抽象 SHALL 不引入对具体支付渠道 SDK 的编译期依赖。
  - source id: issue_design_0004#抽象

## 成功标准

- [AC-001] 两种结算策略在同一抽象下可替换且行为口径清晰。覆盖 REQ-002。
  - source id: issue_design_0004#计算策略
- [AC-002] 舍入规则在结算路径上被一致应用（分单位、银行家舍入）。覆盖 REQ-003。
  - source id: issue_design_0004#舍入规则
- [AC-003] 设计产物中每个 DEC 的来源 REQ/AC 引用不丢失。覆盖 REQ-004。
  - source id: issue_design_0004#抽象

## 范围

- 本 Story 只约定抽象需求；策略结构与舍入细则由下游 Design 产物（形态 04-abstract-traceability）设计，不写测试内容。
  - source id: issue_design_0004#抽象
