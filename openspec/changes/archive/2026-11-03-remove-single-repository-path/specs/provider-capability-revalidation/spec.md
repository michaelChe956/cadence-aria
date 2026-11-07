# provider-capability-revalidation Specification

## Purpose

（主规范沿用）为 LC 作用域内的 provider capability 提供用户显式的真实验证通道。本 delta 只写**增量**：把多个单次核验变成一次「一键核验全部」的异步作业 + 轮询 + 逐项进度。单家的探针语义（显式触发、版本钉定幂等、失败如实上报、状态只读投影）由主规范既有 requirement 拥有，本 delta MUST NOT 重复定义、MUST NOT 放宽。

## ADDED Requirements

### Requirement: 一键批量核验的异步作业入口（REQ-PCR-03）

系统 SHALL 提供 per-LC 的批量核验入口 `POST /api/projects/{project_id}/logical-codebases/{lc_id}/capabilities/revalidate-all`，其响应 SHALL **立即返回 `job_id`** 与初始作业状态，MUST NOT 在该响应内等待任何 provider 探针完成。

系统 SHALL 提供 `GET /api/projects/{project_id}/logical-codebases/{lc_id}/capabilities/revalidate-all/{job_id}` 返回作业状态与逐家结果。作业 SHALL 只由用户显式点击发起，MUST NOT 因聚合初始化失败或任何其它流程自动触发。

#### Scenario: 发起即返回作业号

- **WHEN** 用户对某 LC 触发批量核验
- **THEN** 系统 SHALL 立即返回 `job_id` 与 `running` 状态；MUST NOT 在 HTTP 响应里等待任何 provider 探针完成

#### Scenario: 轮询作业与逐项结果

- **WHEN** 客户端携带 `job_id` 轮询
- **THEN** 系统 SHALL 返回作业状态（`running` / `completed`）与逐家结果（`provider_type` / 状态 / 原因或各 action 结果）

#### Scenario: 不自动触发

- **WHEN** 聚合初始化或其它流程因 provider 能力未核验而失败
- **THEN** 系统 SHALL NOT 自动发起批量核验作业；MUST NOT 运行任何探针（沿用主规范「不点击不运行探针」契约）

### Requirement: 批量核验的并发、失败隔离与逐项结果（REQ-PCR-04）

批量作业 SHALL 对四家真实 provider **全并发**执行（每家一个 provider 进程），MUST NOT 串行。每家 SHALL 直接复用主规范既有的单家重验证服务（`revalidate(project_id, lc_id, provider)`），因此既有的「全行已在当前版本 Confirmed → 秒回 `already_confirmed`（零探针）」与「CLI 缺失在版本探测阶段即失败」语义 SHALL 原样继承，本 requirement MUST NOT 重新定义它们。

单家失败 SHALL NOT 阻断其它家。逐项状态 SHALL 落在 `confirmed` / `failed` / `skipped` 三类并带原因（如 `cli_not_installed`）。作业到终态时仍未完成的家 MUST NOT 被报成已确认。

#### Scenario: 四家全并发

- **WHEN** 批量作业启动，四家 provider 都存在且都未核验
- **THEN** 四家的探针 SHALL 并发执行；MUST NOT 串行（串行 = 4 倍等待）

#### Scenario: 单家失败不阻断

- **WHEN** 某一家探针失败
- **THEN** 其它家 SHALL 继续执行到终态；该家 SHALL 标为失败并带原因，整批作业 MUST NOT 失败，其它家结果 MUST NOT 丢失

#### Scenario: 已核验的家瞬时返回

- **WHEN** 某家的三个 action 行均已在当前 CLI 版本 Confirmed
- **THEN** 该家 SHALL 按既有单家语义秒回 `already_confirmed`（零探针），作业结果标为已确认

#### Scenario: CLI 未安装的家标为跳过

- **WHEN** 某家 provider CLI 不在场（版本探测阶段失败）
- **THEN** 该家 SHALL 标为 `skipped` 并带稳定原因（如 `cli_not_installed`）；作业 SHALL NOT 因此挂起或整体失败

#### Scenario: 超时如实标注

- **WHEN** 作业到达终态时仍有家未完成
- **THEN** 状态 SHALL 为 `completed` 且未完成项 SHALL 标为超时/未完成；MUST NOT 谎报为已确认

### Requirement: 作业状态的 durable 与刷新恢复（REQ-PCR-05）

批量作业的状态（job 与每家状态）SHALL 持久化。页面刷新或重新进入后，系统 SHALL 能按既有 `job_id` 返回该作业的进度与逐项结果，MUST NOT 丢失进度或要求重新发起。

#### Scenario: 页面刷新后继续轮询

- **WHEN** 批量作业进行中，页面刷新或重新进入
- **THEN** 系统 SHALL 能按既有 `job_id` 继续返回该作业进度与逐项结果；MUST NOT 丢失进度或要求重新发起

#### Scenario: 终态作业仍可读

- **WHEN** 页面在作业终态后重新打开
- **THEN** 系统 SHALL 仍能返回该 `job_id` 的最终逐项结果；MUST NOT 返回「作业不存在」

### Requirement: 一键核验的全部入口与逐项进度（REQ-PCR-06）

能力卡 SHALL 提供「一键核验全部」按钮。点击后系统 SHALL 发起批量作业并按固定间隔（2~3 秒）轮询，作业终态后停止。进行中 SHALL 显示「核验中（N/4）」并逐项实时更新状态，按钮 SHALL 禁用。

失败项 SHALL 每行保留单项「重试」，其动作 SHALL 走既有单 provider 端点。`skipped`（CLI 未安装）项 SHALL 引导到该 provider 的【安装】按钮、版本落后项 SHALL 引导到【更新】按钮；安装/更新机制由块2 `provider-cli-installation` 定义（`REQ-PCI-02` / `REQ-PCI-05`），本 requirement MUST NOT 重新定义它。

按钮区域 SHALL 说明：核验是逐逻辑代码库的、会真实跑一次探测并消耗 provider 额度、可能需要几分钟。

#### Scenario: 一键按钮发起并显示逐项进度

- **WHEN** 用户点击「一键核验全部」
- **THEN** 系统 SHALL 发起批量作业并以「核验中（N/4）」显示进度，逐项状态随轮询更新；按钮 SHALL 保持禁用直到作业终态

#### Scenario: 失败项单独重试

- **WHEN** 作业结束且某家标为失败
- **THEN** 该行 SHALL 保留「重试」按钮，点击后走既有单 provider 核验端点，MUST NOT 重新发起整批作业

#### Scenario: 未安装项引导到安装

- **WHEN** 某家结果为 `skipped`（`cli_not_installed`）
- **THEN** 该行 SHALL 显示【安装】入口并走 `provider-cli-installation` 的作业；MUST NOT 显示为「核验失败」

#### Scenario: 版本落后引导到更新

- **WHEN** 某家本地版本落后于 registry 最新版本
- **THEN** 该行 SHALL 显示【更新】入口；判定与作业 SHALL 复用 `provider-cli-installation`，MUST NOT 由本能力自行判版本

#### Scenario: 说明额度与范围

- **WHEN** 用户看到「一键核验全部」按钮
- **THEN** 该区域 SHALL 说明核验是逐逻辑代码库的、会真实跑一次探测、可能需要几分钟；MUST NOT 让用户误以为这是零成本的后台检查

#### Scenario: 终态停止轮询

- **WHEN** 作业到达终态（`completed`）
- **THEN** 前端 SHALL 停止轮询并渲染最终逐项结果；MUST NOT 无限轮询
