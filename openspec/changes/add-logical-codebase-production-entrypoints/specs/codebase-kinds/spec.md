## Purpose

代码库实体与两种同级形式（单仓代码库/逻辑代码库），以及 issue 对代码库的唯一归属。

## ADDED Requirements

### Requirement: 代码库实体与统一列表

一个 project 必须允许一个或多个代码库，种类不限；单仓代码库与逻辑代码库为同级两种形式，通过统一列表端点可枚举（含 kind 区分）。

#### Scenario: 混合列表
- **WHEN** 调用 GET /api/projects/{pid}/codebases
- **THEN** 返回该 project 全部代码库（既有单仓 repositories 呈现为 single_repo 条目 + 逻辑代码库条目），含 id/name/kind 与成员计数

#### Scenario: 多逻辑代码库并存
- **WHEN** 同一 project 创建多个逻辑代码库
- **THEN** 各自独立存储（logical-codebases/{lc_id}/ 子树），相互零耦合；任一逻辑代码库的登记/初始化/索引/指针操作不影响其他代码库

### Requirement: 逻辑代码库 CRUD

逻辑代码库的创建、查询与删除必须通过生产 HTTP 端点可用。

#### Scenario: 创建
- **WHEN** POST /api/projects/{pid}/logical-codebases 携带 name（必填）与 aggregate_root
- **THEN** 创建逻辑代码库记录；manifest 在首批登记提交时原子创建（沿用既有原子语义）

#### Scenario: 详情与删除
- **WHEN** GET/DELETE /api/projects/{pid}/logical-codebases/{lc_id}
- **THEN** 返回详情（成员/初始化/索引状态汇总）或软删除该逻辑代码库；删除不对成员仓产生 git 写副作用

### Requirement: 逻辑专属端点按逻辑代码库寻址

登记/初始化/索引/成员/指针等逻辑专属端点必须按逻辑代码库 id 寻址，不存在时返回稳定 404。

#### Scenario: 端点换形
- **WHEN** 调用登记/初始化/索引/成员/指针端点
- **THEN** 路径为 /api/projects/{pid}/logical-codebases/{lc_id}/...；lc_id 不存在于该 project 时返回 404 logical_codebase_not_found；母 change 既有 /logical-codebase/ 端点作为默认第一个逻辑代码库的兼容别名继续可用

### Requirement: issue 唯一归属代码库

每个 issue 必须且只能归属一个代码库（单仓或逻辑）。

#### Scenario: 归属
- **WHEN** 创建 issue
- **THEN** 必须且只能归属一个代码库：单仓代码库（repository_id）或逻辑代码库（logical_codebase_id + 其 active 成员中的 primary repository_id）；不存在跨代码库 issue
