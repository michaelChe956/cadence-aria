# Spec Delta

## MODIFIED Requirements

### Requirement: 明确 fresh 只换 native 会话

系统 SHALL 仅接受有权 driver 携当前 recovery_id、intent=fresh 与非空 operation_id 的明确动作，重新完整 fresh admission 并使新输入旧 resume ID 为空。该动作 SHALL 保留 workspace/entity、原产物/反馈/可信评审/有效历史/规划事实，按原 run-context 产完整新 artifact 并回原合法门，不增加自动重试或替代动作授权。

明确 fresh 只更换 native 会话，**内部执行策略亦按原运行冻结**：原运行已冻结的 provider 执行策略（含 author effort 档位）SHALL 在 fresh 后保持不变，MUST NOT 随 fresh 更换、MUST NOT 按当前部署默认或宿主新 settings 重算，MUST NOT 因此重置原运行的授权、预算、预留或 attempt 事实。客户端消息 MUST NOT 携带或修改该内部执行策略；平台 SHALL 以服务端冻结事实为准。

存量运行**没有**冻结事实时，平台 SHALL 保留旧继承语义并按「未知」披露，MUST NOT 追溯改档、MUST NOT 补造历史冻结快照；只有下一次**真实新生成**才使用届时的新默认策略。兼容例外 MUST 被披露：存量继承模式的**实际 effective 档位仍可能受宿主变化影响**，平台 MUST NOT 宣称旧运行的 effective 已被追溯冻结。

#### Scenario: 真 fresh 保留修订上下文
- **WHEN** 有权用户明确选择启动新会话继续当前修订且完整 admission 通过
- **THEN** 系统 SHALL 只创建一个不同的新 native ID，fresh 输入不含旧 resume ID，完整 prompt 包含原 artifact、feedback、trusted review 与必要上下文；原 workspace/entity 不变并产完整新 artifact

#### Scenario: admission 不通过仍等待
- **WHEN** 明确 fresh 后 policy/trust/capability/材料/版本等门不满足
- **THEN** 系统 SHALL 零 provider session-start，保存该 operation 的真实失败/未启动结果并回 waiting，不自动 fresh、不自动登记 trust

#### Scenario: 恢复不换配置或身份
- **WHEN** fresh 消息更换 provider 配置、跨 workspace/scope、recovery 身份过期或原业务 source/hash 已变
- **THEN** 系统 SHALL 拒绝，不使用前端身份/root/provider覆盖服务端事实，不降为普通 author 或新 workspace

#### Scenario: fresh 不换内部执行策略
- **WHEN** 原运行已冻结内部执行策略并进入等待，随后有权 driver 明确 fresh 且完整 admission 通过
- **THEN** 系统 SHALL 在新 native 会话上复用同一冻结策略与同一 requested 档位，MUST NOT 按当前部署默认或宿主新 settings 重算，MUST NOT 因 fresh 更换档位

#### Scenario: fresh 不重置授权与预算
- **WHEN** 明确 fresh 触发一次新的 provider 启动
- **THEN** 系统 SHALL 保留原运行的授权、预算余额、预留与 attempt 事实，MUST NOT 因 fresh 视为新的授权来源、MUST NOT 借 fresh 重置预算或增删 attempt

#### Scenario: 旧记录缺冻结事实不追溯改档
- **WHEN** 恢复所依据的原运行记录没有可证明的冻结事实（冻结字段缺失）
- **THEN** 系统 SHALL 保留旧继承语义并以「未知」披露，MUST NOT 补写冻结快照、不得声称其原档位为新默认；下一次真实新生成才使用届时的新默认策略

#### Scenario: 兼容例外不得被宣称已冻结
- **WHEN** 存量运行处于旧继承模式且宿主此后改变了同一 provider 的档位设置
- **THEN** 该运行的实际 effective 档位 MAY 随之改变；系统 MUST NOT 对外宣称其 effective 已被冻结或固定，MUST NOT 把新默认档位追溯适用于该运行

#### Scenario: 客户端不得借恢复消息改策略
- **WHEN** 客户端在 fresh 或恢复消息中表达 effort 档位或等价执行策略
- **THEN** 系统 SHALL 忽略该表达并使用服务端冻结事实，MUST NOT 把它当作 provider 配置变更接受、MUST NOT 据此新建或改写原运行的冻结事实