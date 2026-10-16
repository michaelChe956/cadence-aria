# Spec Delta

> 基线：`openspec/specs/work-item-group-autopilot/spec.md`（REQ-WIGA-01~08、REQ-WIGA-C5-PREFLIGHT、REQ-WIGA-C5-CHAIN）。本 delta 修订 REQ-WIGA-01/02/03/06/08/C5-CHAIN 六项（owner 2026-10-16 05:00 双审裁决与 phase-ledger 批 2 裁定），新增 REQ-WIGA-09~13 五项；REQ-WIGA-04/05/07、REQ-WIGA-C5-PREFLIGHT 不变。规格修订登记依据总览 §6 规格修订完整登记。

## MODIFIED Requirements

### Requirement: 人工 design 确认后的显式授权与范围（REQ-WIGA-01）

系统 SHALL 保持 story/design 的生成和确认由人完成，并仅在 design 经人工确认后提供自动化模式选择；默认 SHALL 为不自动化。自动化 enrollment SHALL 将 target、plan、session、source、provider 和绑定版本作为同一条精确身份合同持久化；target SHALL 是 enrollment 载体身份的唯一权威，并 SHALL 区分三类：单仓（携带与 issue 记录所属物理仓一致的真实 repository 身份，MUST NOT 携带或生成任何 logical repository 替身身份）、逻辑代码库单成员、逻辑代码库成员集合（`logical_codebase_members`：logical codebase 身份＋logical repository 成员集合）。集合 target SHALL 经统一反序列化规范化入口处理原始 REST JSON 与磁盘读取：非 UUID 或空集 SHALL 拒绝，乱序及重复输入 SHALL 规范化为升序唯一集合，单元素 SHALL 折叠为既有单成员 kind，旧合法 JSON/history 不迁移；digest、selection_key 与 bound.target 比较一律在规范化之后。自动化授权 SHALL 覆盖 issue explicit selection 的全部有效成员（manifest∩(included−excluded)，排序去重），MUST NOT 要求逐仓勾选，MUST NOT 提供逐仓授权/启动 API；集合用于 enrollment 及其 binding/generation intent/child 授权身份，单 WI intent 仍单仓、MUST NOT 把任一集合成员当作全部 WI 归属。plan 实际桶集合 P SHALL 满足 ∅≠P⊆A（A＝授权成员集合）且无未归属 WI；attempt 只按 P 创建，A−P 的授权仓 MUST NOT 造空 attempt/空组。用户执行"重新绑定／换代"时 MUST 明确提供目标 plan/session、source、target 和 provider，并携带当前 expected binding/policy version；新旧 logical 走同一路径、规范化后全等，rebind MUST NOT 借道扩权，current/previous 保留全集，旧单成员 MUST NOT 随 selection 扩仓。成功操作 SHALL 写入新的绑定版本并使后续动作只接受当前版本；旧绑定、旧 child 与旧代事件 SHALL 保持只读可追溯。系统 MUST NOT 按最新 session、旧 issue 扫描或后台失败发现猜测接管，也 MUST NOT 将 Manual 已认领 attempt 升级为自动。未选自动化、旧数据缺少 enrollment 或单成员身份不明确时 SHALL 保持既有手动流程；零 target、空集、非法 UUID、错 LC、跨载体或身份不一致 SHALL fail-closed 拒绝自动授权。

#### Scenario: design 确认前不出现自动授权

- **WHEN** story 或 design 尚未由人确认
- **THEN** 不创建 enrollment、不自动准备 plan，原有人工生成与确认入口继续可用

#### Scenario: 默认不自动化

- **WHEN** 用户在 design 确认后选择不自动化或旧 issue 不含 enrollment
- **THEN** plan 准备、生成、advance 和 coding 启动仍按原有手动行为执行，不因自动化补偿扫描产生动作

#### Scenario: 集合授权覆盖全部有效成员

- **WHEN** 已确认 design 的 issue explicit selection 覆盖同一 logical codebase 的四个成员仓，用户选择自动化
- **THEN** 系统创建一个 enrollment，其集合 target 规范化覆盖全部四个成员；入口不出现逐仓勾选，回传原样集合

#### Scenario: 实际桶 P⊆A 且无空组

- **WHEN** 授权集合 A 含四仓而 plan 只把 WI 归属到其中三仓
- **THEN** 系统只为三个实际桶创建 attempt，第四个授权仓不产生空 attempt/空组；存在未归属 WI 时拒绝并要求修正

#### Scenario: 乱序重复输入规范化后比较

- **WHEN** 客户端或磁盘 JSON 提交乱序、含重复 UUID 的成员集合
- **THEN** 统一规范化入口输出升序唯一集合后 digest/selection_key/bound.target 相等判定成立；空集或非法 UUID 被拒绝

#### Scenario: 单元素折叠与旧 wire 兼容

- **WHEN** 集合 target 只含一个成员
- **THEN** 系统折叠为既有单成员 kind 写出；旧单成员 JSON/history 原样可读，不迁移旧 digest

#### Scenario: 错仓或跨载体 target 被拒

- **WHEN** Enable 携带的单仓 repository 身份与 issue 记录所属物理仓不一致、集合含不属于该 logical codebase 的成员，或对单仓 issue 提交逻辑代码库 target、跨载体混用
- **THEN** 系统返回 422 与需重新选择目标的提示，不写 enrollment、不启动 provider

#### Scenario: 显式重绑创建新绑定版本

- **WHEN** 当前 enrollment 处于等待恢复且用户提交匹配 expected version 的 plan、session、source、target、provider
- **THEN** 系统写入新的绑定版本，返回当前 enrollment 身份与下一步操作；旧绑定保持只读，后续编排只消费新版本

#### Scenario: 单 target 精确绑定及重复提交

- **WHEN** 已确认 source 只对应一个精确 target（一个单仓物理仓，或逻辑代码库选择中的一个 logical repository），用户用相同选择键重复开启自动化
- **THEN** 系统仅返回同一 enrollment、同一绑定 plan/session 与策略修订；不同 source 或 provider/options 的冲突提交明确拒绝且不改绑

#### Scenario: 单仓 target 可被授权

- **WHEN** 单仓 issue 的记录指向一个已登记的物理仓，用户在 design 确认后选择自动化并提交服务端投影的单仓 target
- **THEN** 系统创建 enrollment，其 target 为该物理仓身份、不含任何 logical repository 身份，绑定版本 1 建立且后续自动链可以启动

#### Scenario: 多 target 不被自动纳入

- **WHEN** 无成员集合授权而已确认 source 或冻结的 plan/attempt 涉及多个目标仓库、目标不明确或与 enrollment target 不符
- **THEN** 系统拒绝自动授权或自动启动，绝不循环启动多个 attempt；既有逐 target 手工准入仍可用

#### Scenario: 绑定版本过期拒绝

- **WHEN** 用户以过期 expected binding/policy version 或不匹配的 plan/session/target 提交重绑
- **THEN** 系统返回版本冲突/需刷新错误，不覆盖当前绑定、不启动 provider、不改变旧代历史

#### Scenario: 同一命令重放幂等

- **WHEN** 客户端以相同 command_id 和相同绑定负载重复提交同一重绑/换代操作
- **THEN** 系统返回首次操作结果，不创建第二绑定版本、不重复触发后续编排；同一命令异负载被拒绝

### Requirement: durable 策略与可撤销的未消费执行许可（REQ-WIGA-02）

单一 issue 的 enrollment SHALL 是自动化授权唯一来源；`policy_revision` 的并发更新 SHALL 线性化。enrollment SHALL 冻结 `plan_approval: manual|auto`（缺省 manual，旧记录无字段按 manual 读；新自动化授权一律取 auto，并把 `require_execution_plan_confirm` 压为 false——仅在该 auto 授权物化的 WI 内归一化，MUST NOT 修改手动旧 WI 字段）。绑定的 plan session `run_policy` SHALL 可为 `AutoIfValid` 仅当当前 enabled enrollment 精确绑定该 plan/session/source/target 且其冻结策略为 auto；创建、binding/rebind、生成准入、确认投影各接缝 SHALL 执行同一校验，仅客户端传 AutoIfValid MUST NOT 获得自动执行权，错绑或不一致 SHALL 拒绝。AutoIfValid SHALL 只省略正常人工批准（outline、draft、batch 及最终批准自动通过），MUST NOT 跳过 AI choice、人工门（HumanRequired）、机械校验或 trust 准备：自动 compile 前 SHALL 复用既有 trust 准备，非 Ready 时落 typed waiting 停点并通知（REQ-WIGA-12），零 compile 副作用；trust Ready 后自动 Confirmed。coding attempt SHALL 物化 `CodingStartRunPolicy = Manual | AutoStartOnce{enrollment_id, policy_revision, source_plan_revision}`，缺省为 Manual；split SHALL 把冻结 start policy clone 到每仓桶，advance record SHALL 连同 `start_run_policy` 一次写定且 update/replay MUST NOT 改（旧 record 默认 Manual、MUST NOT 升格；existing journal 策略不符即停，不给旧 Manual 原地补授权）。自动许可只准该绑定 target/attempt 使用且须在消费前再核对当前授权。关闭自动化 SHALL 拦住线性化关闭后尚未认领的动作并撤销未消费的启动许可；已被认领/运行的动作不因关闭而自动 Abort。重新开启 SHALL NOT 清除已消费的单发记录或隐式重试 Failed/Aborted attempt；已终止 enrollment MUST NOT Enable/rebind/继续（REQ-WIGA-09）。

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

#### Scenario: 终止后不可重开

- **WHEN** enrollment 已写 terminated_at 后提交 Enable/rebind/继续
- **THEN** 系统拒绝且零副作用（REQ-WIGA-09），不降级为手动轮

### Requirement: 无页面的确定性后端推进与人工停点（REQ-WIGA-03）

对仍有效且精确绑定的 enrollment（单仓、单成员或成员集合授权），服务端 SHALL 从 durable 状态重建下一动作：至多幂等准备一次 plan/session、至多启动一次对应生成、正常批准自动通过（REQ-WIGA-02）、仅在 AI choice 与人工门停等并通知、等待既有 compile/publication 成功、以独立 advance 把全部实际桶推进到 Ready，再经共用 StartCoding 命令启动各桶 attempt。LC 自动编排 SHALL 经 plan_start 提取的 crate 内共享服务推进（prepare→Enrolled advance→bind_ready→mark_start_requested→Enrolled start→真实 outcome）；首启与续驱 SHALL 沿 `CodingPlanSelection::deterministic_start_command_id` 稳定命令身份；首项真实 outcome 落盘后 SHALL 立即以既有全图就绪桶驱动启动其余桶，后续 tick 与完成挂点复用同一驱动与 claim。全部自动桶（尤其非首桶）SHALL 以 Enrolled origin 启动并逐桶核对 policy/enrollment/binding；Created 分支按 attempt 冻结 policy 选 origin（AutoStartOnce→当前 binding 的 Enrolled，失效即停，MUST NOT 降级 Manual）；Running parked 续跑 SHALL 在物化/拉 runner 前复核同一自动授权；撤销、篡改或绑定不符 SHALL 零 provider。服务端 SHALL 在每次 restart、recover 或 new advance 前重新读取当前绑定与租约/attempt claim；编排器每 tick SHALL 先读当前 binding 未解除的自动化失败事实（advance/coding_start），存在时 SHALL 停在该点，MUST NOT 自动 retry。编排事件仅作为唤醒；无页面或没有任何 WS 连接时也 SHALL 继续安全动作及恢复。门关闭、approve 点击、compile 节点结束或 advance_completed 均不得被误判为 Confirmed、coding 完成或 provider 启动。租约冲突时，活跃持有者 SHALL 使系统通知用户等待；已死亡持有者仅 SHALL 展示"确认接管"操作，用户确认后才可继续；活性未知 SHALL 保持等待，不得自动抢占。所有恢复动作 SHALL 通过既有 advance/runner/初始化链继续，不能以通知投递成功冒充业务成功。

#### Scenario: 从 design 确认自动生成且停在人工门

- **WHEN** design 经人确认后选择自动化，用户关闭工作区与 driver 连接
- **THEN** 服务端准备唯一 plan/session、启动一次生成；正常批准自动通过，到 AI choice/人工门时停等驾驶舱操作，不自动回答或代批准

#### Scenario: 确认后独立推进编码

- **WHEN** plan 已 Confirmed 且 enrollment 仍授权
- **THEN** 服务端独立 advance 把全部实际桶置 Ready，随后经共用 StartCoding 发起各桶 coding；advance 自身不启动 provider

#### Scenario: 首项落盘即全图 fan-out

- **WHEN** 首个真实 outcome 落盘且其余独立根已就绪
- **THEN** 服务端立即启动其余就绪桶，不等首个终态 unit；跨仓依赖上游完成后下游自动开始

#### Scenario: 非首桶 Enrolled origin 与失效零启动

- **WHEN** 非首桶按 AutoStartOnce 冻结策略启动，或启动前 binding 已撤销/篡改
- **THEN** 非首桶以当前 binding 的 Enrolled origin 启动并逐桶核对；失效时零 provider、不降级 Manual

#### Scenario: 未解除失败事实停点不自动重试

- **WHEN** 当前 binding 存在未解除的 advance/coding_start 失败事实
- **THEN** 编排器 tick 停在该点，不自动 retry；用户显式链继续（REQ-WIGA-11）成功后才恢复

#### Scenario: 失败或漏唤醒恢复

- **WHEN** 服务重启、事件漏投、计划创建已成功但绑定未写、或动作在持久检查点间中断
- **THEN** 后续扫描/补偿重用同一 plan/session/action 身份，已认领的启动只按权威事实恢复或进入显式人工分诊，不创建重复 plan、尝试或并行 provider run

#### Scenario: 活跃租约冲突停等

- **WHEN** restart、recover 或 new advance 重新读取到仍有活跃 owner/attempt
- **THEN** 系统保留当前 durable 状态，向驾驶舱和系统通知"请等待/已有运行"及绑定身份，不抢占、不启动第二 provider

#### Scenario: 死亡持有者经确认接管

- **WHEN** 重新读取确认旧 owner 已终态或已超过活动窗口且用户提交匹配当前版本的接管命令
- **THEN** 系统原子记录接管结果并继续原 enrollment 的安全恢复；旧 owner 的迟到写入被拒

#### Scenario: 活性未知保持等待

- **WHEN** lease/attempt claim 既不能证明活跃也不能证明死亡
- **THEN** 系统通知需要人工确认/重新绑定并保持停等，不自动 takeover、不盲目重启外部 provider

### Requirement: 编码 amendment 与恢复不承重于 live socket（REQ-WIGA-06）

对已授权后台运行的 coding（单仓、单成员或成员集合授权的全部实际桶），业务 amendment 应用/恢复 SHALL 在无 live coding socket 时继续推进；观察事件送达 SHALL 独立记录真实投递事实并允许重连后按 durable 事实补读，MUST NOT 因没有订阅者而假写 Delivered，也不得以假 socket 代替真实交付。coding choice、amendment 和 runner 恢复 SHALL 不依赖用户打开 Coding Workspace；人工处理门仍停等人操作，不得自动确认。

#### Scenario: 没有 coding 页面仍可应用 amendment

- **WHEN** 所有 coding WS 都已关闭，已获人工批准的 plan amendment 需要应用与恢复
- **THEN** 业务状态可继续到相应 durable 检查点，真实事件未投递则保持未送达并供重连补读，不以 `plan_amendment_coding_socket_unavailable` 拦截业务推进

#### Scenario: 观察面慢或重连不改变业务事实

- **WHEN** coding observer 迟滞/断连后重订阅，期间 amendment 已应用或 runner 已恢复
- **THEN** observer 从 durable 状态获取相同 amendment/执行结果；投递标记反映实际交付，不重复应用 amendment、不因观察端反压改写业务终态

### Requirement: 单一归属、进度下钻与非 enrolled 零回归（REQ-WIGA-08）

会话及 observer 投影 SHALL 从 durable enrollment 的精确绑定派生同源管理投影 `automation:{managed, enrollment_id?, policy_revision?, plan_id?, terminated_at?}`，REST 与 WS SHALL 由同一 manager 派生；terminated 后 SHALL 仍报 managed。仅仍绑定且授权的会话 owner=server，前端 `useCockpitAutopilot` SHALL 在归属未知时先停等、owner=server 时不再发送 advance，非 enrolled 会话继续既有 client 行为。对精确绑定当前 enabled enrollment 的 plan session，服务端 SHALL 一律拒绝 takeover（沿用既有错误码 `workspace_session_takeover_not_allowed`，不创建 Interactive 子 session、不变更绑定），驾驶舱收件箱 SHALL 隐藏该会话的接管入口；非该绑定范围的既有 takeover SHALL 保持原样，MUST NOT 仅按 AutoIfValid 猜测是否托管。plan 阶段停点与失败 SHALL 经持久自动化事实同源通知（REQ-WIGA-12）。WIG 入口 SHALL 显示既有编码仪表盘进度并可点入 Coding Workspace 实时查看；后台运行不要求页面持续流式渲染。非 enrolled 的手工业务、门禁、错误与恢复语义 SHALL 不变；源 story/design 人工流程及 Final Confirm SHALL 保持人工，自动化只改变已授权流程的执行发起者。

#### Scenario: server 归属前端退位

- **WHEN** enrolled 会话由 durable 投影确认 owner=server 或首帧尚未知归属
- **THEN** 前端不会抢发 advance，归属/修订变化时清理旧的待发意图；服务端 advance 幂等为已发命令兜底

#### Scenario: managed/terminated 同源投影

- **WHEN** REST 或 WS 读取任一自动化绑定会话的管理投影，含已终止 enrollment
- **THEN** 两侧返回同一派生结果且 terminated 后仍 managed，含 enrollment/policy/plan/terminated_at 身份

#### Scenario: 托管会话拒绝接管

- **WHEN** 用户对精确绑定当前 enabled enrollment 的 stopped/failed plan session 发起 takeover
- **THEN** 服务端以 `workspace_session_takeover_not_allowed` 拒绝，不创建 Interactive 子 session、不变更绑定；收件箱不显示该会话的接管按钮

#### Scenario: 非托管接管保持原样

- **WHEN** 会话无 enrollment 绑定或属手动流程，用户发起既有 takeover
- **THEN** 既有 takeover 行为与批 2 前一致，不因 AutoIfValid 字面值被额外拦截

#### Scenario: 从入口查看后台进度

- **WHEN** 后台 plan/coding 在没有打开执行页面的情况下推进，用户进入 WIG 入口
- **THEN** 可看到当前编码进度并经按钮进入 Coding Workspace 看实时执行/历史重放，无须保留此前的流式页面

#### Scenario: 两模式真实链对照

- **WHEN** 同一业务分别选择不自动化与授权集合自动化完成两条真实链
- **THEN** 手动链既有操作、门禁/错误/恢复与 Final Confirm 不变；自动链只在 AI 选择/人工门/choice/recovery/Final Confirm 停等，所有其他阶段无需保持页面打开

### Requirement: 单仓载体的无页面自动全链（REQ-WIGA-C5-CHAIN）

对单仓 target 或逻辑代码库成员集合的有效 enrollment，服务端 SHALL 沿既有路径完成自动链：幂等准备唯一 plan/session、启动一次生成、正常 outline/draft/batch/最终批准自动通过（REQ-WIGA-02）、仅在 AI choice 与人工门停等、trust/compile 恢复停点停等、确认后独立 advance 到全部实际桶 Ready、经共用 typed StartCoding 与共享 plan-run 驱动全部桶首启与续驱，coding 运行至各桶 `WaitingForHuman ∧ FinalConfirm`；最终收口 SHALL 只用批 1 既有 plan 级唯一人工最终确认，MUST NOT 自动代点。关闭所有页面与 coding socket 不影响推进。单仓链 MUST NOT 注入逻辑代码库 gateway、MUST NOT 创建逻辑代码库 manifest/selection/snapshot。prepare、Enrolled advance 与 StartCoding SHALL 各自复核 enrollment target 与当前权威载体一致：单仓 target 的 repository 身份 MUST 等于 issue 所属物理仓、plan 的全部工作项 MUST 无 logical target 归属、attempt MUST 无逻辑代码库 target 快照；单成员/集合 target 的成员 MUST ∈ 授权集合且实际桶满足 ∅≠P⊆A、无未归属 WI。任一不符 SHALL fail-closed 零副作用，并以既有等待项呈现需重新绑定。会话 owner 投影、驾驶舱等待项、plan 已确认与待最终确认信息 SHALL 对各载体使用同一派生与展示，并显示当前 target 身份。已终止 plan MUST NOT 出现可点最终确认。

#### Scenario: 单仓关闭页面后自动跑到最终确认

- **WHEN** 单仓 issue 经人确认 design 后明确 Enable 自动化，用户随即关闭全部页面与 socket，并仅在 AI choice/人工门出现时从驾驶舱处理
- **THEN** 系统自动生成 plan、正常批准自动通过、自动 advance 到 Ready 并 typed 首启，coding 完成后停在 FinalConfirm 并出现一次"待最终确认"信息；全程 provider 首启恰一次

#### Scenario: 集合成员链零人工推进到唯一确认

- **WHEN** 四成员仓 issue 选择自动化且各仓均有 WI 归属，主正例无故障推进
- **THEN** 全部实际桶自动建立并推进，跨仓依赖上游完成自动开下游；全部完成后进入批 1 plan 级唯一人工确认，一次点击收口

#### Scenario: 单仓链拒绝错仓与旧绑定

- **WHEN** 单仓 enrollment 的 target 与 issue 当前所属物理仓不一致，或 advance/StartCoding 回执携带旧绑定版本
- **THEN** 该动作零启动、零写入 attempt，驾驶舱出现需重新绑定的等待项，当前绑定不被覆盖

#### Scenario: 逻辑代码库 attempt target 与 enrollment 不符

- **WHEN** 集合/单成员 enrollment 的 plan 实际桶含越权成员、存在未归属 WI，或 attempt 快照指向授权集合外的 logical repository
- **THEN** Enrolled advance 或自动 StartCoding fail-closed 拒绝，不启动 provider

#### Scenario: 终止后不出现可点最终确认

- **WHEN** plan 已终止且部分 attempt 已 Completed
- **THEN** 批 1 plan 级确认入口不可点（保持接口不改），不出现诱导完成的聚合确认

#### Scenario: 手动与关闭模式不被接管

- **WHEN** 单仓 issue 选择手动或关闭 enrollment 后后台补偿扫描运行
- **THEN** 补偿扫描不准备 plan、不 advance、不启动 coding，手动链行为与本 change 前一致

## ADDED Requirements

### Requirement: 整 plan 不可恢复终止与统一 Abort 语义（REQ-WIGA-09）

系统 SHALL 提供 `POST …/automation-enrollment/terminate`（`{command_id, expected_enrollment_id, expected_policy_revision, expected_binding?, confirmed:true}`）。自动化绑定 plan 的门 Abort、原 abort REST、gate-responses REST、WS 与收件箱中止入口 SHALL 统一进入同一整 plan 终止与二次确认：未确认仅返回 `automation_plan_termination_required` 零副作用，确认后按原 command_id 重放整 plan 命令；auto 单仓同样覆盖，手动单 attempt 保持原样。终止不可恢复：收口失败整体 `cleanup_required` 仍不可继续；terminated 后 Enable/rebind/继续/新首启一律拒绝；迟到 Failed/Open MUST NOT 覆盖 Terminated；无 attempt 也关闸/清事实。

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

- **WHEN** 终止收口执行完成（含 blocked/choice/stage 各类 open 门与三类通知事实）
- **THEN** 全部 open 门关闭、旧事实清空；刷新后无幽灵门通知，收件箱不再出现该 plan 待办

#### Scenario: 清理失败可见且重清后消失

- **WHEN** 某仓收口异常
- **THEN** 整体返回 cleanup_required 并写 stage=termination 的 automation_failed（可点回该 plan）；重提同命令重做收口成功后该事实消失，仍不可恢复

#### Scenario: terminated 不可恢复且重启不复活

- **WHEN** enrollment terminated 后提交 Enable/rebind/continue，或服务重启触发补偿扫描
- **THEN** 一律拒绝/零动作，不降级 Manual、不复活任何 attempt

#### Scenario: 迟到写入不覆盖终态

- **WHEN** plan session 取消后迟到的 Failed/Open 完成写入尝试
- **THEN** Terminated 终态不被覆盖；已 Confirmed 的 attempt 状态不改

#### Scenario: 手动单 attempt abort 不变

- **WHEN** 无自动化绑定的 attempt 走原 abort
- **THEN** 单 attempt abort 语义与批 2 前一致（含 clean 检查），不进入 plan 终止

#### Scenario: 同命令重放幂等

- **WHEN** 以相同 command_id 重复提交同一 terminate
- **THEN** 重做/返回收口结果，不重复取消、不重复关门、不启动 provider

### Requirement: plan 阶段失败继续（对话窗口）（REQ-WIGA-10）

系统 SHALL 提供 plan 阶段失败继续入口 continue-plan（`{command_id, expected_binding, expected_stop_key, context?}`）与只读 `GET …/control` 管理投影（字段见 REQ-WIGA-08 与场景；初版不含 takeover）。继续 SHALL 在同一 plan 对话窗口补说明＋继续：保留 enrollment/plan/session/auto 策略，不建子 session、不转手动、不改绑定、不与 coder 门混用。服务端 SHALL 核授权未 terminated、manager 互斥无活 run、持久认领命令、写 context 后按事实分支：Failed 复用 SC 失败重臂与重生成；评审超限同 session 重开 HumanConfirm（预算耗尽只开一个新说明回合）；既有 choice/门照旧。派发前重验授权；失败保留事实＋通知，MUST NOT 自动改 Interactive、跳门或吞异常。

#### Scenario: provider 失败修好原窗口继续仍 auto

- **WHEN** plan 生成因 provider 失败停等（通知已发），用户修好 provider 后在原 plan 对话窗口点继续（说明可空）
- **THEN** 旧 generation intent 重置为 Claimed 后按真实启动推进，enrollment/plan/session/auto 策略不变，自动链继续

#### Scenario: 评审超限对话说明继续

- **WHEN** plan 评审超限停在人工停点，用户在对话输入区填写说明并继续
- **THEN** 同 session 原地重开 HumanConfirm 并产 TurnOpened；预算已耗尽时本次继续只开放一个新说明回合，不自动追加预算

#### Scenario: 终止后拒绝继续

- **WHEN** enrollment 已 terminated 后提交 continue-plan
- **THEN** 返回终止原因并拒绝，零派发、零状态变更

#### Scenario: 同键重放幂等与异负载拒绝

- **WHEN** 相同 command_id 同负载重复提交，或同键异负载提交
- **THEN** 前者返回首次结果不重复写 note/派发；后者拒绝
- **AND** 响应为 `{state:accepted|replayed|needs_human, session_id, reason_code?}`；持久认领命令在 manager 互斥且无活 run 时进行

#### Scenario: control 只读投影真实停点

- **WHEN** 读取 control 而会话停在 trust waiting/compile 恢复门/choice
- **THEN** 返回真实 stop_key/stage/allowed_actions/context_required，allowed 动作不含 takeover；GET 不写任何事实
- **AND** 投影字段为 `{managed, binding?, plan_id?, session_id?, terminated_at?, stop_key?, stage, allowed_actions, context_required}`，按 durable session/门/intent 及持久事实判定

#### Scenario: 停点键不匹配拒绝

- **WHEN** 提交的 expected_stop_key 与服务端当前停点不符
- **THEN** 拒绝继续并提示刷新，不把旧说明作用于新停点

### Requirement: advance/coding_start 链停点继续（REQ-WIGA-11）

系统 SHALL 提供 `POST …/automation-enrollment/continue-chain`（`{command_id, expected_binding, expected_stop_key, confirm_unknown_side_effect?}`），作为 advance/coding_start 停点的第三类显式继续：不携带说明文本、不是 plan 对话继续也不是 coder 门。advance 停点 SHALL 复用既有 retry-initialization 并沿冻结 `start_run_policy`（不覆盖旧 Manual）；coding_start 停点 SHALL 仅重放原 Claimed/RunnerRegistered claim，越 provider barrier 的未知状态不清 claim、走既有恢复路径并核 auto。成功后 resolve 对应失败事实并唤醒驱动；失败保留事实与 auto 模式、不新增 coder 说明通道；编排器对未解除失败事实停点不自动重试（REQ-WIGA-03）。

#### Scenario: advance 失败显式继续保留 AutoStartOnce

- **WHEN** Enrolled 初始化失败落 automation_failed(stage=advance)，用户显式链继续
- **THEN** 系统按冻结 policy 重试初始化并 resolve 原事实，AutoStartOnce 冻结不变、不降级 Manual

#### Scenario: start 失败仅重放原 claim

- **WHEN** coding_start 失败停在 claim 未越 barrier，用户显式链继续
- **THEN** 仅重放原 Claimed/RunnerRegistered claim，不重复首启、不重复 provider；越 barrier 的未知状态走既有恢复并保持 auto

#### Scenario: 成功后事实消失

- **WHEN** 链继续成功
- **THEN** 对应 automation_failed 事实被 resolve，通知投影消失，驱动恢复推进

#### Scenario: 未解除失败编排器停点

- **WHEN** 失败事实存在且用户尚未显式继续
- **THEN** 编排器 tick 停在该点不自动 retry，通知保留

### Requirement: plan 阶段自动化通知事实与接入（REQ-WIGA-12）

系统 SHALL 复用批 1 durable 事实投影 DTO 与 target（不建第二模型、不伪造 attempt、不加 focus.session_id），追加 kind＝plan_question（choice/人工门/trust/compile 恢复门）、automation_failed（provider/链停/清理失败）、review_limit（plan 评审超限）；plan 侧用 workspace_session 目标。enrollment SHALL 持久 attention_facts（缺省空、不存已读/推送）并提供按精确绑定的 upsert/resolve（同一 terminated 绑定可清/写清理失败；旧绑定拒绝）。写者固定：trust/compile 门、其余 plan 停点（含派发前失败）、advance/coding_start、终止清写各归其包；事实有稳定 key 与 durable 时间，重复不增项，写失败上抛。读取侧只读合并进 lifecycle 事项；未 watched 也可见；失效 choice 答后清；终止后无幽灵通知。

#### Scenario: trust waiting 未开页面仍通知

- **WHEN** 自动 compile 前 trust 未 Ready 落 typed waiting，用户从未打开该 plan 页面
- **THEN** 驾驶舱收到 plan_question 通知并可点入对应 session；trust 转 Ready 后事实 resolve、通知消失

#### Scenario: 派发前失败无活窗口仍通知

- **WHEN** plan 生成在 provider 派发前失败转 NeedsHuman（无活 run/对话窗口）
- **THEN** 写 automation_failed 事实并通知；稳定 key 下重复停点不增项、零重复 provider

#### Scenario: 终止后无幽灵通知

- **WHEN** plan 终止且收口完成
- **THEN** 三类旧事实清空，刷新/重连后不出现该 plan 的待办或门通知

#### Scenario: 失效 choice 清理不误清失败事实

- **WHEN** choice 答后或 run 结束（按 incarnation）触发清理，同时存在 durable Failed/review_limit 事实
- **THEN** 失效 choice 项消失，Failed/review_limit 事实与通知保留

#### Scenario: 未 watched 全局可见

- **WHEN** 产生 plan_question/automation_failed 的 issue 不在用户 watched 集合
- **THEN** 驾驶舱全局计数/展示该事项，点击经既有路由进入对应 session

#### Scenario: 写失败上抛

- **WHEN** upsert/resolve 因存储错误失败
- **THEN** 调用方收到错误并保留停点，不冒充已通知、不静默吞错

### Requirement: 自动化整链验收不变量（REQ-WIGA-13）

自动化整链 SHALL 满足：design 人工确认后选自动化，全程不手点 prepare/start/approval/advance——自动生成、正常批准自动通过、有 WI 目标桶的 attempt 与 plan-run 自动建立并推进；上游完成自动开下游且 coder 可读批 1 真实上游交付；无 WI 的授权仓不造空 attempt；全部完成后仅一次批 1 plan 级人工最终确认（不自动代点）。故障用例（plan choice/provider 失败/评审超限/coder 挂起与环境门/review 超限）与零人工正例分开执行且均保留 auto。terminated＋部分 Completed 不出现可点最终确认（确认接口不改）。手动单/多仓回归不变；单仓显式选 auto 同样获得自动链与统一终止；不以单测替代整链验收。

#### Scenario: 四仓零人工正例

- **WHEN** 新 issue 选四仓 explicit selection、人工完成 story/design 并确认，创建 plan 选自动化，此后不手工操作
- **THEN** 自动生成/批准/建桶/推进/跨仓下游自动开始，最终 A2 ready 仅一次人工确认收口

#### Scenario: 无 WI 仓不造空组

- **WHEN** 四仓授权中一仓无任何 WI 归属
- **THEN** 该仓无 attempt/空组，其余三仓正常推进

#### Scenario: 故障后继续仍 auto

- **WHEN** 任一故障用例（choice/provider/超限/挂起/环境门）处理后继续
- **THEN** 自动化模式保留（plan 对话窗口或 coder 门按归属通道），不出现转手动/暂停入口

#### Scenario: 终止后不可最终确认

- **WHEN** terminated plan 存在部分 Completed attempt
- **THEN** 批 1 最终确认不可点，负例通过且确认接口零改动

#### Scenario: 手动回归不变

- **WHEN** 手动单仓与手动多仓链执行原有操作
- **THEN** 确认/中止/门行为与批 2 前一致；单仓显式选 auto 获得自动链并可统一终止
