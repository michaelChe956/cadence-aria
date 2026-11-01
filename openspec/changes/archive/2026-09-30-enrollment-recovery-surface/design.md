# Design

## Context

本 change 只落方案 v1.2 C1 的契约边界，动机与范围见 `proposal.md`。现有系统已具备 enrollment 的 durable CAS/绑定写面、plan gate 与 candidate/recovery 事实、advance journal、生命周期 session/child 查询、work item contract 校验以及驾驶舱只读投影；方案 v1.2 §2 的 S01–S13、S24 将这些作为复用基础，而不是假定按钮或恢复动作已经存在。

上位追溯：

- §3.1 定义统一“错误→通知→用户操作→原链继续”的闭环；§3.2 限制 generation 仅是用户明确换代产生的绑定版本；§3.3 要求展示身份/副作用并对未知副作用 fail-closed。
- §4.2 的 #1/#6/#9 与 §4.3 的 #11/#12/#14/#16 是本 change 的故障面；对应 A07、A09、A12、A13 验收见 §6.2。
- §5.1 是 C1 实施合同；§5.5 要求 C1 先冻结 target/binding 与 plan 操作面供 C5 消费；§5.6（以 §5.1 的“受影响 specs”清单列出）要求同步 autopilot、conversational gate、advance、single-candidate、coding execution 与 repository routing capability。
- §7 将 C1 定义为 4–7 人日、验收映射 A07/A09/A12/A13；§8/§9 明确不引入 owner/fence、自动 takeover、统一 generation 服务或第二 durable operation 状态机。

## Goals / Non-Goals

**Goals:**

- 以一个载体无关 target union 和带 expected version 的 durable binding contract 贯穿 enrollment、plan/session、advance、compile child 及通知，支持用户明确重绑/换代并让旧代只读。
- 将候选门/降级门和租约冲突转成可观察的等待项：完整快照先落盘，恢复/重建、等待或确认接管由用户触发，成功后回到原有 plan 编排。
- 定义 Failed advance 的独立 retry-initialization 语义：原失败审计不变、同 attempt 续做、未知外部副作用不盲重。
- 把 `existing`/`create` 意图作为计划合同和 compile 校验的可观察边界；缺失、错拼或越权时可反馈/修订而不扩大 scope。
- 让每个动作在驾驶舱/inbox 具有稳定 command key、对象身份、版本校验、幂等结果和可理解的下一步。

**Non-Goals:**

- 不后台扫描旧 issue、按最新 plan/session 猜接管、自动迁移/自动 takeover、清空 binding 或旧 child。
- 不建设独立 durable generation、owner/fence、全局 recovery orchestrator、通知数据库或第二套 operation 状态机；不把 generation 当作跨服务一致性平台。
- 不改变人工 plan gate 的 approve/feedback/abandon 边界，不自动代批准；不实现 coding 侧 provider 断连、runner 恢复、验证分诊、LC 冷启动、C5 role-chain preflight 或 pi recipe。

## Decisions

### 1. 单一显式 target 与版本化 binding

扩展既有 enrollment/绑定合同而非另建状态库。target 使用显式 union：单仓携真实 repository 身份，逻辑代码库携 codebase 与 logical repository 身份；绑定记录同时保存 plan/session、source、target、provider、binding version（generation 仅表示一次用户明确换代）。每个读取/写入/继续动作先核对 enrollment、对象身份和 expected version。用户重绑/换代提交后追加新版本，旧记录不删不改；同一 command_id+同负载返回原结果，异负载或过期版本 fail-closed。

**选择理由：** 复用现有 CAS 与精确 session/plan 身份，能拒绝迟到旧回执且不猜接管；避免 v1.1 owner/fence 或全局 generation 复杂度。代价是用户必须明确选择，错误不会被后台“修好”，这是 C1 的人到点处理口径。

### 2. 候选/恢复门先持久化、后通知

候选门故障恢复沿既有 gate/session/journal 事实写面：candidate 全文、source revision、budget、gate/诊断身份先原子落盘，观察 relay/WS 仅是投影。快照不完整或不可读时只显示恢复运行/从权威候选重建，不允许 approve。用户操作带 gate/version/绑定身份和 command_id；恢复成功后调用既有 typed feedback/approve/abandon，不创建平行候选或新门协议。

**选择理由：** 消除 #1/#6 的“裸 approve”和 observer 依赖，保持 `work-item-plan-conversational-gate` 的关门边界。若权威候选不可达，正确结果是停等而非摘要/猜测。

### 3. 租约/接管判别与 advance retry

restart、recover、new advance 都重读现有 worktree lock/attempt claim。判别为活跃、已死亡、未知三态：活跃只通知等待；死亡只在用户确认 takeover 后原子继续；未知停等确认/重绑。Failed advance 另设 retry-initialization 动作，校验 plan/revision/target/attempt/checkpoint；保留原失败并追加 retry 事实，仅续做安全本地步骤，未知外部副作用转用户确认。普通 advance 重发不改变 Failed/Aborted 语义，不生成第二 attempt。

**选择理由：** 满足 §5.1 与 A09 的“同 attempt 到 Ready”而不引入 lease epoch/fence；将 retry 与原命令分开，避免以重放掩盖副作用。

### 4. 计划意图与 child 绑定采用 fail-closed 精确匹配

plan contract 必须表达每项 `existing`/`create`、provider Work Item 和 scope 约束。existing 只解析授权已有目标；create 必须显式声明且满足依赖/范围。compile child 选择同时核对 plan、plan revision、work item revision、当前 binding；换代后显式创建新 child，旧 child/binding 只读。

**选择理由：** 直接修正 #9/#14 的根因，防止用基线存在性代替创建授权，也防止按 entity/最新 session 复用错误 child。旧事实保留保证可追溯，代价是换代会留下历史 child。

### 5. 同一应用服务与通知投影

REST 与页面动作必须调用同一既有应用服务；本 change 的操作结果进入既有 journal/DTO/驾驶舱 inbox/系统通知投影。通知必须说明原因、已完成步骤、目标与 plan/session/attempt/gate 身份、副作用、按钮会/不会做什么及成功后下一阶段。通知投递失败不回滚业务事实；驾驶舱补读 durable 状态。每项动作均有 stable command_id 与 expected object/version，结果可幂等重读。

**选择理由：** 保持页面断开不取消业务事实，避免 REST/页面各自实现导致语义分叉；仅复用既有投影，不建设通知库。

## Failure Handling and Migration

- **快照/持久化失败：** fail-closed，保留可诊断错误；不得显示 approve 或扣预算。修复依赖用户点击恢复/权威候选重建。
- **版本/身份冲突：** 返回需刷新/重新绑定；不覆盖当前版本、不触发 provider、不改旧代审计。
- **租约活跃/未知：** 分别等待或确认；未知永不自动抢占。死亡接管后的迟到旧写入必须按旧身份拒绝。
- **retry 副作用不明：** 停等“确认后重试／重新绑定”，确认前不重跑；原 Failed 记录永久可查。
- **旧 durable 数据：** 缺少新 binding/generation/target/retry 字段时按既有 off/Manual/旧手动行为解释，不批量迁移、不猜测新绑定；旧 child 与记录只读。
- **兼容边界：** 本 change 只改 C1 直接冲突的 requirement；coding resilience 和单仓/LC 入口由 C2/C5 分别消费 target/binding 合同，不在此重复实现。

## Contract Traceability

| C1 依据 | 本设计落点 |
|---|---|
| v1.2 §4.2 #1/#6 | 候选全量快照、恢复/重建按钮、无快照禁止 approve、observer 仅投影 |
| v1.2 §4.2 #9 | existing/create/provider Work Item 合同与反馈/修订停等 |
| v1.2 §4.3 #11 | restart/recover/new advance 重取 lease，活跃/死亡/未知三态 |
| v1.2 §4.3 #12 | 用户明确 plan/session/source/target/provider 重绑/换代，旧代只读、版本校验 |
| v1.2 §4.3 #14 | child 按 plan/revision/work item/binding 精确筛选，新旧 child 分代保留 |
| v1.2 §4.3 #16 | 独立 retry-initialization、Failed 审计保留、同 attempt 续做 |
| v1.2 §5.1/§5.6 | 复用 CAS/journal/gate/投影，修改六项 capability delta，不扩 C2/C4/C5 范围 |
| v1.2 §7 A07/A09/A12/A13 | 以通知→按钮→结果→自动续进作为每项可观察验收链 |

## Risks / Trade-offs

- **显式选择增加交互成本：** 但可证明地避免后台误接管、旧代污染和未知外部副作用；通过通知展示完整身份和下一步降低成本。
- **旧绑定只读会累积历史：** 换取审计和迟到回执拒绝能力；读取面必须明确当前版本与历史版本，不能再用“最新记录”隐式选择。
- **观察面失效时用户依赖补读：** 事实先持久化、通知失败可补读，确保业务不依赖 WS；A07 需验证无 consumer 的恢复路径。
- **多 capability delta 需要严格共用合同：** target/binding DTO 若被 C5/C2 各自扩展会产生漂移；实现阶段只能以 C1 的 union/version 合同为单一写面，并由后续 change 消费。
- **外部副作用未知无法自动化：** 这是安全边界而非缺陷；界面必须将“等待/确认后重试/重新绑定”与安全本地 retry 清楚区分。
