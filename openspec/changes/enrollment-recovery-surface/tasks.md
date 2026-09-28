# Tasks

## 1. Enrollment target、绑定版本与可操作通知（REQ-WIGA-01、REQ-WIGA-03、REQ-WIGA-04、REQ-C1-TARGET-01、REQ-C1-CHILD-01）

- [x] 1.1 冻结并贯通单仓/逻辑代码库 target union、plan/session/source/provider/binding version 合同及 CAS/expected-version 操作语义；迁移所有 enrollment、advance、compile child、通知和恢复动作到同一精确绑定，保留旧代只读并拒绝迟到回执。
- [x] 1.2 提供驾驶舱/inbox 的重新绑定／换代、租约等待/确认接管和 child 恢复操作面；确保 command_id 幂等、过期版本 fail-closed、活跃/未知租约不抢占，成功后唤回原编排链。

**验收映射：** A09 租约冲突、A13 多次换代；work-item-group-autopilot、work-item-group-coding-execution、web-runtime-repository-routing delta。

## 2. Plan 候选门恢复与 existing/create 合同（REQ-C1-GATE-01、REQ-C1-GATE-02、REQ-C1-PLAN-01）

- [x] 2.1 将 candidate/source/budget/gate/诊断事实在 relay/WS 观察失败前持久化，补齐恢复运行/从权威候选重建的通知与 REST/驾驶舱动作；无完整快照时禁止裸 approve，恢复后复用既有 typed feedback/approve/abandon 协议。
- [x] 2.2 扩展 plan contract 与编译校验以表达 existing/create、provider Work Item、依赖及 scope 约束；区分未声明与不能执行的错误并提供反馈/修订停等，不扩大 scope、不清除 finding。

**验收映射：** A07 孤儿/降级门、A12 新文件 AC；work-item-plan-conversational-gate、work-item-plan-single-candidate delta。

## 3. Advance retry 与端到端故障闭环（REQ-ADV-C1-RETRY 及 C1 §5.1 共同约束）

- [x] 3.1 增加 `retry-initialization` 产品动作及 durable retry 审计：核对 plan/revision/target/attempt/checkpoint，保留 Failed 原因并在安全本地边界内续做同一 attempt；普通 advance 不隐式重试，外部副作用未知时转确认/重绑等待。
- [ ] 3.2 将所有 C1 等待项接入统一通知/操作结果投影，证明页面或 WS 断开不取消 durable 事实，重复 command_id 不重复 provider/attempt/候选/预算，并覆盖 A07/A09/A12/A13 的“错误→通知→用户操作→自动续进”真实观察链。

**验收映射：** A09 plan retry、A07/A12/A13 综合验收；work-item-plan-advance 与相关 capability delta。

## 4. 契约集成与兼容边界

- [ ] 4.1 更新既有 capability 的受影响 requirement 与交叉引用，验证 C1 target/binding 写面被 C2/C5 消费而不复制；旧 enrollment/缺失字段按 off/Manual/手动路径解释，非 enrolled、人工 plan 门和 Final Confirm 零回归。
- [ ] 4.2 完成 C1 合同的定向回归与方案 v1.2 §7 的验收记录，明确 C2 coding resilience、C4 LC bootstrap、C5 role-chain/pi defer 不在本 change 实现范围。

**验收映射：** 方案 v1.2 §5.1、§5.5、§5.6、§7；A07、A09、A12、A13。
