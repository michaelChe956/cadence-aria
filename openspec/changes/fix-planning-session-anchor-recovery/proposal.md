# Proposal: fix-planning-session-anchor-recovery

## Why

正常规划产物回写 involved/focus/change_order 会把 native launch target 从聚合根移到成员仓，正确的安全指纹拒绝因此卡住常态修订；实际恢复链又缺 durable waiting、full map 持久化、明确 fresh 授权与完整上下文回灌。owner 已确认 E+A–D 的 v1.1 设计：固定规划 native anchor，真实漂移继续失败关闭，并显式换新 native 会话继续原 workspace。

## What Changes

- 固定 LC Story/Design/WorkItemPlan PlanningReadOnly native target 为 canonical aggregate root；同一 resolver 独立保留原业务 repository、成员材料和 compile target，aggregate view ID 不得流入 IR；Coding/direct 语义不变。
- 将 full fingerprint/native/run binding、superseded tombstone、tagged 原 run-context、fresh operation 结果持久化；mismatch、缺 full map、Plan stale 均零 provider session start、旧 provider node 终态并进入稳定 waiting。
- latest durable record 驱动 driver/observer/live/attach/重连投影，纯 fallback 零恢复写入；读取失败 typed 拒绝，不发布旧 record 或可启动假态。
- **BREAKING（LC 存量规划恢复行为）**：原缺 map 后隐式 fresh 与 Plan stale 自动 OutlineRebuild 改为 waiting＋明确新会话；owner 已批准存量首次继续的一次性显式 fresh，不做特殊回填。已有 WS StartGeneration 新增可选恢复参数，普通出生/direct 消息兼容。
- 明确 StartGeneration.resume_recovery 的 fresh 授权及耐久 Replay 顺序，typed operation 响应；仅更换 native，不换 workspace/entity。legacy outline/Draft/Batch/PlanRevision、SC author/manual 按原 run/turn 完成，不重臂 gate/预算，不捞旧 audit ID。
- 按既有 Task9a 接 full candidate projection，trust 仅只读事实；允许 owner 2026-10-11 已裁定的有界只读 --version 观测，固定 60 秒非滑动 TTL、同键 single-flight。probe/adapter-start/session-child 分别计数；mismatch 后两者为零。
- typed inbox 提供明确“启动新会话继续修订”，resolved 卡消失，后续正常新 native 恢复；保留 lease/provider/连接/终态守卫，不走批量确认/advance/interrupted retry 自动重放。

## Capabilities

### New Capabilities

- `workspace-session-resume-recovery`：持久等待与吊销、native 冻结记录、显式 fresh/Replay/原 run-context、最新 record 纯投影、driver/observer 恢复动作。

### Modified Capabilities

- `logical-codebase-aggregate-planning`：MODIFIED REQ-PLN-03/05/07，ADDED REQ-PLN-08；同 resolver 分离 native anchor 与业务/compile target，scope 收敛不变，真实 snapshot 漂移仍拒旧。
- `session-policy-envelope`：MODIFIED REQ-ENV-04/08，沿未归档 root-cwd 基线消除自动 fresh；REQ-ENV-10/11 交叉引用，版本观测口径明确。
- `lc-gateway-multi-provider`：以未归档源 change 的真实 requirement 标题修改/澄清 admission target、完整 projection、resume 与版本观测；不重造 REQ-LCG 编号。

## 规格来源与时序

获批设计：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/designs/2026-10-11_方案设计_规划阶段会话锚点固定与漂移恢复闭环_v1.1.md`（owner 已在本对话确认，本文不重复字段级实施步骤）。REQ-PLN-08 已核主规划规格只到 07；源 `lc-root-initialization` 的 ENV04/08 有“拒 resume 并启动新会话”旧句，较新 gateway source 已禁止静默 fresh。gateway capability 尚未在主 specs 建立，本 delta 的 MODIFIED 标题来自 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/openspec/changes/lc-gateway-multi-provider/specs/lc-gateway-multi-provider/spec.md`，不是伪造主 spec 条目。

同步/归档前必须核实际状态：源先归档则基于最新主规格；源未归档则按获批 root→gateway→本 change 次序同步，保留 native aggregate 与 Coding member 的具体化，不让旧 delta 覆盖新安全合同。本次不改源 change/主规格。REQ-LCG 仅按源 change＋标题映射。owner 对 --version 的裁决是“有界只读版本观测不算创建会话进程”，60 秒内变化识别可能滞后；不是扩大 provider 会话 spawn 或其它事实缓存。

## Non-Goals

- 不改变原成员 language.md/仓结构/IR target/compile 规则，不新造 resolver、默认选首仓、写面扩大或 OS 级硬只读证明。
- 不新建用户 workspace/entity，不删除产物、反馈、review、audit，不为旧 session 回填当前指纹造相等。
- 不改 Coding、RootRecipe、初始化/direct 合同；无 native contract 的同步 split 不 fallback streaming fresh；未知 run tag 拒绝不转 plan_author。
- 不 ensure/revoke，不改 trust 登记/确认 CAS、SC gate/turn/预算；不自动重试、后台 fresh、批量 fresh 或跨介质事务承诺。
- 不新增依赖，不替代 capability/boundary 真实证据；外部非协作文件写方竞态如实，不能宣称 sidecar 消除全部 TOCTOU。
- 本次只创建契约与审阅用实施 Plan，不写实现代码、不运行产品检查、不 commit；实施等待已获批 Plan。

## Impact

产品：workspace resolver 两用途、durable DTO/CAS/真实恢复、gateway full prepare 与版本观测、四 provider facts 与 native 审计、Plan 全 run 族上下文/SC 同 turn 恢复。协议：optional fresh 参数、typed result/SessionState recovery、latest-record 读取失败合同；前端 driver/observer/store/inbox 共同消费。发布：A/B 共同可用或中间不可启动；与 trust T4/T5.2/T6.1/T7 同文件串行移交。验收：TDD 三口径计数、同存活 manager 持锁 fallback、SC 材料→IR→compile、legacy feedback/manual turn 真 fresh、重复 Replay/断点与真实全链。
