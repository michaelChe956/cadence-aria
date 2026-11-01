# Proposal: coding-execution-resilience-and-product-triage

## Why

方案 v1.2（`cadence/designs/2026-09-28_方案设计_真实链缺口修复总方案_v1.2.md`）把本轮自动化定义为“自动推进＋人到点处理”。coding 侧目前在断连、双 kick、abort 后重启、人工继续、零提交重跑、返修指令消费、验证命令不可满足、reviewer 空配置、大候选返修和政策读取上，要么丢失已完成事实，要么只能靠重启服务、直写 JSON／consumed／findings／context note 脱困（BYPASS-17／18／19）。本 change 把这些断点改成“错误→通知→产品操作→原链继续”的可操作停等，并收敛三条直写旁路。

## What Changes

- **断连产物保护（#3／#8）**：provider 已完成的 source、artifact、completion checkpoint 先写既有持久面再发观察事件；attach／detach／supersede 只影响观察，不取消、不删除已完成结果；完成状态不明时停等用户确认，不自动重启。
- **最小编码互斥与接管判别（#3／#11 coding 侧）**：同一 attempt／共享工作树同时至多一个编码进程；第二次启动按既有租约三态（活跃／已死／未知）拒绝并通知、呈现“确认接管”或停等确认；同 `command_id` 重复 kick 幂等；接管后旧持有者迟到写入被拒。直接复用 C1 已落地的租约三态判定与确认接管服务。
- **abort 后可 restart（#5）**：用户明确停止后，restart 在确认无活跃编码进程时清除进程内“已退役”状态并走既有终态重开 admission；旧 run 的迟到回执不能复活旧 run。不依赖重启服务清内存。
- **人工继续后原链续跑（#4）**：`manual_continue` 等允许的人工继续提交后回到原编排阶段继续，已完成的 review 不重跑。
- **reviewer 毒默认根除（#7）**：持久化与重建严格区分 effective／provisional／enabled 三值，删除 `unwrap_or(author)` 回填；plan 会话 disabled 重启后仍 disabled；coding Code Reviewer 缺失时落“配置缺失”等待和配置／重试操作，绝不以 author 顶替或视为自动通过。
- **执行区间按事实派生（#13）**：每次新 execution 在 provider 认领前冻结真实 HEAD；零提交＝空区间；重连同一 execution 不改写 start；最终 scope 以各 execution 真实区间聚合，人工 WIP 不被倒算，越界提交仍拒。
- **返修指令单次消费（#18／BYPASS-18）**：spawn 与 rework 两路径同一口径——完整 prompt 准备→认领→消费→spawn 一次可重放完成，实际 prompt 必含新指令；中断后用户点继续／重试，旧 hash 不覆盖，禁止提前标 consumed。
- **独立验证处理／受限豁免（#19／BYPASS-19）**：在原门之外新增“进入验证处理”独立入口，绑定 finding、check、plan revision、原／替代命令、cwd、结果、测试量、环境与 scope；用户批准计划修订、可信等价证据或限域环境例外后继续。`coding_output_human_triage` 的 `retry_coding`／`abort` 与三个 Code Review 分诊门四动作保持不变；findings 不清空。
- **计划命令 vs 实际命令（#2）与文案（#9）**：产品面并列展示计划命令与 coder 实际执行命令、cwd、exit、环境，并提供“重跑原计划命令”或“进入验证处理”；#2／#9 错误文案区分“实际执行与计划不一致”“计划未声明／计划路径不可执行”，不再混为升级 Node、泛化 retry 或忽略 finding。
- **受限政策读取（#17／BYPASS-17）**：blocked／rework 页面提供“读取政策／重新授权”操作，消费 C4 resolver 产出的 authority／policy／revision／digest；返修 run 以受限 evidence 授权读取同 digest 正文；错 role、过期、错 attempt、越界仍拒绝，不向 note 写绝对路径。
- **大候选返修预算（#15）**：SC 人工返修在扣回合前计算完整候选＋固定合同＋feedback＋上下文与 provider 真实预算；超 inline 时经既有 artifact 读取或完整有序分块传输，候选不截断不摘要；不可读／缺块／超硬限在回合 CAS 前通知停等，门与预算不变。

## Capabilities

### New Capabilities

- `coding-run-operability`: coding run 的最小互斥与接管判别、断连产物保护、abort→restart、人工继续续跑、reviewer 配置三值完整性，以及对应等待项的通知／操作契约。
- `coding-verification-triage`: 计划命令与实际命令证据并列、重跑原计划命令、独立验证处理／受限豁免入口及其绑定与拒绝条件、#2／#9 错误文案口径。

### Modified Capabilities

- `coding-code-review-triage`: 增加“人工继续后回到原编排阶段且不重跑已完成 review”与“验证处理入口独立于四动作门”的边界声明；四动作集合不变。
- `coder-owned-work-item-commit`: UnitRun 提交证据区间改为逐 execution 在认领前冻结真实起点；零提交空区间与重连不改写 start。
- `coding-workspace-completion`: Group Work Item completion 的区间起点取 execution 启动事实，不再在完成时按基线回填首次 execution。
- `group-final-review-triage`: 组就绪检查按各 execution 真实区间聚合证据，人工 WIP 不归属当前 Work Item，越界提交仍拒。
- `work-item-group-coding-execution`: 增加返修指令／context note 单次消费事务（spawn 与 rework 同口径）。
- `session-policy-envelope`: 增加 blocked／rework 下的受限政策读取与返修 run 的 evidence 重新授权边界（消费 C4 resolver）。
- `work-item-plan-conversational-gate`: 修改 SC manual revision 的预算与传输口径（REQ-CG-03），以完整预算、完整有序传输和 CAS 前停等替换固定 inline 上限拒绝。

## Impact

- 影响 coding runner／socket／registry、attempt admission 与重开、UnitRun 起止提交、rework instruction／context note 消费、coding 门响应与续跑、role provider 快照、workspace session provider 持久化、SC 人工返修 prompt 构造、evidence token／mediator 与驾驶舱 inbox／通知投影及对应 REST／页面动作。
- 复用既有 CAS、attempt 文件锁、工作树锁、C1 租约三态与确认接管、C1 命令账本幂等模式、既有 gate／journal／DTO 与驾驶舱只读投影；不新增全局编排器、durable owner／fence、统一 context evolution 服务或第二套 durable operation 状态机。
- 依赖：#17 消费 C4 `logical-codebase-bootstrap-and-repair-surface` 的 authority／policy resolver 输出；租约判定复用 C1 `enrollment-recovery-surface` 已落地的判定与接管服务。
- 验收映射方案 v1.2 §6.2 的 A06、A08、A09（coding 部分）、A10（reviewer 部分）、A11、A14、A15。

## Non-Goals

- 不实现 durable runner owner、run incarnation 体系、worktree fence／epoch、双主检测、统一 context evolution、自动 retry 未知 provider 副作用。
- 不扩展或改写 `coding_output_human_triage` 两动作与 Code Review 分诊门四动作；不向原门塞 plan repair 动作。
- 不清除、改写历史 finding、旧 execution、旧 artifact、旧 hash 或旧 gate 证据；不以改 JSON／consumed／note／findings 作为恢复手段。
- 不负责 #9 的计划权限语义（C1）、LC authority resolver 本身（C4）、单仓 role-chain 预检与 pi recipe（C5／延期）。
- 不自动批准 FinalConfirm、危险命令或验证豁免；不在服务端新增通用命令执行器。
