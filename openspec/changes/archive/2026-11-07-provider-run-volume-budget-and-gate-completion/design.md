# Design: provider run 体积预算与四层门禁 —— 收尾未完成项

> 本设计与 `proposal.md` 配套：**只冻结口径与切法，不在第一阶段实施**。

## 1. 体积预算的两级线与推导

| 线 | 值 | 性质 | 依据 |
|---|---|---|---|
**硬线** | 131,072 B（128 KiB） | **阻断**：超过即视为回归，A 层断言 | tokio worker 默认栈 2 MiB；spawn 搬运需 ≈2×sizeof(F)；`2 MiB − >1.5 MiB(inbound 链) = <512 KiB` ⇒ 单个对象 >131,072 B 即与搬运开销叠加越界 |
**优先线** | 65,536 B（64 KiB） | **收敛目标**，逐对象冻结 | 让「搬运开销 + 对象」在余量内留出可观的裕度；**不是安全证明**（诊断期曾把余量算成「>1.2 MiB」，实为 **<256 KiB** ⇒ 128 KiB 只能当工程门禁） |
**外壳** | 32,768 B（32 KiB） | 硬线 | 外壳在**深 inbound 栈**上构造 ⇒ 必须小到不可能与其他帧叠加越界 |

⇒ **禁止**把所有对象机械放宽到 128 KiB；逐对象实测后冻结更小阈值；上调须有新栈账 + owner 裁决。

## 2. 为什么「入口小外壳 + 首次 poll 内装箱」是根治

```
async fn ⇒ 状态机：所有跨越 .await 的局部量入状态机；
且被 await 的子 future【内联】进父 future（不单独分配）
⇒ 体积沿调用树累加 ⇒ 1360 行单块 ⇒ 238 KiB（实测 235,088 B）
```
⇒ 无论怎么拆，只要**构造点仍在深 inbound 栈上**，大对象就会压垮栈。
⇒ **构造点必须移出深栈**：入口只构造小 capture 外壳 ⇒ 首次 poll（已在 worker 上）内 `Box::pin(body)` ⇒ 构造落堆。
⇒ 这是第一阶段的核心改动，也是「剩余族拆分只属优化」的根据。

## 3. 剩余族的切法（逐族）与守恒点

| 族 | 现状 | 切法 | 守恒点（**最易出错**） |
|---|---|---|---|
plan revision | 宏 `workspace_ws_work_item_plan_revision_arm!` | 具名 async 臂 + `Box::pin` 调用点 | 宏内 `return`→`Stop`；穿出 loop→`Continue`；原 `break` 只退相应 loop；循环上限与 fresh 冻结 payload 消费不变；**不重算反馈归一化、不新增 retry** |
draft / batch | 匿名块内 | 两个具名臂 | `Option<Option<String>>` 保留（fresh 冻结为空 ≠ 非 fresh）；RetryOnce 每轮**新 split handle**、不新建 generation/batch；**`AwaitConfirmation`(feedback=None 续 loop) ≠ draft 的 `break`**；「mark 后即 return」→`Stop`，不补通用 finish/错误帧 |
SC manual | 620 行单块（**第一阶段最大未拆族**） | 具名臂 | 同 turn ≤2 次真实 start、claim 一次、attempt2 用冻结 prompt；每次 start 的 ledger ↔ 实际探针对应；二次拒收/重放不新 claim、不第三次 start；两个 barrier 处取消都 durable Failed(cancelled)、回显 ≤1 |
followups | 宏 + 匿名块 | 具名 `drive_provider_run_followups` | **不接 entity lease、不复制已消费 receiver**；manager `replace_command_tx_if_token` 原位保留，新 Rx 仅一个消费者；reviewer→outline→draft 接力在 barrier 发 Abort 时**唯一当前 provider 收到**、token 未被偷换；SC Generate 让位而 Evaluate 驱动；旧 finish 不误删新 scratch；正常尾部只一份 |

## 4. 四层门禁的分工与「谁证明什么」

| 层 | 证明什么 | 形态 | 状态 |
|---|---|---|---|
**A** 体积 | 每个未装箱对象 ≤ 预算 | 静态断言（`size_of_val` 于 raw future）+ S0 基线对照 | 第一阶段已交付（硬线） |
**B** 小栈 + 生命周期 | 代码在 **2 MiB** worker 上真能跑；三条守卫与 token/scratch/新旧 run 隔离成立 | 显式 `thread_stack_size(2 MiB)` + `rt.spawn` + 线程 identity + barrier | 第一阶段已交付 |
**C** 真实 WS | 真实 axum→socket→inbound 深链 + 可信 Origin/Hello + 真实 provider start + 完整帧 + durable + 取消后再生成 | 集成测试（自有 tempdir/端口/进程组） | 第一阶段已交付 |
**D** 制品 | 真实 `aria` 二进制在真实 profile 下的同一合同 | 子进程（`CARGO_BIN_EXE_aria`）+ 指纹/profile/生效栈 | 第一阶段已交付（**release 档位与 prod 16 MiB 属后续项**） |

⇒ **C 层的 2 MiB 永不豁免**（任何运行时档位都不得放宽它）。

## 5. 工具面与其盲区（必须诚实登记）

| 工具 | 能测 | **不能**测 |
|---|---|---|
`size_of_val(&raw_future)` | 未装箱对象的真实字节 | Box/Pin 后的指针（8~16 B ⇒ **恒绿 ⇒ 假门禁**） |
clippy `large_futures` | 未装箱 future 体积（**须先核实其测量对象**） | 运行时栈峰值；Box 盲点（rust-clippy discussion #16702：它只建议 `Box::pin`） |
显式 2 MiB worker | 该线程栈下的真实可运行性 | 构造期栈峰值（除非采样）、其它线程的占用叠加 |
tokio `BOX_FUTURE_THRESHOLD`（dev 2048 / release 16384） | 哪些 future 会被 tokio 自动装箱 | **自动装箱发生在 future 已在栈上构造之后 ⇒ 消不掉构造期临时值** |

⇒ **禁止**把任何一项的通过结论外推到另一项；**debug 数字不得冒充 release**。

## 6. 风险与回退

| 风险 | 缓解 |
|---|---|
拆分过程中破坏三守卫 ⇒ 死锁（r16 缺陷#10 家族） | 每族拆分**同提交**保留三守卫注释与合同；B 层生命周期矩阵作为门禁逐族验证 |
`Box::pin` 改变了 drop/取消语义 | 只在**被选中的臂**处装箱；`Stop` 路径不得多跑 followups/不得重复 finish；既有断言全保留 |
回退到旧制品 ⇒ 又崩（无 S1 止血） | 回退候选必须**先按实际启动栈验证 D**；无合格候选 ⇒ NOT_READY，不以 `git revert` 等同服务恢复 |
prod 误用 debug 制品（8 MiB 而非 16 MiB） | 启动日志打印 profile + 生效栈；D 层记录指纹 ⇒ 误用可检出；prod 只跑 release |

## 7. 本 design 与第一阶段交付的关系

第一阶段（当前）已交付 A/B/C/D 四层骨架与 6/13 族拆分；本 design 冻结**剩余族切法**、**预算推导**、**工具盲区**与**发布/回退合同**，供后续会话直接派工，**不需要重新推导**。
