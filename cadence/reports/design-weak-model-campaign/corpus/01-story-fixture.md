# 订单状态查询接口需求（上游 Story Spec fixture）

source id: issue_design_0001#接口
source id: issue_design_0001#分页
source id: issue_design_0001#错误码
source id: issue_design_0001#幂等

## 用户故事

- [REQ-001] 作为运营人员，我希望按订单状态筛选订单列表并分页浏览，以便快速定位待处理的订单。
  - source id: issue_design_0001#接口

## 功能需求

- [REQ-002] 系统 SHALL 提供按状态过滤订单列表的查询能力，状态为必填枚举参数。
  - source id: issue_design_0001#接口
- [REQ-003] 列表接口 SHALL 支持分页返回，包含页码与页大小参数，页大小有上限。
  - source id: issue_design_0001#分页
- [REQ-004] 非法状态枚举或越界分页参数 SHALL 返回可区分的错误码与错误信息。
  - source id: issue_design_0001#错误码
- [REQ-005] 相同参数的重复调用 SHALL 返回一致结果（幂等语义），不产生副作用。
  - source id: issue_design_0001#幂等

## 非功能需求

- [NFR-001] 接口响应时间在正常负载下 SHALL 满足现有订单查询服务的同级别 SLO。
  - source id: issue_design_0001#接口

## 成功标准

- [AC-001] 按合法状态查询返回订单摘要列表与分页元数据。覆盖 REQ-002、REQ-003。
  - source id: issue_design_0001#分页
- [AC-002] 非法状态或越界分页返回对应错误码且不产生数据变更。覆盖 REQ-004、REQ-005。
  - source id: issue_design_0001#错误码

## 范围

- 本 Story 只约定接口行为需求；接口字段与错误码的详细设计由下游 Design 产物（形态 01-api-design）完成。
  - source id: issue_design_0001#接口
