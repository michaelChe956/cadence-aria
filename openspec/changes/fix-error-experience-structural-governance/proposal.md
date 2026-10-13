# Proposal

## Why

依赖×仓序、成员归属及非法 heading 连续外溢到用户，用户被迫替系统写技术修订反馈，甚至未收到合法候选仍消耗反馈轮次。采用 owner 于 2026-10-13 全部批准的 `cadence/designs/2026-10-13_方案设计_五阶段错误体验_确定性结构治理_v1.2.md`，将确定性结构问题在系统门内识别、机器指令驱动 author 修复、全量复验并审计闭合。

## What Changes

- 新增 L0 运行恢复 / L1 确定性结构治理 / L2 技术逻辑 / L3 业务决策分层；L1 多解也是系统与 author 的技术工作，不要求用户选择仓 UUID、删边或修 heading。仅事实证明必须改变已声明业务契约时进入原业务门。
- 单一结构套件检查 grammar/canonical/split/preflight、target×成员归属、Inputs×change_order 有效图及 scope/intent/trusted commands；candidate 门与 Final Compile 复用同核心。坏候选不提升 current refs、不送 reviewer、不允许确认发布。
- 机器生成来源边、可核验有向 witness、保护项与精确指令；所有新内容修复（含唯一解及原 autorepair 语义内容修改）经原授权 author、完整复验与原 reviewer。只有核验过整组 immutable refs 的纯状态回退可直执，并失效旧 approval/compile reservation。
- 建立合法 checkpoint 与 resolved structure predicates，连续 R1 环→R2 非法 heading→R3 回退再犯不能无限循环，也不能删掉真实 Inputs/AC 假通过。
- **BREAKING：显式覆盖原 B 预算合同。** 用户反馈预算沿同 logical gate 最多 3 轮；durable hold 占可用轮次，合法业务对应候选交付才 `delivered_charge`。SC 同 turn 原调用＋追加调用合计最多 2 physical attempts，L0/L1 共享该物理上限、不乘池。仅 `structural_exhausted_before_delivery` 释放 hold；provider/timeout/取消等非结构失败仍原预约扣次不退，记 `nonstructural_failure_charge`。同步 MODIFIED REQ-CG-02/03/04/05，不伪称澄清或所有未交付免费。
- story/design/plan/coding/review 五阶段均纳入适用自身 schema/scope 的 L1 自动结构修复；继承 v1.1 typed producer、durable grant、复评防旁路、完整 prompt、reservation/actual start 分账及四错误泄漏面的同源呈现。普通、legacy、初轮、coding/review 只能消费既有授权及已批准机会，不能新增隐含执行权限。
- 主区默认“系统正在整理方案结构”，不刷 attempt 次数；详情分别呈 user settlement、structural/mechanical attempt、actual start。系统处理/已解决、业务决定、最终工程故障分域；可选采纳机器建议仅预填当前合法编辑器、零出站，不是闭环前提。
- 保留终态 command_id 稳定拒收及可见回显、完整 gateway admission/full fingerprint、anchor 显式 fresh 同 run/turn/prompt join、trust 只读安全边界及原 confirm/compile/advance 门。

## Capabilities

### New Capabilities

- `workspace-structural-governance`：五阶段结构校验、机器修复指令、授权闭环、合法历史恢复、耐久身份/预算事件及统一错误操作面。

### Modified Capabilities

- `work-item-plan-conversational-gate`：REQ-CG-02/03/04/05 的 hold/交付/非结构失败扣次/结构退款、同 turn 两 attempt、结构耗尽与用户预算耗尽区分、恢复 CAS。
- `logical-codebase-aggregate-planning`：新增 candidate/compile 同源有效图、成员归属与系统前置结构门，原 change_order 方向/成员权限不变。
- `coding-provider-transport-retry`：仅解除首 requirement 对正常输出结构无效自动处理的 L1 禁令；transport 合同和 coder 既有 teaching/cycle、group review repair 最大组合不改、不叠新池。

## Impact

影响 workspace engine 的结构 producer、投影/编译、prompt/author/review/recovery、lifecycle durable turn/session/checkpoint、WS DTO 和 web live/rebuild/observer/cockpit/editor；扩现就地测试及事故链 fixture，不另造平行 harness。以 v1.2 §11 独占文件工作包为主体，继承基础与 coding/review 接线在实施 Plan 明列，按 trust/anchor 移交证据串行接管共同文件。

冻结依赖不是本 change 的修改面：先组合 `lc-root-initialization` 与 `lc-gateway-multi-provider` 来源，再叠 admission 收窄与 trust，最后叠最新 `fix-planning-session-anchor-recovery`，本 change 最后应用；详细标题覆盖规则见 design。当前未归档来源不得从旧主规格恢复自动 fresh。本次只生成规划产物和 strict 验证，无实现、无 commit。

## Non-Goals

不改变已确认 Design 仓序、真实业务 Inputs/AC、成员写权限、provider 信任登记政策、anchor/fingerprint 或 confirm/advance 许可；不通过 L1 新开无人授权 author run、不复活旧 WS 命令、不将所有未交付终态退款；不系统直 patch 已持久/已评审内容，不保证模型永不格式错，不承诺跨 JSON 文件事务原子性或 OS 级只读。09:31/d4c4 为事故标识，未经 refs/hash/scope/current context 核验不得硬编码恢复。
