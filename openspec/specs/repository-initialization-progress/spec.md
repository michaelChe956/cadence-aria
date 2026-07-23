# repository-initialization-progress Specification

## Purpose

为代码库初始化提供可轮询的异步操作状态：固定五步真实进度、失败诊断与最终结果，服务端是步骤状态的唯一事实来源。

## Requirements


### Requirement: 可轮询的代码库初始化操作
系统 SHALL 在接受添加代码库请求后创建一个具有唯一 `operation_id` 的持久化代码库初始化操作，并立即返回 operation 快照，而不等待 Cadence-skills 与 Claude Code 初始化完成。系统 SHALL 提供只读接口，以便客户端通过 `operation_id` 获取当前快照和最终结果。

#### Scenario: 成功接受初始化请求
- **WHEN** 用户为已有 Project 提交有效的未注册 Git 代码库
- **THEN** 系统 SHALL 创建状态为 `created` 或 `running` 的 operation、返回其 `operation_id` 和初始快照，并在后台开始初始化

#### Scenario: 查询非终态操作
- **WHEN** 客户端查询一个仍在执行的有效 `operation_id`
- **THEN** 系统 SHALL 返回服务端已持久化的 operation 状态、五个步骤的真实状态和当前可用诊断信息

#### Scenario: 查询完成操作
- **WHEN** 客户端查询状态为 `completed` 的有效 `operation_id`
- **THEN** 系统 SHALL 返回五个 `completed` 步骤以及与现有代码库创建成功响应等价的 `repository` 和 `initialization` 最终结果

#### Scenario: 查询失败操作
- **WHEN** 客户端查询状态为 `failed` 的有效 `operation_id`
- **THEN** 系统 SHALL 返回失败步骤、未执行步骤、结构化错误详情以及可用的 changed paths

#### Scenario: 查询不存在操作
- **WHEN** 客户端查询未知的 `operation_id`
- **THEN** 系统 SHALL 返回稳定的“初始化操作不存在”错误，且不得返回其他 Project 或代码库的操作信息

### Requirement: 固定五步真实状态机
每个代码库初始化 operation SHALL 按以下顺序创建且仅创建五个步骤：`cadence_skills`、`rule_config`、`pre_check`、`mcp_configuration`、`project_rules_examples`。每一步 SHALL 具有 `pending`、`running`、`completed` 或 `failed` 状态；服务端 SHALL 是这些状态的唯一事实来源。

#### Scenario: 初始化开始时的步骤状态
- **WHEN** 系统创建新的初始化 operation
- **THEN** 系统 SHALL 将五个固定步骤按规定顺序记录为 `pending`

#### Scenario: 真实步骤开始与完成
- **WHEN** 后端即将实际执行某个固定步骤
- **THEN** 系统 SHALL 在开始该调用前将该步骤写为 `running`，并且仅在该调用成功返回后将其写为 `completed`

#### Scenario: 严格顺序推进
- **WHEN** 前一固定步骤尚未处于 `completed`
- **THEN** 系统 SHALL 不得将后续步骤置为 `running`

#### Scenario: 步骤失败
- **WHEN** Cadence-skills 准备或任一 Claude Code 初始化命令失败、超时、取消或要求交互
- **THEN** 系统 SHALL 将当前步骤写为 `failed`、将 operation 写为 `failed`、保留后续步骤为 `pending`，且不得创建 Repository 记录

#### Scenario: Repository 持久化失败
- **WHEN** 五个固定步骤均已完成但 Repository 记录持久化失败
- **THEN** 系统 SHALL 将 operation 写为 `failed`，提供持久化失败的结构化诊断和 Git changed paths，且不得报告 operation 成功

### Requirement: Cadence-skills 准备步骤可见
系统 SHALL 将 Cadence-skills 的下载/更新、离线回退和三层 Skills 软链同步作为第一个固定步骤 `cadence_skills` 的实际工作内容。

#### Scenario: Cadence-skills 准备成功
- **WHEN** Cadence-skills 源准备和软链同步成功
- **THEN** 系统 SHALL 将 `cadence_skills` 标为 `completed`，并保留现有 source mode、Git 更新、软链同步和 warning 摘要供最终结果使用

#### Scenario: Cadence-skills 准备失败
- **WHEN** Cadence-skills 无法下载、更新、验证或同步软链
- **THEN** 系统 SHALL 将 `cadence_skills` 标为 `failed`，不得开始任一 Claude Code 命令，并提供既有可恢复错误信息

### Requirement: 初始化操作终态一致性
系统 SHALL 仅在五个固定步骤完成、最终 Git 状态已收集且 Repository 记录持久化成功后将 operation 标为 `completed`。系统 SHALL 保持同一 Git 代码库路径的初始化互斥，并将执行中断转换为可诊断终态，避免无限期 `running`。

#### Scenario: 全部步骤成功后完成
- **WHEN** 五个固定步骤、最终 Git 状态采集和 Repository 持久化均成功
- **THEN** 系统 SHALL 将 operation 标为 `completed`，保存最终成功结果，并使后续查询返回相同终态结果

#### Scenario: 重复提交同一路径
- **WHEN** 同一 Project 中某 Git 根目录已注册或正在初始化
- **THEN** 系统 SHALL 拒绝新的重复初始化请求，且不得为该路径并发执行两个初始化流程

#### Scenario: 执行进程中断
- **WHEN** 服务在 operation 处于非终态时重启或无法继续该 operation
- **THEN** 系统 SHALL 在 operation 恢复可见时将其呈现为带恢复建议的失败终态，而不得无限期报告其为 `running`
