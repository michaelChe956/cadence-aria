## ADDED Requirements

### Requirement: 删除工作项组时的终止必须取消该实体绑定的在途 run（REQ-WIGA-15）

删除工作项组时，删除路径终止该实体绑定的自动化登记后，SHALL 同时**取消该实体绑定的 workspace session 的在途 run，并等待该 run 的真实收口（真实 completion）**，使删除独占可获得；删除 SHALL NOT 因**该实体自身**的在途 run 而以 `entity_busy` 长期阻塞。

取消 SHALL 沿用既有的非 WebSocket 取消入口，SHALL NOT 新造第二套终止语义或只摘除 run 登记而不等收口。取消范围 SHALL **严格限定**在该实体绑定的 session（按 `entity_id` 精确匹配）。

该实体**没有**在途 run 时，删除行为 SHALL 与既有语义**完全一致**（不引入额外等待、不引入额外副作用、不改变删除结果与既有错误码）。

#### Scenario: 删除时先取消在途 run 再删

- **WHEN** owner 删除一个工作项组，而该实体绑定的 workspace session 上存在一个在途 provider run
- **THEN** 删除路径 SHALL 先取消该 run 并等待其真实收口，随后 SHALL 取得删除独占并完成删除
- **AND** 该次删除 SHALL NOT 以 `entity_busy`（或同义占用错误）失败
- **AND** 取消后 SHALL NOT 出现该 run 继续向 provider 输出

#### Scenario: 取消范围不越界

- **WHEN** 删除某实体，且同一 issue 下另一个实体（或另一个 session）上存在在途 run
- **THEN** 该 run SHALL 保持运行、SHALL NOT 被取消
- **AND** 取消动作 SHALL 仅作用于 `entity_id` 与被删实体精确匹配的 session

#### Scenario: 没有在途 run 时删除语义不变

- **WHEN** owner 删除一个工作项组，且该实体绑定的 session 上没有在途 run
- **THEN** 删除行为 SHALL 与本 requirement 引入前完全一致（不额外等待、不产生额外副作用、结果与错误码不变）

#### Scenario: 取消未收口时如实报占用者

- **WHEN** 删除路径已发出取消，但该 run 在兜底超时内仍未收口
- **THEN** 删除 SHALL 返回明确指出占用者的可诊断错误，SHALL NOT 假称 run 已停止
- **AND** SHALL NOT 在该 run 仍在跑时开始破坏性清理

#### Scenario: 既有终止与删除前置语义不被弱化

- **WHEN** 删除路径执行本 requirement 的取消动作
- **THEN** SHALL NOT 写入 enrollment、不新建 plan/session、不启动 provider、不复活被终止的 plan（故与 REQ-WIGA-09「绑定实体删除流程再次运行 ⇒ 零副作用」不冲突）
- **AND** SHALL NOT 物理删除 enrollment 记录（REQ-DRG-18、REQ-WIGA-VOID-01）
- **AND** SHALL NOT 改动人工终止路径（REST terminate / WebSocket abort）的既有行为、顺序与文案
- **AND** SHALL NOT 新增或放宽任何删除前置条件；owner 既有裁决「执行中禁删保持现状」仍成立——run 在跑时不得删，本 requirement 只是让终止**先把 run 停掉**

### Requirement: 被中止的 run 不得表述为失败，且其 stage 归一为终态阶段（REQ-WIGA-16）

run 被取消（中止）时，其时间线节点 SHALL **NOT** 以「失败」表述「中止」：节点 `status` SHALL NOT 为 `failed`，SHALL 使用与既有词汇一致、可与失败区分的「中止」表达（既有 `ProviderStatus` / `ExecutionEventStatus` 已使用 `aborted`，SHALL NOT 另造冲突词汇）；节点 `summary` SHALL 表达「已中止」而非失败原因。

带 provider agent 的 provider run 节点进入终态（中止 / 失败 / 完成 / 跳过）时，其 `stage` SHALL NOT 仍为进行中阶段 `running`，SHALL 归一为终态阶段 `completed`；不带 provider agent 的工作流节点（compile、门卡、上下文类）的 `stage` SHALL 继续表示其所属工作流阶段，SHALL NOT 因本 requirement 被改写，既有断言逐字不变。

#### Scenario: 中止后的节点不表述为失败

- **WHEN** 某个 run 被取消（中止）并收口
- **THEN** 其时间线节点的 `status` SHALL NOT 为 `failed`，SHALL 表达为「中止」
- **AND** `summary` SHALL 表达「已中止」，SHALL NOT 表述为失败原因
- **AND** `completed_at` SHALL 落盘

#### Scenario: 被中止的 run 只额外约束 status 与 summary 的表达

- **WHEN** 某个 run 被取消（中止）并收口
- **THEN** 本 requirement 对被中止的 run 只额外约束 `status` 与 `summary` 的表达：`status` SHALL NOT 为 `failed`，`summary` SHALL 表达「已中止」。

#### Scenario: 真实失败仍表述为失败

- **WHEN** 某个 run 因真实错误（provider 失败、权限超时等）收口，或某个与 run 中止无关的时间线节点被更新
- **THEN** 该节点 SHALL 继续按既有判定与既有文案表达，既有失败/跳过/完成三态语义与断言 SHALL 逐字不变
- **AND** 本 requirement SHALL NOT 改动 run 的收口顺序与完成度信号语义

### Requirement: 取消与取锁的时序保证及取消结果不得谎称（REQ-WIGA-17）

取消完成后删除独占 SHALL 最终取得成功；取消与取锁之间的锁释放时序 SHALL 由实现保证，SHALL NOT 依赖未验证的时序假设。

取消调用 SHALL 检查其返回值：返回「无活跃 run」属正常结果；返回「已取消」而该 run 实际未收口（兜底超时）时，删除 SHALL NOT 谎称已取消该 run，SHALL NOT 因此阻塞删除。

#### Scenario: 取消后删除独占必须最终取得成功

- **WHEN** 取消已发出且该 run 已真实收口
- **THEN** 删除独占 SHALL 最终取得成功，删除 SHALL NOT 仍以 `entity_busy` 失败
- **AND** 取消与取锁之间的锁释放时序 SHALL 由实现保证，SHALL NOT 依赖未验证的时序假设：completion 信号已到而 scope 共享锁尚未释放的尾隙，SHALL 以**有界**短退避补足——每轮一次「取删除独占并立即释放」的探测（探测不保留 lease、不触碰任何破坏性清理），Busy/Deleting 才重试，至多 10 轮、每轮间隔 50ms（合计 ≤500ms），SHALL NOT 无限等待或长挂起

#### Scenario: 取消未真正收口时不得谎称已取消

- **WHEN** 取消调用返回「已取消」但该 run 并未真正收口（兜底超时）
- **THEN** 删除 SHALL NOT 宣称已取消该 run、SHALL NOT 记录任何与之矛盾的「已停止」事实
- **AND** 删除 SHALL NOT 因此阻塞删除（仍按既有独占判定推进；拿不到独占时按既有语义报出占用者）
- **AND** 取消调用返回「无活跃 run」SHALL 属正常结果，SHALL NOT 被当作失败
