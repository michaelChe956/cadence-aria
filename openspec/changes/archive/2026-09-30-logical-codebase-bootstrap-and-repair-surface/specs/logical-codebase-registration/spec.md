# Spec Delta

## MODIFIED Requirements

### Requirement: 逻辑代码库成员登记（REQ-REG-01）

系统 SHALL 为逻辑代码库建立唯一、可审计的 authority 路由：明确逻辑代码库身份时，成员、checkout、manifest、selection、policy 与 index 只能从该逻辑代码库的权威子树解析；单仓代码库只能从其合法物理仓布局解析。系统 SHALL 禁止按路径猜测、跨逻辑代码库回退或把旧项目级布局静默当作当前 authority。发现旧布局、authority 冲突、路径越界、来源身份不一致或无法唯一解析时，系统 SHALL fail-closed，返回可操作的迁移/核验等待项且不得产生成员或仓库写入。

#### Scenario: 扫描公共父目录登记成员

- **WHEN** 用户提供公共父目录并触发登记
- **THEN** 系统 SHALL 扫描发现全部子 git 仓，逐项校验（canonical path、git root、非嵌套、非重复），并为每个成员创建 CodebaseMemberRecord 与 RepositoryRecord，全程不修改任何仓库内容

#### Scenario: 成员身份与去重

- **WHEN** 同一仓库以不同路径或别名重复提交
- **THEN** 系统 SHALL 按 canonical git-dir/source identity 去重，不产生重复成员，并返回重复原因

#### Scenario: 新逻辑代码库使用唯一 authority

- **WHEN** 用户创建一个没有历史 seed 的逻辑代码库并提交成员登记
- **THEN** 所有后续 selection、policy、checkout 与 aggregate index 引用 SHALL 指向同一逻辑代码库 authority；不得要求用户复制成员文件或先手工建立索引

#### Scenario: 旧布局与 LC authority 冲突

- **WHEN** 请求同时发现项目级历史布局和逻辑代码库子树，且两者的成员或来源身份不一致
- **THEN** 系统 SHALL 返回冲突诊断及迁移/核验操作，保持当前权威事实不变，不猜测路径、不自动合并、不切换读取来源
### Requirement: 冷启动登记的可重入步骤与操作面（REQ-REG-10）

系统 SHALL 将全新逻辑代码库的首次可用过程暴露为可观察、可重入的步骤事实，至少区分 identity、manifest/checkout、rules/policy、member index 与 aggregate index active；每个步骤 SHALL 具有未开始、进行中、已完成或失败/等待状态、稳定对象身份、checkpoint/失败原因和允许的准备、继续、重试或核验操作。操作必须绑定目标与预期版本，重复同一命令键 SHALL 返回同一结果，过期或错误对象 SHALL fail-closed。GET/只读投影 SHALL 只返回状态、缺失材料、已完成 checkpoint 与允许操作，MUST NOT 启动 provider、checkout、索引或其他副作用。

#### Scenario: 全新 LC 无手工铺底完成首建

- **WHEN** 用户从产品面创建一个没有 active index、规则或 selection seed 的逻辑代码库并触发首建
- **THEN** 系统 SHALL 按 identity→manifest/checkout→rules/policy→member index→aggregate index active 的依赖顺序自动推进；每步完成事实可查询，最终进入 PlanningReady，成员仓登记不产生 Git 写副作用

#### Scenario: 首建中断后继续

- **WHEN** 首建在任一步骤产生可诊断失败或用户关闭页面后再次查询
- **THEN** 系统 SHALL 保留已完成步骤与失败证据，展示针对该步骤的继续/重试/准备操作；用户执行允许操作后 SHALL 从 checkpoint 继续，已完成步骤与 provider turn 不重复执行

#### Scenario: 只读查询不触发首建

- **WHEN** 客户端反复调用逻辑代码库详情、成员、policy 或 aggregate index 状态查询
- **THEN** 系统 SHALL 只返回 durable 投影和允许动作，不启动 provider、不创建 index、不修改 manifest/checkout/rules 或步骤状态

### Requirement: Failed identity journal 的产品化修复（REQ-REG-11）

系统 SHALL 提供不依赖普通成员列表或成功身份解析的 repair/diagnostic 入口，允许用户读取 Failed identity journal 的 source digest、已完成键、read mode、当前映射和冲突详情。对 digest 可证明一致的安全前缀，系统 MAY 提供继续 repair 操作；mapping 冲突 SHALL 要求用户提交明确 mapping 并执行再次核验。repair 成功前 SHALL 保持原 journal 与权威 JSON 不变、禁止删除 journal、禁止直接改数据、禁止切换到未确认的成员或来源身份；未完成裁决 SHALL 保持等待并可重复读取。

#### Scenario: Failed journal 安全前缀继续

- **WHEN** 用户打开一个 Failed identity journal，source digest 与安全前缀核验一致且不存在 mapping 冲突
- **THEN** 产品 SHALL 展示已完成键和可继续范围；用户确认继续后保留原失败记录并追加可审计 repair 事实，随后回到原登记/读取链

#### Scenario: mapping 冲突须人工确认

- **WHEN** Failed journal 检测到多个候选 mapping 或 source digest 不一致
- **THEN** 产品 SHALL 展示候选、digest 和影响范围，要求用户提交 mapping 并重新核验；用户确认前 SHALL 不切换读取 authority、不产生成员变更、不启动 provider

### Requirement: 规则与 capability 缺失的准入预检（REQ-REG-12）

逻辑代码库首次登记、索引或 provider 准入前，系统 SHALL 预检实际 provider 将消费的规则材料、聚合 policy 引用和 capability/gateway 条件。材料缺失、不一致、过期或 provider 能力不满足时，系统 SHALL 在 provider 启动前返回可诊断等待项，通知原因、目标、缺失材料和准备/重试操作；准备完成后 SHALL 回到原步骤继续。系统 MUST NOT 通过写入一条 capability 记录、复制一次成员规则或绕过 gateway 将缺失能力视为可用。

#### Scenario: 成员规则缺失

- **WHEN** 规则消费前检查发现成员所需规则材料缺失
- **THEN** 系统 SHALL 在真实 provider 启动前停等并提供产品化准备/重试操作；修复后使用实际规则来源继续，不将运行时的 Failed 错误作为首次告知

#### Scenario: gateway 能力不满足

- **WHEN** capability 检查发现当前 provider/gateway 组合不被支持
- **THEN** 系统 SHALL 拒绝准入并通知合法配置/目标选择，不启动 provider、不伪造 capability 成功、不回落到另一套路径

### Requirement: 首建失败的可操作通知（REQ-REG-13）

每个冷启动或 repair 等待项 SHALL 通过既有驾驶舱 inbox/系统通知投影失败原因、已完成步骤、逻辑代码库与成员身份、步骤/operation 身份、可能的外部副作用、可用按钮会做什么及成功后的下一阶段。通知投递失败不得回滚业务事实，客户端 SHALL 能通过只读查询补读同一等待项；用户操作成功后 SHALL 调用原有登记/索引/编排链继续，而不是创建第二套持久化操作状态机。

#### Scenario: 失败通知后产品操作续进

- **WHEN** member index 或 aggregate index 步骤失败且通知可投递
- **THEN** 用户 SHALL 能从通知或补读页面触发安全重试/继续，系统保留失败事实并在成功后推进到下一 checkpoint

#### Scenario: 通知投递失败仍可补读

- **WHEN** 业务步骤已持久化失败但 inbox/系统通知投递失败
- **THEN** 详情/状态查询 SHALL 返回同一失败原因和允许操作；补读或重试通知不得重复执行步骤
