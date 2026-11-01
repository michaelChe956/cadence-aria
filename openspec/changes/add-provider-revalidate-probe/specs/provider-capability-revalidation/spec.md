# Spec Delta

## Purpose

为 LC 作用域内的 provider capability 提供用户显式的真实验证通道：一次点击触发真实现场探针并原子导入 durable Confirmed；已验证且版本一致时秒回不重跑；失败如实上报。门本身（admission fail-closed）不变。

## ADDED Requirements

### Requirement: 用户显式 capability 重验证通道

系统 SHALL 为每个逻辑代码库提供 per-provider 的显式 capability 重验证动作：仅由用户显式触发（HTTP POST，携带 `provider_type`），对该 provider 执行真实现场边界探针（复用既有 `run_cli_boundary_probe` 通道：覆盖 CodingTargetWrite、PlanningReadOnly、ReviewReadOnly 三个 action，含 resume 面），并把每个通过三方一致性校验的探针结果经 `record_verified_probe` 原子导入 durable Confirmed。导入写入的 capability store SHALL 与 gateway/admission 消费的作用域同一 LC 子树。系统 SHALL NOT 在无显式点击时运行任何探针，SHALL NOT 提供手工改写 Confirmed 的旁路。

#### Scenario: 点击核验补齐 Unknown
- **WHEN** provider 的 capability 行为 Unknown 且用户对该 provider 触发重验证
- **THEN** 系统 SHALL 执行真实现场探针，探针通过后将该 provider 的 action 行导入 durable Confirmed（launch/write_boundary/resume 由工件签发），随后同 LC 作用域的 admission 读到 Confirmed

#### Scenario: 不点击不探测
- **WHEN** capability 行为 Unknown 且用户未触发重验证
- **THEN** 系统 SHALL NOT 运行任何 provider CLI 探针；admission 维持既有 fail-closed 拒绝

#### Scenario: Fake 与未知 provider 拒绝
- **WHEN** 重验证请求的 `provider_type` 是 Fake 或未知值
- **THEN** 系统 SHALL 返回稳定错误码（provider_capability_probe_unsupported），不执行任何探测、不写入 capability 记录

### Requirement: 版本钉定的一次性幂等语义

重验证 SHALL 尊重 capability 记录的 exact version 钉定：action 行已在当前 CLI exact version 上 Confirmed（launch、write_boundary、resume 全 Confirmed）时，该行 SHALL NOT 重跑探针；三行全齐时点击 SHALL 快速返回 `already_confirmed`（仅执行一次 CLI `--version` 版本探测，不启动探针会话）。CLI 版本相对 durable 记录漂移，或任一必需行缺失/非 Confirmed 时，系统 SHALL 仅对缺失或漂移的 action 执行探针；探针内部版本门漂移时 SHALL 如实失败，不沿用旧版本证据。

#### Scenario: 全行已验证秒回
- **WHEN** provider 三个 action 行均已在当前 CLI 版本 Confirmed 且用户再次点击核验
- **THEN** 系统 SHALL 不运行边界探针并返回 already_confirmed，durable 记录与 probed_at 保持不变

#### Scenario: 版本升级触发重验
- **WHEN** CLI 升级导致 durable 记录 version 与当前 CLI 不一致
- **THEN** 系统 SHALL 重新执行探针并以新 exact version 导入；导入前的旧 Confirmed 行不跨版本沿用（沿既有 import 语义）

### Requirement: 探针失败如实上报

任一 action 探针失败（CLI 缺失、边界不可用、正/负探针未达预期、resume 面失败、导入被拒）时，系统 SHALL 返回携带稳定错误码、action 名与原始错误详情的失败响应； SHALL NOT 把失败静默为通过、不伪造 Confirmed、不部分遮蔽（同一请求中先成功的 action 导入保留，失败 action 如实点名）。

#### Scenario: CLI 缺失如实失败
- **WHEN** 目标 provider CLI 不在场（`--version` 探测失败）
- **THEN** 系统 SHALL 返回稳定错误码（provider_capability_probe_failed）与缺失详情，capability 记录保持点击前状态

#### Scenario: 探针中断不伪造
- **WHEN** 边界探针在任一环节失败关闭
- **THEN** 系统 SHALL 上报该 action 的失败详情；未通过校验的证据 SHALL NOT 进入 durable

### Requirement: 只读 capability 状态投影

系统 SHALL 提供 per-LC 的只读 capability 状态 GET 面：逐 provider 投影 durable action 矩阵（Confirmed/Denied/Unknown）、exact version、probed_at、probe_artifact_ref 与 provider 通道可用性（四家真实 provider 之外不出现）；该面零写入、零探针触发。

#### Scenario: 状态面只读
- **WHEN** 用户读取 capability 状态
- **THEN** 系统 SHALL 返回当前 durable 事实且不运行任何 CLI、不修改任何记录

#### Scenario: 未探测投影 Unknown
- **WHEN** provider 尚无任何真实探针证据
- **THEN** 状态面 SHALL 投影全 Unknown（bootstrap 默认记录不得被当作真实证据显示为 Confirmed）
