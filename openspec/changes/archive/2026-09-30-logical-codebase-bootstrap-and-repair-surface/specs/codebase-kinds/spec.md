# Spec Delta

## MODIFIED Requirements

### Requirement: 代码库实体与统一列表

一个 project 必须允许一个或多个代码库，种类不限；单仓代码库与逻辑代码库为同级两种形式，通过统一列表端点可枚举（含 kind 区分）。统一代码库路由 SHALL 将单仓代码库与逻辑代码库作为同级但互斥的 target kind 解析。请求携带逻辑代码库身份时只能读取其明确的 logical repository/member authority；请求携带单仓身份时只能读取对应真实物理仓；缺失 kind、跨 kind 身份、重复候选或目标与当前 issue/enrollment 不一致时 SHALL fail-closed。系统 MUST NOT 通过路径猜测、项目级历史布局或“最新可用”记录把一种 kind 转换为另一种 kind。

#### Scenario: 混合列表

- **WHEN** 调用 GET /api/projects/{pid}/codebases
- **THEN** 返回该 project 全部代码库（既有单仓 repositories 呈现为 single_repo 条目 + 逻辑代码库条目），含 id/name/kind 与成员计数

#### Scenario: 多逻辑代码库并存

- **WHEN** 同一 project 创建多个逻辑代码库
- **THEN** 各自独立存储（logical-codebases/{lc_id}/ 子树），相互零耦合；任一逻辑代码库的登记/初始化/索引/指针操作不影响其他代码库

#### Scenario: 两种代码库并列且路由稳定

- **WHEN** 同一 project 同时存在单仓代码库和一个或多个逻辑代码库，用户分别查询其详情或成员
- **THEN** 每次响应 SHALL 带明确 kind 与稳定 identity，并只返回对应 authority 下的数据；一个代码库的成员、索引或 repair 操作不得影响另一个代码库

#### Scenario: kind 或 identity 不一致

- **WHEN** 请求缺少代码库 kind、提供与 issue 绑定不符的 kind，或试图以单仓身份访问 LC 子树（反之亦然）
- **THEN** 系统 SHALL 返回可诊断的目标冲突/重新选择提示，不切换读取来源、不创建索引、不修改现有绑定
