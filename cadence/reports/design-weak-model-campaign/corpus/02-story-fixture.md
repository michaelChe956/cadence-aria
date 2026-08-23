# 多租户报表系统数据模型需求（上游 Story Spec fixture）

source id: issue_design_0002#报表定义
source id: issue_design_0002#生成任务
source id: issue_design_0002#租户隔离
source id: issue_design_0002#索引

## 用户故事

- [REQ-001] 作为租户管理员，我希望在报表系统内定义报表模板并触发生成任务，以便定期获得报表产物。
  - source id: issue_design_0002#报表定义

## 功能需求

- [REQ-002] 系统 SHALL 维护报表定义实体，含模板内容、参数 schema 与所属租户。
  - source id: issue_design_0002#报表定义
- [REQ-003] 系统 SHALL 维护生成任务实体，含任务状态、触发来源与产物引用，并关联其报表定义。
  - source id: issue_design_0002#生成任务
- [REQ-004] 所有报表相关实体 SHALL 含租户隔离字段，任何查询 SHALL 限定在单租户范围内。
  - source id: issue_design_0002#租户隔离

## 非功能需求

- [NFR-001] 高频查询路径（按租户列报表、查任务状态）SHALL 设计索引策略支撑，避免全表扫描。
  - source id: issue_design_0002#索引

## 成功标准

- [AC-001] 报表定义与生成任务实体关系完整，任务可追溯其报表定义与租户。覆盖 REQ-002、REQ-003。
  - source id: issue_design_0002#生成任务
- [AC-002] 跨租户访问被隔离字段与查询口径阻止。覆盖 REQ-004。
  - source id: issue_design_0002#租户隔离

## 范围

- 本 Story 只约定模型需求；实体关系、字段与索引策略的详细设计由下游 Design 产物（形态 02-data-model）完成，不做数据库迁移。
  - source id: issue_design_0002#报表定义
