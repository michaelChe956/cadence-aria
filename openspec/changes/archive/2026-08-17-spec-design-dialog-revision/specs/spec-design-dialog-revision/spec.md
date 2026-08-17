# spec-design-dialog-revision — Delta Spec

## Purpose

Story/Design workspace 的 spec 生成采用对话式修订循环：AuthorConfirm 阶段用户以自由文本反馈驱动 author 增量修订，确认时可选送审或直接定稿；reviewer 降级为可选的只读建议来源，最终定稿权在用户。

## ADDED Requirements

### Requirement: AuthorConfirm 对话式修订循环

Story/Design workspace 在 AuthorConfirm 阶段必须接受用户自由文本反馈，并驱动 author 基于当前产物与反馈进行增量修订；修订完成后流程必须回到 AuthorConfirm 等待下一轮用户决策。

#### Scenario: 用户提交反馈触发增量修订
- **WHEN** Story/Design workspace 处于 AuthorConfirm 阶段，用户提交非空自由文本反馈
- **THEN** 系统进入 Revision 阶段，author 以"当前产物全文 + 用户反馈"为输入生成修订版（新版本，历史版本保留），修订完成后回到 AuthorConfirm，且对话流中展示本轮「改动摘要」

#### Scenario: 空反馈被拒绝
- **WHEN** 用户在 AuthorConfirm 阶段提交仅含空白字符的反馈
- **THEN** 系统返回校验错误，阶段与产物不变

#### Scenario: 多轮循环
- **WHEN** 修订完成后用户再次提交新反馈
- **THEN** 系统重复增量修订流程，修订轮次无上限，每轮产生新版本并保留全部历史版本

#### Scenario: 推倒重来出口被移除
- **WHEN** Story/Design workspace 在 AuthorConfirm 阶段收到 Reject 决策
- **THEN** 系统返回引导性错误（提示改用反馈修订表达重写意图），不清空产物、不回退到上下文准备阶段

### Requirement: 确认双出口与 review 默认值

用户确认 Author 产物时必须有两个出口：「确认并送审」与「确认定稿」；创建 workspace 时的 reviewer 配置决定两者的默认推荐项，但不锁死另一个出口。

#### Scenario: 配置启用 review 时默认推荐送审
- **WHEN** workspace 创建时启用了 reviewer，用户在 AuthorConfirm 点击「确认并送审」
- **THEN** 流程进入 CrossReview 执行 reviewer 评审；reviewer 配置仅作为默认推荐，用户仍可选择「确认定稿」跳过 review

#### Scenario: 未配置 reviewer 时凭创建快照临时送审
- **WHEN** workspace 创建时未启用 review 但创建快照中包含 reviewer 选择，用户在 AuthorConfirm 点击「确认并送审」
- **THEN** 本次评审按快照中的 reviewer 恢复执行（评审轮次设为 1），后续轮次的默认推荐仍为「确认定稿」

#### Scenario: 无任何 reviewer 选择时送审返回引导错误
- **WHEN** workspace 创建时未启用 review 且创建快照中未提供 reviewer 选择，用户在 AuthorConfirm 点击「确认并送审」
- **THEN** 系统返回引导性错误（提示确认定稿，或重新开始并启用 review），阶段与产物不变

#### Scenario: 旧变体 Accept 的兼容路由
- **WHEN** 客户端发送旧协议变体 `Accept`（无新变体能力的旧客户端）
- **THEN** 系统按创建时 review 默认值路由：已启用 review 等价于「确认并送审」，未启用等价于「确认定稿」（已 provisional 送审后再次 Accept 仍按创建默认值，不因已送审而改变）；无法追溯创建默认值的旧存量会话按当前有效 reviewer 配置路由

#### Scenario: 确认定稿直接完成
- **WHEN** 用户在 AuthorConfirm 点击「确认定稿」
- **THEN** 当前产物版本被标记为人工确认，workspace 进入 Completed，不再经过任何中间确认阶段

### Requirement: review 结果回对话流

reviewer 评审完成后，其报告必须作为消息进入对话流并回到 AuthorConfirm；reviewer 的结论不得直接驱动流程终结或自动返修。

#### Scenario: review 报告展示后回 AuthorConfirm
- **WHEN** reviewer 评审完成（无论结论为通过还是建议修订）
- **THEN** 评审报告以消息形式出现在对话流中，流程回到 AuthorConfirm，用户可基于报告继续反馈修订或点击「确认定稿」

#### Scenario: reviewer 通过不自动定稿
- **WHEN** reviewer 结论为通过
- **THEN** 系统不得自动进入 Completed，最终定稿仍由用户显式点击「确认定稿」完成

### Requirement: 存量会话恢复兼容

升级前停留在已退役中间阶段（HumanConfirm / ReviewDecision）的 Story/Design 存量会话，恢复时必须被安全引导。

#### Scenario: 停留在 HumanConfirm 的存量会话
- **WHEN** 恢复一个阶段为 HumanConfirm 的 Story/Design 存量会话
- **THEN** 系统将其引导回 AuthorConfirm（保留当前产物与历史消息），不丢失数据

#### Scenario: 停留在 ReviewDecision 的存量会话
- **WHEN** 恢复一个阶段为 ReviewDecision 的 Story/Design 存量会话
- **THEN** 系统将其引导回 AuthorConfirm 并携带已有评审报告，不丢失数据

### Requirement: 修订中断线恢复

修订 run（Revision 阶段）执行期间连接中断后重连时，系统必须将会话恢复到一致状态且不丢失数据。

#### Scenario: 修订 run 完成后重连
- **WHEN** 修订 run 已完成但连接在中断前未收到完成事件，用户重连
- **THEN** 系统将产物更新到修订后版本并回到 AuthorConfirm，对话流含本轮改动摘要

#### Scenario: 修订 run 未完成时重连
- **WHEN** 连接中断时修订 run 尚未完成，用户重连
- **THEN** 系统提供重试本轮修订的能力，产物保持在修订前版本，不产生部分写入的中间态
