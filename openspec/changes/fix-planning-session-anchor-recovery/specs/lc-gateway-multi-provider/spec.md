# Spec Delta

> 来源：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/openspec/changes/lc-gateway-multi-provider/specs/lc-gateway-multi-provider/spec.md`（未归档，主spec尚无该capability）。本delta按真实标题MODIFIED，不重造REQ-LCG编号；同步须先确立源change，再应用本delta，不能单独当新capability重复创建。owner2026-10-11允许有界只读版本观测＋60秒TTL，此口径只解释会话进程，不改变其它指纹/权限门。

## MODIFIED Requirements

### Requirement: LC admission 与 spawn 前复验

每个LC provider会话 SHALL 使用manifest provider_context_root canonical path作为process working_directory并传独立native target。workspace Story/Design/Plan的PlanningReadOnly native target SHALL 按REQ-PLN-08固定aggregate-root，业务材料/compile target保留原成员规则；Coding target仍member checkout/worktree。validate与会话child启动之间 SHALL 重新核authority/policy/target/cwd/trust/capability/projection/availability/适用D4，漂移拒绝，不裸input/legacybridge/provider fallback。

#### Scenario: validate 后 authority 或投影漂移
- **WHEN** validate后、会话spawn前authority/policy/cwd/target/trust/capability/projection/availability冻结事实变化
- **THEN** 系统 SHALL 返回可审计PolicyDrift/ProviderUnavailable，adapter session-start与会话child为零；版本观测单独遵下述60秒TTL合同，不缓存其它事实

#### Scenario: LC Coding target 独立于 root cwd
- **WHEN** LC Coding以member worktree为target请求启动
- **THEN** process cwd SHALL 为canonical root，唯一writable root恰为target；聚合root不成为Coding写根，target不替cwd

#### Scenario: 同步与流式入口
- **WHEN** LC规划/split/Coding/Review入口请求真实provider
- **THEN** 同步/流式 SHALL 经同validated gateway policy；不支持/材料缺失fail-closed，不以本恢复功能替无native合同的同步split启动streaming fresh

#### Scenario: PlanningReadOnly aggregate native 具体化
- **WHEN** workspace Story/Design/Plan处于PlanningReadOnly且同一authority/snapshot有效
- **THEN** native envelope target SHALL 为canonical aggregate-root、writable roots为空，business repository/material/IR target保持原有效成员规则；此为原成员target表述的规划具体化，不影响Coding

### Requirement: 可验证的 provider policy projection

系统 SHALL 从不可由caller伪造的gateway完整projection将action/role/tool policy/permission/approval/sandbox/MCP/config/trust/cwd/native target/writable roots映射到真实CLI/RPC/ACP权限，并记录精确digest。LC workspace在session spawn前 SHALL 准备候选full projection，与旧full冻结基准比较，spawn时审计/复验同源；logical roots不是OS隔离证明，真实boundary evidence仍为write_boundary Confirmed前提。

#### Scenario: Projection 与逻辑写根一致
- **WHEN** gateway为provider/action生成projection
- **THEN** adapter SHALL 仅消费绑定envelope/provider exact version/dialect/role/native target的projection，audit可追digest/evidence；业务target不能污染规划native聚合身份

#### Scenario: Boundary 证据缺失
- **WHEN** target-only或readonly boundary probe未证实真实provider/OS权限
- **THEN** write_boundary SHALL 为Unknown/Denied并阻断action，不以空writable/MCP成功/git status/模型自述替证据

#### Scenario: candidate 与 native 审计同源
- **WHEN** workspace准备fresh或resume candidate
- **THEN** current version/trust/config/MCP/boundary材料 SHALL 来自同provider真实只读facts，候选full fingerprint与spawn/audit投影一致；不得以空常量域basis或registry未知version冒充full材料，不将旧basis与full混比

### Requirement: Provider resume 与冻结指纹

系统 SHALL 仅在action resume=Confirmed、原生恢复真实同ID且policy/authority/canonical cwd/native target/git/worktree/provider exact version/dialect/capability evidence/tool policy/full projection/trust/适用Aria MCP digest一致时恢复LC旧session。Kimi原生MCP不是Aria bundle。不支持/失配/缺完整旧冻结记录 SHALL 不续旧、不静默fresh；用户明确新会话后才完整fresh admission。比较自身不创建会话进程，有界只读--version按下述独立观测合同。

#### Scenario: 原生恢复形态
- **WHEN** 请求Claude --resume、Codex thread/resume、Pi session-id/RPC或Kimi session/load
- **THEN** resume能力/版本/dialect/fullprojection指纹 SHALL 一致且原生确认同ID，不回填请求ID伪确认

#### Scenario: Resume capability 未确认
- **WHEN** resume能力为Unknown/Denied
- **THEN** 系统 SHALL 会话child前typed拒收，不伪称恢复或静默fresh

#### Scenario: Fingerprint 漂移
- **WHEN** policy/cwd/native target/trust/capability/tool/projection/适用MCP与旧冻结事实失配
- **THEN** 系统 SHALL 标记旧native superseded并要求明确fresh，adapter session-start/session child为零；同一revision driver不继续start_streaming

#### Scenario: 缺完整旧 map
- **WHEN** native/audit/旧运行存在但fullmap不可验证或basis不兼容
- **THEN** 系统 SHALL 耐久waiting＋显式fresh，不用当前fingerprint覆旧会话，不借audit兜底捞已superseded ID

## ADDED Requirements

### Requirement: LC 版本只读观测与60秒缓存

LC当前exact-version观测 SHALL 允许有界只读CLI --version，进程内成功结果TTL固定60秒且非滑动；同key并发miss/到期 SHALL single-flight一条观测。缓存仅版本事实，key隔离provider/命令canonical executable identity/参数/supplier域；失败/超时/取消不缓存，不回退过期成功。probe不是会话进程，仍与adapter start/session child分别计数；TTL更长防护更钝，不默改参数。

#### Scenario: 并发 burst 压一条
- **WHEN** 同key当前版本缓存缺失/到期，多caller同时需要观测
- **THEN** 系统 SHALL 只执行一条有界version probe，共享同结果；不同key互不串行、不串值；不因观测创建native会话

#### Scenario: 非滑动TTL边界
- **WHEN** 成功观测完成后age小于60秒或达到60秒
- **THEN** age<60秒 SHALL 复用而不延长到期，age>=60秒 SHALL 再观测；进程重启不复用旧缓存，命令identity变化失效；最多60秒版本变化识别滞后如实呈现

#### Scenario: 失败与取消不保留过期值
- **WHEN** 观测失败/超时/取消
- **THEN** 系统 SHALL 不缓存失败、不使用已到期旧成功，不把候选当准入通过；caller取消后不会复活已拒绝旧run

#### Scenario: 三种spawn口径
- **WHEN** 验收mismatch或native握手失败
- **THEN** 证据 SHALL 分列version probe、adapter session-start、session child/native创建；mismatch后两项为零，握手后错ID须kill/reap且不得冒称零child；direct原版本cache行为不改
