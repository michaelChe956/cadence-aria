# Spec Delta

## Purpose

本 delta 为已授权的 enrollment 增加可追溯的 target/绑定版本与用户驱动恢复面，使 plan 侧错误在驾驶舱中通知、停等并可安全重绑/换代/接管后继续，同时保持人工计划门、显式自动授权和非 enrolled 流程不变。

## MODIFIED Requirements

### Requirement: 人工 design 确认后的显式授权与范围（REQ-WIGA-01）

系统 SHALL 保持 story/design 的生成和确认由人完成，并仅在 design 经人工确认后提供自动化模式选择；默认 SHALL 为不自动化。自动化 enrollment SHALL 将 target、plan、session、source、provider 和绑定版本作为同一条精确身份合同持久化；target SHALL 明确区分单仓与逻辑代码库载体。用户执行“重新绑定／换代”时 MUST 明确提供目标 plan/session、source、target 和 provider，并携带当前 expected binding/policy version。成功操作 SHALL 写入新的绑定版本并使后续动作只接受当前版本；旧绑定、旧 child 与旧代事件 SHALL 保持只读可追溯。系统 MUST NOT 按最新 session、旧 issue 扫描或后台失败发现猜测接管，也 MUST NOT 将 Manual 已认领 attempt 升级为自动。未选自动化、旧数据缺少 enrollment、或单 target 身份不明确时 SHALL 保持既有手动流程；零/多 target 或身份不一致 SHALL fail-closed 拒绝自动授权。

#### Scenario: design 确认前不出现自动授权

- **WHEN** story 或 design 尚未由人确认
- **THEN** 不创建 enrollment、不自动准备 plan，原有人工生成与确认入口继续可用

#### Scenario: 默认不自动化

- **WHEN** 用户在 design 确认后选择不自动化或旧 issue 不含 enrollment
- **THEN** plan 准备、生成、advance 和 coding 启动仍按原有手动行为执行，不因自动化补偿扫描产生动作

#### Scenario: 单 target 精确绑定及重复提交

- **WHEN** 已确认 source 只对应一个 logical repository，用户用相同选择键重复开启自动化
- **THEN** 系统仅返回同一 enrollment、同一绑定 plan/session 与策略修订；不同 source 或 provider/options 的冲突提交明确拒绝且不改绑

#### Scenario: 多 target 不被自动纳入

- **WHEN** 已确认 source 或冻结的 plan/attempt 涉及多个目标仓库、目标不明确或与 enrollment target 不符
- **THEN** 系统拒绝自动授权或自动启动，绝不循环启动多个 attempt；既有逐 target 手工准入仍可用

#### Scenario: 显式重绑创建新绑定版本

- **WHEN** 当前 enrollment 处于等待恢复且用户提交匹配 expected version 的 plan、session、source、target、provider
- **THEN** 系统写入新的绑定版本，返回当前 enrollment 身份与下一步操作；旧绑定保持只读，后续编排只消费新版本

#### Scenario: 绑定版本过期拒绝

- **WHEN** 用户以过期 expected binding/policy version 或不匹配的 plan/session/target 提交重绑
- **THEN** 系统返回版本冲突/需刷新错误，不覆盖当前绑定、不启动 provider、不改变旧代历史

#### Scenario: 同一命令重放幂等

- **WHEN** 客户端以相同 command_id 和相同绑定负载重复提交同一重绑/换代操作
- **THEN** 系统返回首次操作结果，不创建第二绑定版本、不重复触发后续编排；同一命令异负载被拒绝

### Requirement: 无页面的确定性后端推进与人工停点（REQ-WIGA-03）

对仍有效且精确绑定的单 target enrollment，服务端 SHALL 从 durable 状态重建下一动作：至多幂等准备一次 plan/session、至多启动一次对应生成、等待人处理 choice 和人工计划门、等待既有 compile/publication 成功、以独立 advance 推进到 Ready，再通过共用 StartCoding 命令启动该 attempt。服务端 SHALL 在每次 restart、recover 或 new advance 前重新读取当前绑定与租约/attempt claim。编排事件仅作为唤醒；无页面或没有任何 WS 连接时也 SHALL 继续安全动作及恢复。门关闭、approve 点击、compile 节点结束或 advance_completed 均不得被误判为 Confirmed、coding 完成或 provider 启动。租约冲突时，活跃持有者 SHALL 使系统通知用户等待；已死亡持有者仅 SHALL 展示“确认接管”操作，用户确认后才可继续；活性未知 SHALL 保持等待，不得自动抢占。所有恢复动作 SHALL 通过既有 advance/runner/初始化链继续，不能以通知投递成功冒充业务成功。

#### Scenario: 从 design 确认自动生成且停在人工门

- **WHEN** 单 target design 经人确认后选择自动化，用户关闭工作区与 driver 连接
- **THEN** 服务端准备唯一 plan/session、启动一次生成；到 choice/人工门时停等驾驶舱操作，不自动回答或代批准

#### Scenario: 确认后独立推进编码

- **WHEN** 人批准了绑定 plan 且既有 compile/publication durable 成功，enrollment 仍授权
- **THEN** 服务端请求独立 advance 到单 attempt Ready，随后经 StartCoding 发起 coding；advance 自身不启动 provider

#### Scenario: 失败或漏唤醒恢复

- **WHEN** 服务重启、事件漏投、计划创建已成功但绑定未写、或动作在持久检查点间中断
- **THEN** 后续扫描/补偿重用同一 plan/session/action 身份，已认领的启动只按权威事实恢复或进入显式人工分诊，不创建重复 plan、尝试或并行 provider run

#### Scenario: 活跃租约冲突停等

- **WHEN** restart、recover 或 new advance 重新读取到仍有活跃 owner/attempt
- **THEN** 系统保留当前 durable 状态，向驾驶舱和系统通知“请等待/已有运行”及绑定身份，不抢占、不启动第二 provider

#### Scenario: 死亡持有者经确认接管

- **WHEN** 重新读取确认旧 owner 已终态或已超过活动窗口且用户提交匹配当前版本的接管命令
- **THEN** 系统原子记录接管结果并继续原 enrollment 的安全恢复；旧 owner 的迟到写入被拒

#### Scenario: 活性未知保持等待

- **WHEN** lease/attempt claim 既不能证明活跃也不能证明死亡
- **THEN** 系统通知需要人工确认/重新绑定并保持停等，不自动 takeover、不盲目重启外部 provider

### Requirement: StartCoding 单入口与 per-attempt 单发（REQ-WIGA-04）

人工 WS 命令和 enrolled 编排器 SHALL 经同一 typed StartCoding 准入服务；enrolled 编排器在发起独立 StartCoding 或 advance 相关动作前 MUST 核对当前绑定版本、精确 target 与 attempt 身份。绑定读取失败、过期或不匹配 SHALL fail-closed。该核对不得创建 coding provider 的第二启动入口，人工门和现有 StartCoding 单入口语义不变。

#### Scenario: 手工与后台同时请求同一 attempt

- **WHEN** 人工 StartCoding 与已授权的自动 StartCoding 同时进入同一 attempt
- **THEN** 最多一个请求认领首启，另一个幂等命中该 attempt 的原状态，不出现两个 runner/provider

#### Scenario: Ready 前与身份不匹配均拒绝

- **WHEN** attempt 未被该 plan 的 durable advance 绑定置 Ready、绑定不可读、或 target 与 enrollment 不一致
- **THEN** 自动请求零启动；SC Ready 前请求复用 SC_CODING_REQUIRES_ADVANCE，禁止直接调用 runner 绕过守卫

#### Scenario: 崩溃后不盲目二次启动

- **WHEN** 启动意图已持久化但 runner 是否触达外部 provider 不明，服务重启并补偿该 attempt
- **THEN** 服务只能按单发检查点与既有恢复协议重用原启动或进入明确的人工恢复态，不重新发一条首启命令

#### Scenario: 迟到旧代回执不改变当前 attempt

- **WHEN** 旧绑定版本的 advance/start 回执在用户换代后到达
- **THEN** 系统拒绝该回执且不改变当前代绑定、attempt 或 provider 状态
