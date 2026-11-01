# coding-run-operability Specification

## Purpose

定义 coding run 在断连、重复启动、abort 后重启、人工继续与 reviewer 配置缺失时的可操作停等契约：已完成事实先持久化，同一时刻至多一个编码进程，所有恢复都由带对象身份的用户操作触发并回到原编排链继续。

## Requirements

### Requirement: 已完成产物先持久化后观察（REQ-CRO-01）

provider 完成一次运行后（coding 的 Coder、Code Reviewer、Internal Reviewer、返修运行，以及驱动 coding 前置的 workspace 会话修订回合），系统 SHALL 先把该运行的输出、artifact、source 与 completion checkpoint 写入既有持久面，再发送任何观察事件（WebSocket、relay 或通知）。连接 attach、detach、supersede 或观察事件投递失败 MUST NOT 取消、回滚或删除已持久化的完成结果，MUST NOT 把已完成运行改写为失败或中止。重连页面 SHALL 只从持久面补读投影。若系统无法证明 provider 是否已完成（例如完成信号与持久化之间中断），系统 SHALL 进入等待确认状态并通知用户，MUST NOT 自动重新启动该运行。

#### Scenario: finalize 与 relay 窗口断连

- **WHEN** provider 已完成返修且完成事实已持久化，驱动该 attempt 的连接在 relay 发送前断开
- **THEN** 该返修 artifact 与 completion checkpoint 保留；重连后页面补读到同一结果；后续 Code Review 只对该结果执行一次

#### Scenario: 观察通道失败不影响业务结果

- **WHEN** 没有任何 WebSocket 订阅者或观察通道投递失败
- **THEN** 业务结果与 attempt 状态照常持久化，驾驶舱 inbox 可补读，不产生失败或中止终态

#### Scenario: 完成状态不明时停等

- **WHEN** provider 输出流已结束但系统找不到对应的持久化 completion checkpoint
- **THEN** 系统落地“完成状态待确认”等待项并通知用户，展示最后活动时间与已知证据；用户确认前不启动新的 provider 运行

### Requirement: 编码进程最小互斥与接管判别（REQ-CRO-02）

同一 coding attempt 及其共享工作树 SHALL 同时至多存在一个编码进程。任何启动、重试、继续或 restart 请求 SHALL 在原子临界区内重读工作树租约与 attempt claim，并按租约三态处置：持有者活跃时拒绝第二次启动并通知“已在运行／请等待”；持有者已死（终态 attempt、锁已释放或超出活动窗口）时呈现“确认接管”操作，用户确认后方可接管并继续；活性未知时停等并要求用户确认。接管确认前系统 SHALL 展示最后活动时间与节点证据。接管后旧持有者的迟到写入 SHALL 被拒绝。同一 `command_id` 的重复请求 SHALL 返回首次结果，不产生第二个进程。系统 MUST NOT 自动抢占活跃或未知持有者，MUST NOT 为此引入 owner incarnation、epoch 或 fence 体系。

#### Scenario: 双客户端重复 kick

- **WHEN** 两个客户端对同一 attempt 几乎同时发起启动，且其中一个已成功取得编码进程
- **THEN** 另一个请求收到“已在运行”结果与通知，provider 只启动一次

#### Scenario: 同命令重放幂等

- **WHEN** 同一 `command_id` 与相同负载的启动请求被重复提交
- **THEN** 系统返回首次 durable 结果，不创建第二个编码进程或第二个 UnitRun

#### Scenario: 已死持有者经用户确认接管

- **WHEN** 工作树锁的持有者 attempt 已处于终态，用户在驾驶舱查看最后活动时间与节点证据后点击“确认接管”
- **THEN** 系统原子清出旧持有者并按当前请求继续；旧持有者随后的写入被拒绝，旧 attempt 记录只读保留

#### Scenario: 活性未知不抢占

- **WHEN** 租约持有者的活性无法由持久证据证明
- **THEN** 系统停等并通知用户确认，不自动接管、不启动 provider

### Requirement: abort 后显式 restart 可用（REQ-CRO-03）

用户明确中止 attempt 后，系统 SHALL 在同一服务进程内允许该 attempt 通过显式 restart 操作重新执行：restart SHALL 先确认该 attempt 没有活跃编码进程，再清除进程内为该 attempt 保留的中止退役状态，并经既有终态重开 admission 回到可执行状态。restart 按钮 SHALL 仅在 attempt 处于 Aborted／Failed 且无活跃编码进程时可见。restart 请求 SHALL 携带当前 attempt 身份与版本；旧 run 的迟到回执、旧版本或错对象请求 SHALL 被拒绝且 MUST NOT 复活旧 run。系统 MUST NOT 依赖重启服务来恢复 restart 能力。

#### Scenario: 同进程 abort 后 restart

- **WHEN** 用户在驾驶舱中止正在运行的 attempt，随后在同一服务进程内点击 restart
- **THEN** attempt 回到 Running 并启动一个新的编码进程，无需重启服务

#### Scenario: 旧回执不复活旧 run

- **WHEN** restart 成功后，被中止的旧 run 迟到提交完成或事件回执
- **THEN** 系统拒绝该回执，当前 attempt 状态与新 run 不受影响

#### Scenario: 仍有活跃进程时不显示 restart

- **WHEN** 中止请求已发出但旧编码进程尚未退出
- **THEN** restart 操作不可用，界面显示“正在停止”；进程退出后才可 restart

### Requirement: 人工继续后回到原编排阶段（REQ-CRO-04）

用户在 coding 门上提交允许的继续类动作（`manual_continue`、允许的质量绕过或验证处理结论）后，系统 SHALL 把门结果写入既有 gate 记录，并唤回原 runner 从该门所在阶段之后继续推进，而不是只改变门状态后停止。已完成且持久化的 review 结论 SHALL 被复用，MUST NOT 被重新执行。该继续 SHALL 不依赖提交动作的 WebSocket 连接保持打开。

#### Scenario: CodeReview 门人工继续后关闭 socket

- **WHEN** 用户在 Code Review 分诊门执行 `manual_continue` 后立即关闭页面与 coding socket
- **THEN** runner 从 Code Review 之后的阶段继续推进，已完成的 Code Reviewer 运行不被重新启动

#### Scenario: 重复提交继续动作

- **WHEN** 同一门的同一继续动作以相同 `command_id` 被重复提交
- **THEN** 系统返回首次结果，不产生第二个 runner 或重复的阶段推进

### Requirement: reviewer 配置三值完整性（REQ-CRO-05）

系统 SHALL 分别持久化 reviewer 的 effective（实际生效 provider，可为空）、provisional（用户已选但当前未生效的 provider）与 enabled（是否启用）三个值，且重建会话或 attempt 时 SHALL 按原值恢复。系统 MUST NOT 在 reviewer 为空或 disabled 时以 author/coder provider 回填 reviewer。plan 会话 reviewer disabled 时 SHALL 保持 disabled，不启动 reviewer。coding attempt 需要 Code Reviewer 或 Internal Reviewer 但其 provider 缺失时，系统 SHALL 落地“reviewer 配置缺失”等待项并通知用户，提供配置／重试操作；MUST NOT 将缺失视为自动通过或以 author 顶替。

#### Scenario: 重启后 disabled reviewer 保持 disabled

- **WHEN** plan 会话以 reviewer disabled 启动，随后服务重启并从持久面重建会话
- **THEN** 重建后的会话 reviewer 仍为 disabled、effective 为空，系统不启动任何 reviewer

#### Scenario: coding reviewer 缺失停等

- **WHEN** coding attempt 进入需要 Code Reviewer 的阶段，但其 reviewer provider 为空
- **THEN** 系统落地“reviewer 配置缺失”等待项与通知，不启动 author 作为 reviewer，不把该阶段视为通过

#### Scenario: 补齐配置后继续

- **WHEN** 用户在等待项上配置合法 reviewer 并点击重试
- **THEN** 系统以新配置从原阶段继续，历史快照不被改写

### Requirement: coding 等待项的通知与操作结果（REQ-CRO-06）

本 capability 定义的每个等待项（完成状态待确认、已在运行、确认接管、活性未知、restart、reviewer 配置缺失）SHALL 在驾驶舱 inbox 与系统通知中展示原因、attempt／unit／gate 身份、已完成步骤、可能的外部副作用、操作会做与不会做的事以及成功后的下一阶段。每个操作 SHALL 携带稳定 `command_id` 与 expected 对象版本；同键同负载返回首次结果，异负载或过期版本 fail-closed。REST 与页面动作 SHALL 调用同一应用服务。通知投递失败 MUST NOT 回滚业务事实。

#### Scenario: 过期版本操作被拒

- **WHEN** 用户在旧页面上对已被其他操作推进的 attempt 点击接管或 restart
- **THEN** 系统返回“请刷新”结果，不改变当前 attempt，不启动 provider

#### Scenario: 通知失败可补读

- **WHEN** 系统通知投递失败
- **THEN** 等待项仍可在驾驶舱 inbox 补读，业务状态不变
