# Proposal: enrollment-recovery-surface

## Why

当前自动化 enrollment 的 plan 链在候选/relay 失败、租约残留、绑定冲突或 advance 初始化失败时，能够保留部分事实却缺少与这些事实对应的用户操作面，导致用户只能看到停滞或被迫绕过 durable 记录。方案 v1.2 将本轮自动化定义为“自动推进＋人到点处理”：错误必须通知、停等并提供受约束的恢复操作，而不是后台猜测接管、清绑定或重放未知副作用。

## What Changes

- 为 enrollment 及其 plan/session/target 建立载体无关的显式绑定合同；用户可在驾驶舱明确选择 plan、session、source、target、provider 后执行“重新绑定／换代”，写入新的绑定版本，旧代只读保留，迟到旧回执和同代重复操作被拒绝或幂等处理。
- 为孤儿/降级 plan 门持久化完整 candidate、source revision、budget 与 gate 事实；relay 或 WS consumer 失败时通知用户并提供“恢复运行／从权威候选重建”，缺少完整快照时不得裸 approve，恢复后回到既有 feedback/approve 流程。
- 为 Failed advance 提供显式 `retry-initialization` 操作：保留原失败审计和独立 retry 事实，核对当前 plan、revision、attempt、target 与安全 checkpoint；可证明安全的本地初始化可继续，外部副作用不明时停等确认，不创建第二 attempt。
- 在每次 restart/recover/new advance 时重读租约/attempt claim；活跃冲突通知等待，已死亡的持有者仅在用户确认接管后继续，未知活性不自动抢占，并拒绝迟到旧锁写入。
- 在既有 plan contract 中表达 `existing`/`create` 意图及 provider Work Item；缺失或越权时停等计划反馈/修订，区分“未声明”与“不能执行”，不自动扩大 scope。
- 使换代后的 compile child 同时按 plan、plan revision、work item revision 和当前绑定身份精确筛选；显式创建新 child，旧 child/binding 只读保留，不清空或覆盖历史绑定。
- 复用驾驶舱 inbox/系统通知呈现每个等待项的失败原因、已完成步骤、目标身份、plan/attempt/gate、可能副作用、可用按钮及成功后的下一阶段；操作带稳定 `command_id` 和 expected version/绑定身份，重复命令幂等，过期版本 fail-closed。

## Capabilities

### New Capabilities

- `enrollment-recovery-surface`: enrollment 绑定版本、plan 门恢复、租约处理、advance retry、existing/create 意图与通知/操作闭环。

### Modified Capabilities

- `work-item-group-autopilot`: 修改 enrollment 的 target/绑定版本、操作性停等与自动推进边界；保留人工 plan 门、单 target 授权和非 enrolled 零回归。
- `work-item-plan-advance`: 增加 Failed advance 的显式 retry-initialization 语义、审计保留与同 attempt 恢复约束。
- `work-item-plan-conversational-gate`: 增加孤儿/降级 candidate 快照及恢复/重建操作边界；恢复后继续既有 typed feedback/approve/abandon 协议。
- `work-item-plan-single-candidate`: 增加 plan contract 的 `existing`/`create` 意图、provider Work Item 与缺失/越权时的停等语义。
- `work-item-group-coding-execution`: 增加换代后 compile child 的精确绑定筛选和旧 child 只读保留。
- `web-runtime-repository-routing`: 复用并扩展单仓/逻辑代码库的显式 target union，确保 enrollment、plan、advance 与后续通知使用同一 target 身份。

## Impact

- 影响 enrollment、绑定、plan gate/advance、compile child、租约/attempt claim 及驾驶舱通知和操作 DTO/应用服务；实现必须复用现有 CAS、journal、gate、lifecycle 与只读投影，不新增全局编排器、owner/fence、统一 generation 服务或第二套 durable operation 状态机。
- 方案 v1.2 §5.6 的受影响 capability 中，C1 只修改上述直接冲突的 requirement；不把 coding 侧断连/验证分诊、LC 冷启动或单仓 role-chain 准入纳入本 change。
- 验收映射到方案 v1.2 §4.2 的 #1/#6/#9 与 §4.3 的 #11/#12/#14/#16，以及 §7 的 A07、A09、A12、A13；每个故障均须观察“错误→通知→用户操作→自动续进”。

## Non-Goals

- 不后台自动迁移、按“最新 plan/session”猜接管、自动 takeover、清除或重绑旧 child/binding，不把旧代改写为当前代。
- 不建设独立 durable generation/owner/fence 服务、第二套恢复编排器、全局验证影子库或新的通知数据库。
- 不自动批准/反馈/放弃人工 plan 门，不启动 coding provider，不代替 Final Confirm；coding runner、provider 断连/指令消费/验证分诊属于 C2。
- 不实现逻辑代码库冷启动、pi 初始化 recipe、单仓完整 role-chain gateway 预检或其他方案 v1.2 明确 defer 的项目。
