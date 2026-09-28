# Spec Delta

## ADDED Requirements

### Requirement: Failed advance 的显式初始化重试（REQ-ADV-C1-RETRY）

当 advance 初始化进入 Failed 时，系统 SHALL 提供明确的 `retry-initialization` 产品操作；重复原 advance 命令本身 MUST NOT 被解释为重试。retry MUST 核对当前 plan、plan revision、target、attempt 身份及已完成 checkpoint，并保留原 Failed 记录、原因和审计事实。系统 SHALL 为用户操作创建独立 retry 事实；仅可证明未产生外部副作用的本地步骤可自动续做，外部副作用状态未知时 SHALL 先通知并等待用户确认。安全重试 SHALL 继续同一 attempt，不创建第二 attempt，也不得把原失败记录改写成成功。

#### Scenario: 用户重试 Failed advance 并到 Ready

- **WHEN** 当前 plan/target 的 advance attempt 为 Failed，用户从驾驶舱点击 `retry-initialization` 且对象版本、绑定和 checkpoint 仍匹配
- **THEN** 系统保留原 Failed 审计，写入独立 retry 事实，安全本地步骤从 checkpoint 续做并将同一 attempt 推进到 Ready

#### Scenario: 普通 advance 不隐式重试

- **WHEN** 某 plan/target 已有 Failed attempt，客户端再次发送原 `advance`（同或不同 command_id）但未提交 retry-initialization
- **THEN** 系统返回原 attempt 与失败原因，不创建新 attempt、不清除失败审计、不启动 provider

#### Scenario: 外部副作用未知时先确认

- **WHEN** retry 检查发现初始化步骤可能已触发但无法证明外部副作用是否完成
- **THEN** 系统通知用户当前未知事实并展示确认后重试/重新绑定操作；在用户确认前不盲目重放该步骤

#### Scenario: retry 版本过期拒绝

- **WHEN** retry 携带的 plan/revision/target/attempt 版本已过期或绑定不匹配
- **THEN** 系统 fail-closed 返回刷新/重新绑定提示，原 Failed 事实与 attempt 均不变
