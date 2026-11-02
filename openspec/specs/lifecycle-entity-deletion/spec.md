# lifecycle-entity-deletion Specification

## Purpose
规定 Story、Design 和 Plan 的实体永久删除边界：物理清除其拥有的业务与审计内容，保留不含内容的删除事实，并以存活引用、运行互斥、分级确认和可重试失败处理防止悬空引用、误伤与删除后复活。

## Requirements

### Requirement: Story 与 Design 永久删除自己的完整闭包（REQ-DEL-01）

系统 MUST 在允许删除 Story 或 Design 后物理清除其主记录、全部版本、所属会话及 timeline/checkpoint/消息/锁等附属内容、关联 review 和明确归属 provider-run 档案；Confirmed 或曾发布不豁免内容清除。删除 MUST NOT 自动删除其上游或下游实体。

#### Scenario: Story 和 Design 分别清理自己拥有的内容
- **WHEN** 无存活后置引用的 Story 或 Design 满足删除门禁，且需要的确认已完成
- **THEN** 系统 MUST 清除该实体的所有自有内容及关联 review/run，而 issue、其他实体、仓库注册与 LC trust 数据保持不变

#### Scenario: 不虚构专用源存储
- **WHEN** 实体的源文本和报告实际存放在其版本或会话附属对象，而非独立 source store
- **THEN** 系统 MUST 清除这些真实所属对象，不得将其他实体的 source/IR/report 当成该实体的拥有对象删除

### Requirement: 每个删除实体仅留下无内容的删除记录（REQ-DEL-02）

系统 MUST 为成功永久删除的实体持久保存恰一条删除事实，记录仅含业务 scope、实体种类及 ID、删除时间和完成状态，不含标题、正文、source/IR/report/review、原 hash 或恢复副本。重复请求 MUST 幂等返回已删除，不新增记录；该记录 MUST NOT 冒充活跃实体或授权迟到写入恢复内容。

#### Scenario: 删除事实与内容分离
- **WHEN** 实体完整拥有闭包已被清除
- **THEN** 该实体仅有一条无内容删除记录，活动列表及 workspace attach 不再将其视为可编辑实体

#### Scenario: 重试不重复留痕
- **WHEN** 客户端在删除完成后重发同一 scope 和实体身份的删除请求
- **THEN** 系统 MUST 返回幂等已删除结果，原删除时间及记录数量不变，且不得复活内容

#### Scenario: 失败不假记完成
- **WHEN** 任一真实清理或删除事实落盘失败
- **THEN** 系统 MUST 返回可定位错误，不得保存已完成的虚假删除事实或返回成功

### Requirement: 被存活后置对象引用的 Story 或 Design 拒删（REQ-DEL-03）

系统 MUST 在任何删除动作前核验 Story/Design 的存活 incoming references，包括 Design、Plan、生命周期 Work Item 和发布 lineage 中的真实引用。存在引用时 MUST 拒绝删除，给出稳定 `entity_referenced` 错误和引用者类型/ID，并提示先删除后置对象；不得自动跨级删除或改写引用。

#### Scenario: Story 被 Design 或 Plan 引用
- **WHEN** 待删 Story 被任一存活 Design、Plan、Work Item 或发布 lineage 引用
- **THEN** 系统 MUST 拒删并列出实际阻挡对象，原实体、所有版本和审计内容均保持不变

#### Scenario: Design 被发布记录或 Work Item 引用
- **WHEN** 待删 Design 被仍存活 Plan 的发布 lineage 或生命周期 Work Item 引用，即使当前 Plan 摘要已不包含该 ID
- **THEN** 系统 MUST 拒删并提示先删相应后置对象，不得仅依据当前摘要放行

#### Scenario: 先删除后置再删除上游
- **WHEN** 后置对象已按自身完整闭包删除，剩余扫描无存活 incoming reference
- **THEN** 系统 MUST 允许上游进入其分级确认/运行门禁流程，删除记录本身不构成业务引用

### Requirement: 删除分级确认以服务端真实历史为准（REQ-DEL-04）

系统 MUST 让可证从未确认的 Draft 删除零确认提示；当前或曾经 Confirmed 的实体 MUST 在破坏性删除前弹且仅弹一次确认框，明确说明内容、所属工作项和 review 将永久清除且不可恢复。确认资格 MUST 由服务端真实历史与单调确认事实判定，不得仅相信当前 Draft 或确认参数。兼容注记：无法判定历史的旧 Draft MUST 按一次确认处理，MUST NOT 伪推从未确认；不改变内容确认/发布 CAS。

#### Scenario: 从未确认的 Draft 不弹窗
- **WHEN** 用户删除一个新路径有单调事实证明从未 Confirmed、且无阻挡的 Draft
- **THEN** 系统 MUST 直接执行删除，不显示确认框，不额外发送“确认删除”第二步

#### Scenario: 曾确认后回到 Draft 仍只确认一次
- **WHEN** 实体当前为 Draft 或 ChangeRequested，但存在过去确认版本、确认事件或发布事实
- **THEN** 系统 MUST 在删除前显示一次完整闭包警告；用户同意后一次删除完成，不为子 session、WI 或 review 分别弹窗

#### Scenario: 旧 Draft 确认历史未知时保守一次确认
- **WHEN** 旧 Draft 无可靠单调确认字段，现存版本/发布/会话事实也不能证明曾经或从未确认
- **THEN** 系统 MUST 要求一次确认，不把未知历史推断为未确认；不批量伪造旧字段，新路径可证从未确认仍零提示

#### Scenario: 用户取消确认
- **WHEN** 用户取消 Confirmed 实体的删除确认
- **THEN** 系统 MUST 不发出破坏性删除，不清任何对象，也不新增删除记录

#### Scenario: 状态在读取后变为需确认
- **WHEN** 客户端按旧 Draft 状态提交未确认删除，而服务端已存在确认事实
- **THEN** 系统 MUST 以稳定 `deletion_confirmation_required` 响应拒绝本次破坏性动作，前端随后仅显示一次确认框，不绕确认资格

### Requirement: coding 后置存在时拒绝而非中止（REQ-DEL-05）

系统 MUST 在删除任何闭包前检查目标 Plan/group 及所属 Work Item 的 coding attempt；只要对应 attempt 记录存在，无论是否运行中，MUST 以既有 `coding_workspace_exists` 拒删并提示先删除 coding workspace，保留 plan_id/attempt_id 细节。MUST NOT 自动 abort 或删除 attempt；无 attempt 的残留 lock 不构成阻挡。

#### Scenario: 后一个 Work Item 有 coding attempt
- **WHEN** Plan 的多个所属 Work Item 中，前一个无 attempt 而后一个存在 attempt
- **THEN** 系统 MUST 在删除任何 Work Item、session 或内容前拒绝，完整 Plan 闭包、attempt 与删除记录均保持不动

#### Scenario: 正在 coding 的后置不得被级联
- **WHEN** 用户尝试删除仍有 coding 后置的 Plan，或删除被该 Plan 引用的 Story/Design
- **THEN** 系统 MUST 提示先删后置对象，不以终止 coding 或自动跨级清除代替拒删

#### Scenario: coding 创建与删除并发只有一个赢家
- **WHEN** 同 Plan 的 coding attempt 正在创建，而另一个请求尝试删除该 Plan
- **THEN** 创建先赢则删除 MUST 忙或按 attempt 存在拒绝且零破坏；删除先赢则创建 MUST 拒绝，不得在删除门禁检查之后再留下新的后置 attempt

### Requirement: 删除与生成修订及 compile 互斥且无复活（REQ-DEL-06）

系统 MUST 对同scope删除与生成、修订、review、compile/publish、advance/retry/初始化恢复的启动及耐久提交实行互斥。破坏性清理前 MUST 阻止新启动并阻挡已有writer；冲突响应无部分删除。接受删除或已删除身份的旧连接、manager、迟到writer及旧Ready事实 MUST 不能重建文件或重新授权；无关实体运行不受影响。

#### Scenario: 同 scope writer 运行时拒删
- **WHEN** 同scope仍有可落盘的生成、review、compile或advance/retry/初始化恢复writer
- **THEN** 系统 MUST 返回稳定 `entity_busy` 与先等待该运行收口的提示，零删除；不得仅摘除 registry 条目就认为运行已停止

#### Scenario: 删除赢得准入
- **WHEN** 删除已获得同scope独占准入，另请求尝试启动生成、compile或advance/retry
- **THEN** 新请求 MUST 被阻挡，不创建 provider 子调用或耐久产物；删除完成后旧身份也不得写回

#### Scenario: 旧连接与迟到提交
- **WHEN** 已删除实体的旧连接/缓存manager尝试提交source/session/review/compile、advance/retry结果或shared-worktree owner状态
- **THEN** 系统 MUST 拒收提交，删除闭包保持不存在，其他实体及 trust/确认 CAS 保持原合同

### Requirement: 先清子树后删根并支持失败重入（REQ-DEL-07）

系统 MUST 在所有拒删门禁通过后才开始清理，按真实拥有关系清理子树，根实体最后删除。对象缺失 MUST 视为已清，真实 IO/JSON/归属错误 MUST 返回可定位错误；接受后的部分失败 MUST 可按原 scope 重入完成，不假称跨 store 原子事务。磁盘清理与内存失效 MUST 消费同一所属集合。

#### Scenario: 子树清理失败保留重试入口
- **WHEN** source/revision/review/session 等任一子树清理发生真实 IO 失败
- **THEN** 系统 MUST 返回错误，保留可重入删除事实或根入口；下一次请求 MUST 能继续清到完整闭包而非因根已删不可达

#### Scenario: 最后一步附近崩溃
- **WHEN** 子树已清，服务在根删除或删除记录完成之间中断
- **THEN** 重入 MUST 从耐久 scope/身份事实完成根删除与单条记录，不新增副本或把已删除 ID 重新分配为旧实体

#### Scenario: 半残子对象和完整内存集合
- **WHEN** Plan 摘要缺少某所属 Work Item 或 session，但该对象仍有真实 plan binding
- **THEN** 系统 MUST 将其纳入磁盘清理和 manager 失效集合，不遗漏也不误删另一 Plan 的同名逻辑 Work Item
