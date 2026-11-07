## MODIFIED Requirements

### Requirement: durable 策略与可撤销的未消费执行许可（REQ-WIGA-02）
单一 issue 的 enrollment SHALL 是自动化授权唯一来源。

#### Scenario: 策略版本线性化

- **WHEN** 并发更新 `policy_revision`
- **THEN** `policy_revision` 的并发更新 SHALL 线性化。

#### Scenario: 批准策略冻结

- **WHEN** enrollment 冻结 `plan_approval` 并物化新自动化授权
- **THEN** enrollment SHALL 冻结 `plan_approval: manual|auto`（缺省 manual，旧记录无字段按 manual 读；新自动化授权一律取 auto，并把 `require_execution_plan_confirm` 压为 false——仅在该 auto 授权物化的 WI 内归一化，MUST NOT 修改手动旧 WI 字段）。

#### Scenario: 自动策略绑定校验

- **WHEN** 设置或校验 plan session 的 `AutoIfValid` 策略
- **THEN** 绑定的 plan session `run_policy` SHALL 可为 `AutoIfValid` 仅当当前 enabled enrollment 精确绑定该 plan/session/source/target 且其冻结策略为 auto；创建、binding/rebind、生成准入、确认投影各接缝 SHALL 执行同一校验，仅客户端传 AutoIfValid MUST NOT 获得自动执行权，错绑或不一致 SHALL 拒绝。

#### Scenario: 自动批准范围与停点

- **WHEN** `AutoIfValid` 自动批准并准备 compile
- **THEN** AutoIfValid SHALL 只省略正常人工批准（outline、draft、batch 及最终批准自动通过），MUST NOT 跳过 AI choice、人工门（HumanRequired）、机械校验或 trust 准备：自动 compile 前 SHALL 复用既有 trust 准备，非 Ready 时落 typed waiting 停点并通知（REQ-WIGA-12），零 compile 副作用；trust Ready 后自动 Confirmed。

#### Scenario: 启动策略物化

- **WHEN** coding attempt 物化启动策略
- **THEN** coding attempt SHALL 物化 `CodingStartRunPolicy = Manual | AutoStartOnce{enrollment_id, policy_revision, source_plan_revision}`，缺省为 Manual。

#### Scenario: 拆分冻结策略写定

- **WHEN** split 复制 start policy 并写定 advance record
- **THEN** split SHALL 把冻结 start policy clone 到每仓桶，advance record SHALL 连同 `start_run_policy` 一次写定且 update/replay MUST NOT 改（旧 record 默认 Manual、MUST NOT 升格；existing journal 策略不符即停，不给旧 Manual 原地补授权）。

#### Scenario: 自动许可限目标使用

- **WHEN** 消费自动启动许可前
- **THEN** 自动许可只准该绑定 target/attempt 使用且须在消费前再核对当前授权。

#### Scenario: 关闭撤销未消费许可

- **WHEN** 关闭自动化时存在尚未认领或已运行的动作
- **THEN** 关闭自动化 SHALL 拦住线性化关闭后尚未认领的动作并撤销未消费的启动许可；已被认领/运行的动作不因关闭而自动 Abort。

#### Scenario: 重开不清已消费记录

- **WHEN** 重新开启自动化
- **THEN** 重新开启 SHALL NOT 清除已消费的单发记录或隐式重试 Failed/Aborted attempt。

#### Scenario: 终止后禁止启用

- **WHEN** enrollment 已终止后请求 Enable、rebind 或继续
- **THEN** 已终止 enrollment MUST NOT Enable/rebind/继续（REQ-WIGA-09）。

#### Scenario: Interactive 保留唯一人工门

- **WHEN** 自动化绑定 plan 已经通过机械评审进入 AI 主动人工门（HumanRequired）
- **THEN** 系统停等并通知，不自动 approve/feedback/abandon；仅人处理后（或正常机械路径 Valid）才可能经确定性 compile 达 durable Confirmed

#### Scenario: auto 冻结与旧缺省 manual

- **WHEN** 新自动化授权提交 approval=auto，或读取无该字段的旧 enrollment JSON
- **THEN** 新授权冻结 auto 并幂等比较选项；旧记录按 manual 读，不静默改写已绑定旧 Interactive session

#### Scenario: 正常批准自动通过、AI 主动停点仍停等

- **WHEN** 自动化绑定 plan 正常生成（无 AI 提问、无人工门）
- **THEN** outline、draft、batch 及最终批准全部自动通过并推进到 Confirmed；出现 AskUserQuestion/choice 或 HumanRequired 时停等并通知，不自动回答或代批准

#### Scenario: 错绑 AutoIfValid 拒绝

- **WHEN** 请求在无当前 enabled enrollment 精确绑定（或冻结策略为 manual）的 session/plan 上携带 AutoIfValid
- **THEN** 创建、绑定、准入与确认投影均拒绝，不进入自动执行

#### Scenario: trust 未齐不 compile

- **WHEN** 自动 compile 前 trust 准备未达 Ready
- **THEN** 系统落 typed ConfirmationTrustWaiting 停点（保留原 reason 与 session 目标）并通知，不产生 compile 副作用；trust 转 Ready 后自动 Confirmed

#### Scenario: split 冻结策略与旧 Manual 不升格

- **WHEN** Enrolled 集合授权 split 出多个仓桶，或重放旧 Manual advance record
- **THEN** 每仓桶冻结同一 Enrolled 策略且 replay 保持 AutoStartOnce；旧 record 默认 Manual 不升格，journal 策略不符即停

#### Scenario: 关闭发生在启动认领前

- **WHEN** 用户关闭 enrollment 并在线性化点后才有编排动作或自动 StartCoding 请求尝试认领
- **THEN** 该动作被拒且无 provider 启动；虽已物化但未消费的 AutoStartOnce 许可不再生效

#### Scenario: 关闭发生在启动认领后

- **WHEN** 自动启动已原子认领并开始运行，用户随后关闭 enrollment 或再开启
- **THEN** 已运行 provider 不被开关隐式中止；重新开启不再次消费该 attempt 的启动许可，不自动重启 Failed/Aborted attempt

#### Scenario: 终止后自动重开被拒、人工重新开启受控

- **WHEN** enrollment 已写 terminated_at 后提交 Enable/rebind/继续
- **THEN** rebind 与继续一律拒绝且零副作用；Enable 仅当由人工入口携带稳定 command_id、`source` 与 `target` 与终止记录逐字段相等、且收口已完成时，以同 id 新 revision 取代并清空 terminated_at（REQ-WIGA-09）；其中 `options`（含 provider）允许与终止记录不同 —— 该放行**仅限非悬空源场景**（终止记录的原 source 已悬空时，悬空源重授权路径的 `options`/`target` 仍须与终止记录逐字段相等，MUST 拒绝漂移，见 REQ-DRG-18）；非悬空源场景下该差异按换代路径落盘（REQ-WIGA-SWAP-01）；任何自动路径不得重新开启，不降级为手动轮

### Requirement: 整 plan 终止与统一 Abort 语义（REQ-WIGA-09）

系统 SHALL 提供 `POST …/automation-enrollment/terminate`（`{command_id, expected_enrollment_id, expected_policy_revision, expected_binding?, confirmed:true}`）；门 Abort/原 abort REST/gate-responses REST/WS/收件箱 SHALL 统一进入同一整 plan 终止与二次确认（未确认 `automation_plan_termination_required` 零副作用），auto 单仓同覆盖。plan 与 attempt 终止不可恢复。terminated SHALL 阻断全部自动路径，且 SHALL 只可由人工显式 Enable 以同 `enrollment_id` 新 `policy_revision` 取代重开（清 `terminated_at`、解绑旧 plan/session、账本留痕）；被终止 plan 与 attempt 不复活。字段与错误码见下各场景。

#### Scenario: 未确认零副作用返回确认要求

- **WHEN** 用户从门 Abort/原 abort REST/gate-responses REST/WS/收件箱任一入口对自动化绑定 plan 发起中止，未完成统一二次确认
- **THEN** 返回 `automation_plan_termination_required`（含 plan 与绑定身份），不写单 attempt Aborted、不写 gate-response 成功账本、不启动终止

#### Scenario: 确认后整 plan 全部停止

- **WHEN** 用户确认二次确认并提交 terminate（含并行 WI 中从任意工作台发起）
- **THEN** 全部 runner 结束、无后续 provider、未完成成员 Aborted、Completed 保留、文件留存；响应含逐 attempt 结果
- **AND** 执行顺序为：enrollment 锁内写 terminated_at＋enabled=false→禁新 run/失效 CodingPlanRun→取消 plan run 等真实 completion（已 Confirmed 不改）→取消全部成员 runner（内部 AbortAttempt 仅干净退出并完成 completion，不走旧 handle_abort/clean、不递归发终止）→逐 mutation lease 专用收口关全部 open blocked/choice/stage 门（不伪造答案、不续跑、跳过 clean 保留文件、单仓异常继续后仓）→清三类旧通知事实（清理失败写 stage=termination 的 automation_failed）；group 锁不跨等待

#### Scenario: 脏工作树不阻断终止

- **WHEN** 终止时某仓工作树有未提交改动（旧 abort 会因 dirty 拒绝）
- **THEN** 内部 AbortAttempt 干净退出，专用收口置 Aborted 并保留文件；终止仍成功，不以"不能停止"回绝

#### Scenario: stage 门等待中终止

- **WHEN** 终止时某 attempt 正等待 stage 门
- **THEN** 该门等待的 AbortAttempt 干净退出，专用收口关闭该门，不倒计时续跑、不启 provider

#### Scenario: 全部 open 门关闭与事实清空

- **WHEN** 终止收口执行完成（含 blocked/choice/stage 各类 open 门与三类通知事实，也含尚无 attempt 的 plan）
- **THEN** 全部 open 门关闭、旧事实清空；刷新后无幽灵门通知，收件箱不再出现该 plan 待办

#### Scenario: 清理失败可见且重清后消失

- **WHEN** 某仓收口异常
- **THEN** 整体返回 cleanup_required 并写 stage=termination 的 automation_failed（可点回该 plan）；重提同命令重做收口成功后该事实消失；attempt 终态仍不可恢复（enrollment 的人工重新开启见「terminated 记录经人工 Enable 重新开启」）

#### Scenario: terminated 记录阻断一切自动路径

- **WHEN** enrollment 已 terminated 后 reconcile/自动生成/自动首启/续驱/重启补偿扫描运行，或绑定实体删除流程再次运行
- **THEN** 一律零副作用：不写 enrollment、不建 plan/session、不启动 provider、不把已 Aborted attempt 改回非终态、不复活被终止 plan；MUST NOT 代发 Enable（重新开启唯一入口为人工 `PUT`）
- **AND** 不降级 Manual；管理投影保持 terminated（owner=server、`terminated_at` 非空）

#### Scenario: terminated 记录经人工 Enable 重新开启（取代并留痕）

- **WHEN** enrollment 已 terminated（`terminated_at` 非空、`policy_revision=n`），用户在人工入口提交 Enable（携带稳定 `command_id`、`expected_revision=n`，`source`/`target` 与终止记录逐字段相等；`options`（含 provider）允许与终止记录不同 —— 该放行**仅限非悬空源场景**（终止记录的原 source 已悬空时，悬空源重授权路径的 `options`/`target` 仍须与终止记录逐字段相等，MUST 拒绝漂移，见 REQ-DRG-18）；非悬空源场景下该差异按换代路径落盘，见 REQ-WIGA-SWAP-01）
- **THEN** 系统以同 `enrollment_id`、`policy_revision=n+1` 取代落盘：`terminated_at` 清空、`enabled=true`、`plan_id`/`session_id` 为空、`binding_history` 新代（旧代进 `previous`、`binding_version` 递增、plan/session 空串占位）、命令账本追加该 `command_id` 条目（`state=Accepted`）；响应 200 返回 enabled 记录
- **AND** 随后既有 prepare 路径建立全新 plan/session 并绑定；被终止 plan 与既有 attempt 不复活、不复用，单次请求不重复生成
- **AND** 同 `enrollment_id` 取代**必须轮换 `prepare_intent_id`**（`plan_id`/`session_id` 由其派生；沿用会让 prepare 幂等复用被终止的旧 plan 实体）

#### Scenario: 重新开启的命令身份幂等与异负载拒绝

- **WHEN** 以相同 `command_id`＋相同 payload 重放重新开启，或以相同 `command_id` 配不同 payload/`expected_revision`，或 terminated 重开请求缺 `command_id`
- **THEN** 同 payload 返回首次 durable 结果（`Replayed`，不再递增 `policy_revision`、不新增账本条目）；异 payload 返回 409 零副作用；缺 `command_id` 返回 422 零副作用

#### Scenario: 重新开启不改写授权并保留墓碑语义

- **WHEN** 对 terminated 记录提交 Enable 时 `target` 与终止记录不等、或 `source` 与终止记录不等（**source 漂移的拒绝条件仅适用非悬空源场景** —— 终止记录的原 source 已悬空时 source 漂移放行，`options`/`target` 仍须与终止记录逐字段相等，见 REQ-DRG-18）、或提交 Disable、或 `expected_revision` 不等于当前 `policy_revision`
- **THEN** 返回 409 Conflict 零副作用；**非悬空源场景下**，`options`（含 provider）与终止记录不等**不属于**本场景的拒绝条件 —— 该差异按 REQ-WIGA-SWAP-01 的换代语义以换代取代落盘；**终止记录的原 source 已悬空时不在本豁免内** —— 悬空源重授权路径的 `options`/`target` 漂移 MUST 拒绝（REQ-DRG-18），而 `source` 漂移放行（不属本场景拒绝条件）；**已 disabled（非 terminated）记录**换授权 payload 仍须先成功重新开启、再走既有 Disable→Enable 路径（见「非 terminated 记录的并发语义不变」）

#### Scenario: 存在未终结 attempt 时不可重新开启

- **WHEN** terminated 记录仍持有**未终结的 attempt 或未收口的运行事实**（含 `stage=termination` 未收口），用户提交重新开启
- **THEN** 返回 409 `automation_plan_cleanup_required` 且零副作用；以原 `command_id` 重提终止完成收口、该事实消失后方可重新开启
- **AND** 绑定实体**已被删除**（墓碑）且**无未终结 attempt** 时视为无可收口之物，**允许**人工重新开启（不因"清理任务失败"本身而永久阻断）

#### Scenario: 非 terminated 记录的并发语义不变

- **WHEN** 未 terminated 的 enrollment 提交 Enable：enabled 状态下 `source`/`options`/`target` 与当前绑定不等或 `expected_revision` 过期（失效），或已 disabled 记录换 payload 重开
- **THEN** 前者返回 409 Conflict 零副作用；disabled 记录的 Disable→Enable 换 payload 重开与 `policy_revision` 线性化行为与本 change 前一致

#### Scenario: 迟到写入不覆盖终态

- **WHEN** plan session 取消后迟到的 Failed/Open 完成写入尝试
- **THEN** Terminated 终态不被覆盖；已 Confirmed 的 attempt 状态不改

#### Scenario: 手动单 attempt abort 不变

- **WHEN** 无自动化绑定的 attempt 走原 abort
- **THEN** 单 attempt abort 语义与批 2 前一致（含 clean 检查），不进入 plan 终止

#### Scenario: 同命令重放幂等

- **WHEN** 以相同 command_id 重复提交同一 terminate
- **THEN** 重做/返回收口结果，不重复取消、不重复关门、不启动 provider

## ADDED Requirements

### Requirement: 换 provider 一次完成且不做接续（REQ-WIGA-SWAP-01）

对**已终止且无法继续**的自动化授权，owner SHALL 能通过**一次操作**完成「删除旧工作项组 → 以新 provider 从零重建」，系统 MUST NOT 要求 owner 先用旧 provider 重开一次。

该操作 SHALL 保持既有换代留痕（授权身份轮换、绑定换代、旧代进历史、命令账本留痕、旧工作项组全闭包物理删除），且 SHALL NOT 残留旧 provider 的绑定身份、生成意图或半成品产物。

本 requirement 只约束 **terminated** 路径。**运行中**（enabled）的授权变更 SHALL 继续走 REQ-WIGA-02 的 fail-closed，由 owner 显式终止后重开。

#### Scenario: terminated 记录上直接以新 provider 重建被接受

- **WHEN** owner 对一个已终止的授权提交带有不同 provider 的启用命令
- **THEN** 系统 SHALL 接受该命令并完成换代（授权身份轮换、绑定换代、旧代进历史、命令账本留痕、旧工作项组全闭包物理删除），SHALL NOT 因「`options`（含 provider）与终止记录不同」而拒绝；`source`/`target` 的逐字段相等要求不变（见 REQ-WIGA-02 / REQ-WIGA-09 的相应场景）

#### Scenario: 与既有 terminated 重开条款的分工

- **WHEN** owner 对一个已终止的授权提交 Enable（可能携带与终止记录不同的 `options`）
- **THEN** 逐字段相等要求与放行条件（人工入口、稳定 `command_id`、`expected_revision`、收口已完成、同 `enrollment_id` 新 `policy_revision` 取代、`prepare_intent_id` 轮换、旧 plan 不复活）SHALL 仍以 REQ-WIGA-02 / REQ-WIGA-09 的相应场景为准；只有 `options`（含 provider）的差异按本 requirement 的换代语义处理

#### Scenario: 换 provider 后旧 provider 不再产出

- **WHEN** owner 完成一次换 provider
- **THEN** 系统 MUST NOT 以旧 provider 启动任何生成；SHALL NOT 产生归属旧 provider 的半成品产物

#### Scenario: 换 provider 后不残留旧链身份

- **WHEN** owner 完成一次换 provider
- **THEN** 旧的生成意图文件 SHALL 已被删除、旧 plan/session 身份 SHALL 已解绑、新的生成意图 SHALL 由新的授权身份派生；SHALL NOT 复用旧链的身份派生产物

#### Scenario: 换 provider 失败时旧工作项组已删除且失败面如实告知

- **WHEN** 换 provider 的操作在「已删除旧工作项组」之后失败
- **THEN** 系统 SHALL 停在「旧的已删除、新的未建立」的状态；错误面 SHALL 明确告知该事实并给出重试指引，MUST NOT 表述为「未发生任何变化」

### Requirement: 授权变更失败面按语义分诊（REQ-WIGA-SWAP-02）

授权写命令的失败 SHALL 以**区别于运行态限制**的稳定错误码呈现，并给出可执行的下一步；机器码与原始 detail SHALL 保留在次要行或折叠区，MUST NOT 作为主文案。

#### Scenario: 运行中不许改授权内容

- **WHEN** 自动化处于运行态，owner 提交与当前授权内容不同的变更
- **THEN** 系统 SHALL 拒绝该提交并返回专属于「运行态限制」的稳定错误码；错误面 SHALL 以中文说明「运行中不能直接改」并 SHALL 指明 owner 可执行的下一步（在既有的「终止整个自动化」确认面终止后回到配置入口重新启用），且该指引 SHALL NOT 声称终止入口在所有落屏都可见

#### Scenario: 版本号过期

- **WHEN** owner 提交授权写命令时携带的预期版本与当前版本不一致
- **THEN** 系统 SHALL 返回专属于「版本过期」的稳定错误码；错误面 SHALL 以中文说明「授权记录刚发生变化，本次操作没有生效」并 SHALL 指明「等当前生成停下来再点一次（会自动重试）」

#### Scenario: 两类失败不得共用同一稳定码

- **WHEN** 授权写命令因运行态限制而失败，或因版本号不一致而失败
- **THEN** 两者 SHALL 返回不同的稳定错误码与不同的中文主文案；错误面 SHALL 能让 owner 分辨该「等一下重试」还是「该先终止」

#### Scenario: 终止自愈路径保持原有稳定码

- **WHEN** 终止确认面遇到授权版本冲突并触发既有自愈流程
- **THEN** 该流程 SHALL 继续使用其原有稳定码与原有行为，MUST NOT 因本次错误码细分而失效
