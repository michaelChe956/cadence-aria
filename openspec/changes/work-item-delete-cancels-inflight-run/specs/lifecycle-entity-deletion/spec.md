## MODIFIED Requirements

### Requirement: 删除与生成修订及 compile 互斥且无复活（REQ-DEL-06）

系统 MUST 对同scope删除与生成、修订、review、compile/publish、advance/retry/初始化恢复的启动及耐久提交实行互斥。破坏性清理前 MUST 阻止新启动并阻挡已有writer；冲突响应无部分删除。**★例外：同scope writer 恰好是该实体自身绑定的自动化 run 时，删除路径 MUST 先取消该 run 并等待其真实收口（见 REQ-WIGA-15），再取删除独占；此支 MUST NOT 以 `entity_busy` 长期阻塞删除★。** 接受删除或已删除身份的旧连接、manager、迟到writer及旧Ready事实 MUST 不能重建文件或重新授权；无关实体运行不受影响。

#### Scenario: 同 scope writer 运行时拒删

- **WHEN** 同scope仍有可落盘的生成、review、compile或advance/retry/初始化恢复writer
- **THEN** 系统 MUST 返回稳定 `entity_busy` 与先等待该运行收口的提示，零删除；不得仅摘除 registry 条目就认为运行已停止
- **AND** ★例外：该 writer 是该实体自身绑定的自动化 run 时，按「实体自身被终止的自动化 run 不长期阻塞删除」处理★

#### Scenario: 实体自身被终止的自动化 run 不长期阻塞删除

- **WHEN** 删除某实体，且同scope唯一在跑的 writer 是该实体自身绑定的自动化 run（删除路径已对该实体的自动化登记执行终止）
- **THEN** 删除路径 MUST 先取消该 run 并等待其真实收口，随后 MUST 取得删除独占并完成删除
- **AND** MUST NOT 以 `entity_busy` 长期阻塞该次删除
- **AND** MUST NOT 在该 run 仍在跑时开始破坏性清理；取消未收口时 MUST 明确报出占用者

#### Scenario: 删除赢得准入

- **WHEN** 删除已获得同scope独占准入，另请求尝试启动生成、compile或advance/retry
- **THEN** 新请求 MUST 被阻挡，不创建 provider 子调用或耐久产物；删除完成后旧身份也不得写回

#### Scenario: 旧连接与迟到提交

- **WHEN** 已删除实体的旧连接/缓存manager尝试提交source/session/review/compile、advance/retry结果或shared-worktree owner状态
- **THEN** 系统 MUST 拒收提交，删除闭包保持不存在，其他实体及 trust/确认 CAS 保持原合同
