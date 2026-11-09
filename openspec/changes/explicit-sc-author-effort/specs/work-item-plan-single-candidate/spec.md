# Spec Delta

## ADDED Requirements

### Requirement: Claude SC author 显式 effort 冻结策略（REQ-WSC-29）

平台 SHALL 为**新的 Claude SingleCandidate author 原运行**冻结 effort=`high`；同一原运行的教学、resume 与明确 fresh SHALL 使用**相同冻结策略**，不得从宿主新 settings 或部署默认重算。**原运行**＝同一 session record 上的同一冻结事实；**真实新生成**＝服务端**新建 session record** 的首次生成，只有它适用部署默认，旧 record MUST NOT 写入新默认。无冻结事实的存量运行 SHALL 保留旧继承模式并披露未知。冻结策略 SHALL 为服务端内部事实（策略版本、provider、author effort）并耐久保存，MUST NOT 暴露为公共 provider 配置项。作用域 SHALL 限定 Claude + SingleCandidate author；平台 SHALL 区分 requested 与 effective，MUST NOT 把组织 cap 后的 effective 冒称 `high`。客户端不得经恢复消息改策略。

#### Scenario: 新的 Claude SC 原运行冻结 high

- **WHEN** 平台为一次新的 Claude SingleCandidate author 原运行建立执行策略
- **THEN** 平台 SHALL 在该原运行上冻结 author effort=`high` 并耐久保存该冻结事实；该运行的 provider 启动 SHALL 携带该 requested 档位，MUST NOT 依赖宿主 settings 决定档位

#### Scenario: 宿主更高档位不覆盖新运行的显式档位

- **WHEN** 宿主环境把同一 provider 的 effort 设为比 `high` 更高的档位，且平台为新的 Claude SC 原运行已冻结 effort=`high`
- **THEN** 该原运行的 requested 档位 SHALL 仍为 `high`；宿主设置 MUST NOT 抬高或改写该原运行的冻结档位

#### Scenario: 教学、resume 与 fresh 三路复用同一冻结策略

- **WHEN** 同一原运行发生编译/IR/结构教学重驱、native resume 或明确 fresh
- **THEN** 这三路 SHALL 使用与原运行相同的冻结策略与相同 requested 档位；MUST NOT 从当前部署默认或宿主新 settings 重新计算，MUST NOT 因为换了一条执行路径而换档

#### Scenario: 作用域外角色与 provider 档位不变

- **WHEN** 受影响面为 reviewer、Story/Design author、coding 或任何非 Claude provider
- **THEN** 这些执行路径的档位 SHALL 与变更前一致，MUST NOT 因本策略被注入 `high` 或任何显式档位

#### Scenario: SC 人工修订出口沿用同一冻结策略

- **WHEN** 同一原运行进入 SC 人工修订出口（`sc_manual_revision.rs:356-366`）
- **THEN** 该出口 SHALL 使用与原运行相同的冻结策略与相同 requested 档位，MUST NOT 因换了出口而换档

#### Scenario: 存量运行保留旧继承模式并披露未知

- **WHEN** 一个已存在的运行没有可证明的冻结事实（冻结字段缺失）
- **THEN** 平台 SHALL 保留该运行的旧继承语义，MUST NOT 补写冻结快照、不得声称其原档位为 `high`；平台 MUST 以「未知」披露该运行的 effective 档位，且 MUST NOT 声称旧运行的 effective 已被追溯冻结

#### Scenario: 兼容例外的实际档位仍受宿主影响

- **WHEN** 存量运行处于旧继承模式，且宿主在此后改变了同一 provider 的 effort 设置
- **THEN** 该运行的实际 effective 档位 MAY 随之改变；平台 MUST NOT 对外宣称该运行的 effective 已被冻结或固定，`high` 的冻结承诺 MUST NOT 追溯适用于它

#### Scenario: requested 与 effective 不得混称

- **WHEN** 平台观测某次 SC author provider 启动
- **THEN** 记录 MUST 包含 requested 档位；若无 provider 支持能力或请求证据证明实际生效档位，effective MUST 记为未知，MUST NOT 以 `high` 代替未知

#### Scenario: 客户端不得修改冻结策略

- **WHEN** 客户端在恢复消息、新生成消息或任何控制消息中表达或尝试表达 effort 档位
- **THEN** 平台 MUST 忽略该表达并使用服务端冻结值；MUST NOT 把它当作 provider 配置变更接受、不得据此新建或改写原运行的冻结事实

#### Scenario: 恢复不得改档也不得重置授权与预算

- **WHEN** 原运行因 native 会话指纹漂移进入等待并由有权 driver 明确 fresh
- **THEN** fresh SHALL 只更换 native 会话，MUST NOT 更换 effort 档位，MUST NOT 重置原运行的授权、预算或预留事实；下一次**真实新生成**才使用届时的新默认档位

#### Scenario: 创建时 provider 与准入时 author_provider 不一致按无冻结事实处理

- **WHEN** 创建时写入冻结策略的 provider 与该原运行真实 SC 首次生成准入时的 author_provider 不一致
- **THEN** 平台 MUST NOT 静默沿用该档位，MUST 按「无冻结事实」处理（披露未知、不注入平台覆盖），MUST NOT 以创建时值覆盖准入时值

#### Scenario: 同一 session 重臂仍属原运行

- **WHEN** 同一 session record 上的重臂、resume、native fresh 或教学 attempt 再次触发 SC author 启动
- **THEN** 平台 SHALL 视其为同一**原运行**并继续使用同一冻结策略与同一 requested 档位，MUST NOT 写入当前部署默认，MUST NOT 因新 session ID 换档

#### Scenario: 新建 session 适用新默认

- **WHEN** 服务端**新建 session record** 并对其首次生成建立执行策略
- **THEN** 该首次生成即**真实新生成**，SHALL 适用当前部署默认（本期为 `high`）；MUST NOT 被追溯适用于既有 session record 上的重臂或恢复

#### Scenario: 冻结策略的 policy_version 不可识别时零启动
- **WHEN** 冻结策略引用的 policy_version 在当前部署不可识别
- **THEN** 平台 SHALL 零 provider 启动并保留原记录，MUST NOT 回落当前部署默认或宿主继承继续启动，MUST NOT 改写原记录中的冻结事实

#### Scenario: 上下文与缓存控制变量同属冻结策略且只作用于 SC author 出口
- **WHEN** 平台为同一原运行装配 `MAX_MCP_OUTPUT_TOKENS`、`BASH_MAX_OUTPUT_LENGTH` 与 `CLAUDE_CODE_PROMPT_CACHE_TTL`
- **THEN** 三者 SHALL 与 effort 档位同属该原运行的**冻结执行策略**，初轮、compile/IR/structure 教学、native resume/fresh 共用**同一组取值**，MUST NOT 每次启动重读宿主或平台新默认；该组变量 SHALL 只在 Claude SingleCandidate author 出口生效，其它出口（Story/Design 通用 author、Story/Design 修订、reviewer、coding）MUST NOT 设定；`CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS` 本期 MUST NOT 设定

#### Scenario: 控制变量取值变更只对新生成授权生效
- **WHEN** 平台调整了输出上限或缓存 TTL 的取值
- **THEN** 变更 SHALL 只对**新的合法生成授权**（新建 session record）生效，在途运行的既有取值 MUST NOT 被改写，MUST NOT 借此重置预算或清空预留
