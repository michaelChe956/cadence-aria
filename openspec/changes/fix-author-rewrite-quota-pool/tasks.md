# Tasks: fix-author-rewrite-quota-pool

> **本 change 只立项，尚未实施** —— 下列条目全部是**待办清单**（未勾选），不是已完成记录。
> owner 已拍板选项 A（把语义做进执法，契约不动）；依据与决策见 `proposal.md` / `design.md`。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。

> **实施提示（2026-11-03 立项时留痕）**：本 change 的**回归面最集中**的两处是
> ① **第 2 组**要改写的既有断言 `src/product/workspace_engine/tests/compile_failure_auto_rewrite.rs:254-260`（`cycle.repairs_used == 1`）；
> ② **第 3 组**的 provider 预领 / CAS 幂等链路。
> 其余各组（预算域归一、迁移兼容）基本都是「加字段 + 读侧派生」，风险低。
> ⇒ 排实施 Plan 时把这两处的**回归验证**放在最前面。

## 1. author 重写额度归一为单一预算域

映射 requirement：MODIFIED REQ-WSC-21（余额只有一个权威事实）

- [ ] 1.1 盘点并冻结现状：author 重写次数预算的**全部**读写点（`auto_rewrites_used` 的写入/读取、`manual_repairs_remaining` 的写入/读取、快照建立公式 `max_manual_repairs − manual_repairs_used`、门反馈 hold / receipt / 退款路径），产出「谁在读余额」的完整清单
- [ ] 1.2 在 session 级 durable 记录里建立**唯一预算域**：总额 `max_manual_repairs`、已计费的「自动重写段」、已计费的「门反馈段」、活动预留（沿用既有 `UserBudgetHold`）；可用余额按 `总额 − 已计费 − 活动预留` 计算，沿用 `StructuralGovernanceState::available_user_budget` 的既有模式，**不得新造第二套预算模型**
- [ ] 1.3 把平台自动重写与 owner 反馈两条消费路径接到**同一预算域**；MUST NOT 保留任何「自动重写侧余额」与「反馈侧余额」互不相干的读法
- [ ] 1.4 `CandidateCompileStatus.auto_rewrites_used` 降级为**诊断计数**（保留 REQ-WSC-25「模型重写次数」可观测），MUST NOT 继续被任何执法点或呈现点当作授权余额来源
- [ ] 1.5 `decide_route` 的 `remaining` 与所有执法判定 MUST 取**同一个**余额事实源；MUST NOT 出现「一处读诊断计数、一处读预算域」
- [ ] 1.6 用例：自动重写 1 次 + owner 反馈 1 次 → 权威余额为 1（不是 2、不是各自 2）；纯自动重写 3 次 → 余额 0 且停；纯反馈 3 次 → 余额 0

## 2. 平台自动重写改扣 B 池，A 池不再被挪用

映射 requirement：MODIFIED REQ-WSC-21（两个预算池互不挪用 / 平台自动重写不得消耗机械返修池）

- [ ] 2.1 平台自动重写路径（编译失败驱动的 author 重跑）MUST NOT 产生 A 池扣次：轮内 `repairs_used` 与会话观测 `repairs_used` 均不递增；A 池继续服务「评审 finding 驱动的机械聚合返修」，其语义与上限不变
- [ ] 2.2 `routing_platform_rescue.rs` 的 A 池守卫（`cycle.repairs_used >= max_repairs`）改为读**B 池可用余额**；MUST NOT 因机械返修池已用尽而阻断平台自动重写
- [ ] 2.3 **同批改写既有测试** `src/product/workspace_engine/tests/compile_failure_auto_rewrite.rs`：`cycle.repairs_used == 1` 的断言改为「**B 池余额减 1** 且 A 池 `repairs_used` 不递增」；MUST NOT 削弱「自动重写必须消耗（可观测的）预算」这层断言的强度
- [ ] 2.4 复核既有测试中其它钉住旧池行为的断言（`compile_failure_auto_rewrite.rs` 全量 + 其它引用 `repairs_used` 的 SC 用例），逐条判定「改口径」还是「不动」
- [ ] 2.5 用例：编译失败 → 平台自动重写 → A 池 `repairs_used == 0`、B 池余额减 1；A 池已用尽时平台自动重写**仍可**进行（只要 B 池有余额）

## 3. 扣减的原子性、幂等与对账

映射 requirement：MODIFIED REQ-WSC-21（互扣同一余额 / 余额只有一个权威事实）

- [ ] 3.1 平台自动重写的预算扣减 MUST 与「author 重跑预领 / 开始」在**同一个 session CAS** 内完成（同 REQ-CG-02 的 hold + provider-start ledger 模式）；MUST NOT 出现「扣了但没跑」或「跑了没扣」
- [ ] 3.2 扣减 MUST 幂等：同一 command / 同一 provider-start 键重放 MUST NOT 重复扣
- [ ] 3.3 「自动重写未真正委托」的对账 MUST 同时**回退该次扣减**（现状只把 phase 改成 stopped、不减计数）；MUST NOT 出现「没跑也占额度」
- [ ] 3.4 崩溃恢复路径（进程在扣减后、委托前中断）MUST 可判定并收敛到「已扣已跑」或「已退未跑」两态之一，不得留悬空扣减
- [ ] 3.5 用例：同一 command 重放 → 余额只扣一次；委托失败 → 余额回退；恢复后账目与实际重写次数一致

## 4. 「编译成功时两个字段谁清」的落地

映射 requirement：MODIFIED REQ-WSC-21（编译成功只清编译链上的自动重写消费）

- [ ] 4.1 编译成功 MUST 清零：编译链上的「平台自动重写」诊断计数（B1）**及其**在预算域里对应的已计费消费；MUST NOT 返还同一 logical gate 内已由 owner 反馈消费或已预留的门反馈额度（B2 段）
- [ ] 4.2 门反馈段 MUST 继续按 REQ-HGC-01（同一 logical gate 内 carry-forward）与 REQ-CG-02（预留 / 结算 / 退款）的既有语义处理；MUST NOT 因编译成功而被整池重置回默认
- [ ] 4.3 确认实现与源码注释一致：现状 `cleared_candidate_compile_status` 写死 `auto_rewrites_used: 0`（注释「编译成功＝账本清零」）—— 该行为**保留**，但注释 MUST 写清「清的是编译链上的自动重写段」
- [ ] 4.4 显式记录**未采用**的口径（编译成功即整池回满额）及其前提（必须同时修改 REQ-HGC-01 / REQ-CG-02），MUST NOT 在代码或注释里留下「整池清零」的暗示
- [ ] 4.5 用例：自动重写 2 次 + owner 反馈 1 次 → 编译成功后余额为 2（自动重写段归零、反馈段保留）；同一门内再次反馈 → 余额 1（carry-forward 未被重置）

## 5. 迁移、旧记录兼容与可观测

映射 requirement：MODIFIED REQ-WSC-21（余额只有一个权威事实）

- [ ] 5.1 新增预算字段一律 `#[serde(default)]` + `skip_serializing_if`；老记录仍可反序列化
- [ ] 5.2 老记录缺新事实时按「余额未知」保守处理：MUST NOT 补默认满额、不得让旧会话看起来还有 3 次（呈现口径与块1「旧数据缺事实」规则一致）
- [ ] 5.3 已发生的消费事实按「已计费」并入统一预算域：MUST NOT 因合并而凭空多给或抹掉已消费的额度；归并 MUST 幂等（按 turn / provider-start 键去重）
- [ ] 5.4 迁移 MUST NOT 批量改写历史 durable 记录文件（只在新事实时按新口径计算，读侧派生）
- [ ] 5.5 可观测：自动重写次数、门反馈轮数、两段已计费与活动预留的原始计数 MUST 仍可查（REQ-WSC-25 的尝试序列不丢）

## 6. 跨 change 登记（与块1 G-2 的顺序约束）

> **这不是 task 组，是跨 change 的顺序约束登记** —— 块1 change（`fix-automation-enrollment-provider-swap`）的门卡呈现口径在本 change 落地后过期。

| # | 事项 | 顺序 | 说明 |
|---|---|---|---|
| 1 | 块1 的 REQ-HGC-01 呈现口径（「分别呈现两个账面事实」+「两处账本尚未合并」说明 + 「当前后端尚未提供该事实 → MUST NOT 展示共享余额」） | **本 change 落地后** | 本 change 让共池成为事实 → 块1 的该条 requirement 须同步升级为「单一权威余额」；**该同步属块1 change 的后续迭代，不在本 change 范围** |
| 2 | 本 change 的 delta MUST NOT 修改块1 的 delta 文件或主 specs | 硬约束 | 不同 change 独立归档；本 change 只在自己的 `specs/work-item-plan-single-candidate/spec.md` 里写 |
| 3 | 归档顺序 | 本 change 先于「块1 呈现升级」 | 否则主 specs 会同时存在「MUST NOT 展示共享余额」与「后端提供单一权威余额」两句互相打架的话 |

- [ ] 6.1 在本 change 归档前复核：块1 是否已归档、其 REQ-HGC-01 归档后的实际措辞；据此决定本 change 的 delta 是否需要在归档时补一条与块1 的衔接说明
- [ ] 6.2 归档前补独立审查结论（本 change 的 proposal / design / delta / tasks 一致性审查）

## 7. 回归、门禁与真机验收

映射 requirement：MODIFIED REQ-WSC-21（全部）

- [ ] 7.1 相关单测全绿（含行数门禁 `large_file_guard`）；隔离与纪律测试绿；受影响前端测试绿（若呈现层被动触及，MUST NOT 顺手改文案）
- [ ] 7.2 真机：**自动重写 1 次 + owner 反馈 1 次** → 门卡与后端显示的剩余额度为 **1**（不是 2、不是 5、不是 6）；这是选项 A 的核心验收
- [ ] 7.3 真机：平台自动重写**不再**受机械返修池限制 —— 机械返修额度已用尽时，只要 author 重写额度有余额，自动重写仍能进行
- [ ] 7.4 真机：编译成功后额度按「清编译链段、不清反馈段」如实递减；同一门内继续反馈不再回填满额
- [ ] 7.5 真机：自动重写用满 3 次 → 停自动重写、剩余 0、交人工门（REQ-WSC-21 既有场景不得回归）
- [ ] 7.6 真机：同一依赖环重复 → 仍提前停并升级提示（REQ-WSC-22 既有语义不得回归）
- [ ] 7.7 明确不改：REQ-HGC-01 同门 carry-forward / REQ-CG-02 预留·结算·退款、机械返修池 A 的用途、REQ-WSC-22 结构指纹、REQ-WSC-25 尝试序列、门卡文案与额度命名

## 8. 契约与留痕

- [ ] 8.1 本 change 的 `proposal.md` / `design.md` / `specs/work-item-plan-single-candidate/spec.md` / `tasks.md` 四件齐备且互相对齐
- [ ] 8.2 delta 为 **MODIFIED REQ-WSC-21**：逐字复制主 specs 全文（含 5 个既有场景），只改两处必要措辞（「复用既有机械返修通道」→「复用既有返修路由通道」；「编译成功 MUST 清零」→ 写清清零对象），「扣同一个池」表述逐字保留；新增 4 个场景
- [ ] 8.3 MUST NOT 改主 specs（`openspec/specs/**`）—— 归档时由 delta 自动同步
- [ ] 8.4 留痕：owner 选项 A 裁定（2026-11-03）与本 change 的「谁清」口径记入 `cadence/notes/`；真机额度语义验证结果记入交付说明
