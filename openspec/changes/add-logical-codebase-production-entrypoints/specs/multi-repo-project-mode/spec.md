## Purpose

project 级多仓库模式开关与 legacy 端点防护，保证单仓 project 行为零变化的前提下为多仓链路提供生产启用路径。

## ADDED Requirements

### Requirement: 多仓库模式 opt-in

创建 project 时必须可选择启用多仓库模式；创建后不可切换。

#### Scenario: 创建多仓 project
- **WHEN** 创建 project 请求携带 multi_repo=true
- **THEN** project 持久化 multi_repo 标志；仓库存储层按项目启用逻辑代码库 feature；多仓链路端点可用

#### Scenario: 默认单仓不变
- **WHEN** 创建 project 不携带 multi_repo（或 false），或读取旧版本 project 数据
- **THEN** project 为单仓模式，全部既有单仓行为保持不变

### Requirement: 多仓 project 的 legacy 仓库端点防护

多仓 project 下传统仓库 CRUD 端点必须受限，避免绕过登记链产生第二套成员来源。

#### Scenario: 多仓下调 legacy 创建/删除被拒
- **WHEN** 多仓 project 调用 POST/DELETE /api/projects/{pid}/repositories 或 GET /api/projects/{pid}/repository-initializations/{operation_id}
- **THEN** 返回 409 稳定错误码 legacy_repository_endpoint_on_multi_repo，响应体指引使用登记端点

#### Scenario: 多仓下 legacy 列表投影
- **WHEN** 多仓 project 调用 GET /api/projects/{pid}/repositories
- **THEN** 返回逻辑成员的兼容投影（不暴露物理登记写路径）

### Requirement: 单仓 project 调多仓端点被拒

单仓 project 调用多仓专属端点时必须返回稳定错误码拒绝。

#### Scenario: 单仓误调登记
- **WHEN** 单仓 project 调用登记或聚合索引端点
- **THEN** 返回 409 稳定错误码 logical_codebase_feature_disabled
