# 独立复核：provider run 爆栈修复方案挑刺

- 日期：2026-11-06。
- 工作树：`bugfix-b-11-02`。
- 角色：独立复核，不替代主设计，不重新质疑已确诊崩溃。
- 范围：候选方案可行性、生命周期不变量、回归测试、发布回滚与行数门禁。
- 边界：仅写本报告；不改产品代码、spec delta 或 tasks；不运行 cargo、构建、测试；不访问共享验收与部署目录；不暂存报告。

## 一、结论

**有条件成立，不能按“机械拆 async fn 就必然减小 future”直接实施。** 以下四条必须修正后才能把方案交给实施者：

1. 独立 `async fn` 若仍在父 future 内直接 `.await`，子 future 仍内嵌在父状态机；需要明确间接存储边界，并量测父体、子体和构造/poll 栈，而不是按行数承诺收益。
2. “整体 `Box::pin` 单独无效、栈需求完全不变”只能改为“不能保证解决”：当前 Tokio 已自动 box 大 future，手动把 box 提到 spawn 外仍可能改变帧布局；修改后的二进制尚未测量，不能下绝对结论。
3. `.cargo/config.toml` 强制 `RUST_MIN_STACK=32MiB`，仅改 `#[tokio::test(flavor="multi_thread", worker_threads=1)]` 会继续掩盖问题；必须显式钉 2MiB worker 栈，并让真实路径在 worker 上执行。
4. 128KiB 只是工程门禁，不是安全证明：`2MiB - >1.5MiB - 2×128KiB < 256KiB`，不是诊断建议中的“>1.2MiB 余量”。真实 WS 链、debug/release 都要独立验收。

## 二、逐条挑刺与证据
### 2.1 已知事实不是待证假设

沿用诊断报告 `2026-11-06_诊断_stack_overflow_workspace_supersede_v1.0.md:5,47-74`：本次是 **debug 二进制**，崩在 `tokio::spawn`，238KiB future 尚未执行，现场 spawn 单帧 487,928B，2MiB worker 上的上游 poll 链已经消耗超过 1.5MiB。不是递归、不是空 content、不是数据环。以下挑刺只限制“候选修法能保证什么”，不推翻 gdb 根因。

### 2.2 候选方案可行性矩阵

| 候选 | 有效性与源码依据 | 代价、副作用、能否独立成立 |
|---|---|---|
| 拆臂为独立 `async fn`，直接 `.await` | **不是自动减肥机制**。`provider_run.rs:331-332` 的 drive 包含整个 match；`:1641-1651` 又展开 followups。async 状态机须保存正在 await 的子 future；拆函数不等于增加间接层。互斥臂通常按最大活跃状态布局，不应说“13 臂体积简单相加”。 | 阅读性改善成立；体积收益依赖字段存活区间、共享字段、编译器布局与内联，可能变小、持平甚至变大。独立成立的标准只能是实测父体、最大子体及真实 worker 路径均过线，不能先承诺每臂几十 KiB。 |
| 臂内 `Box::pin(arm(...)).await` | **能切断该 await 的内联状态大小**，父体保存 box 指针而非该 child 的全部状态；可不依赖 trait object。作用点应在最大 child/await 子树，不能只把小薄臂包起来。 | 分配发生在实际执行的臂，而非同时分配所有臂；但 child 仍按值构造，debug 下瞬时栈/child poll 栈没有被证明消除。`:1641-1651` 的巨大 followups 宏若仍内联，仍可能主导父体。需逐个量测，不能只测入口 box 指针。可构成主要减肥手段，但本轮未测量收益。 |
| 整体 `Box::pin` 后传给 spawn | **不能保证治愈，但不能绝对判“完全无收益”**。锁定版本为 `Cargo.lock:4763-4764` Tokio 1.52.3，其 `task/spawn.rs:179-184` 已对大 future 自动 `Box::pin`；debug/release 阈值分别 2KiB/16KiB（runtime/mod.rs:642-648）。当前崩溃恰好说明“最终堆存储”不等于“构造/搬移过程不占栈”。 | 手动 box 提到 spawn 外，spawn 的泛型 F 会从巨型 future 变成指针包装，原来约 488KB 的 spawn 帧可能改变；但可能把大帧移到 `Box::pin`/调用者。必须比较调用侧、box 构造、spawn、poll 各层，不可只看 spawn 帧。独立修复是否有效 **未验证**。 |
| `Pin<Box<dyn Future<Output=…> + Send + 'a>>` 动态派发 | 尺寸截断来自 **Box 间接存储**，不是 dyn 本身；`&mut dyn Future` 若借用栈上巨型 future 并未解决构造问题。现已有 `Arc<dyn StreamingProviderAdapter>`（`StreamingProviderInput` 邻近 trait，`streaming_provider/mod.rs:1372-1395`），不需要再为减栈建立新驱动 trait。 | 每次 poll 有虚表派发和分配；签名/lifetime/Send 更复杂。可在异构分发确有需要时使用，不应作为首选通用抽象。不能绕过按值构造与 poll 栈量测。 |
| 删除 spawn，在现有任务 `.await` 整体驱动 | 能去掉**这个 spawn 搬移帧**，但将 child 合入原有 socket/inbound future，可能加大更深 poll 链；不等于减少总栈。 | **在现有入口不成立**：`socket.rs:1644-1647,1829-1842` 逐条读取 WS，再直接 await inbound；长 run 内联会让同连接后续 Abort/新消息/lease 写停在读循环之外，supersede 无法及时进入。还会改变断线后的 run 独立存活语义。除非重做连接与驱动所有权架构，不可独立采用。 |
| 显式提高 worker 栈到 8MiB | 对“2MiB 预算不足”是直接缓解；`main.rs:1-10` 当前裸宏无自定义 builder。要保留多线程 runtime 及 I/O/time 驱动，不能改成 current_thread 来回避问题。 | **治标，可单独作为紧急版本，但不是结构修复**；未来体积增长仍可能突破新上限。影响 runtime worker 与其共用 blocking pool 的线程栈预留，不能说完全零成本或 RSS 必然不变。配置式或环境式缓解需 owner 决策，固定 2MiB 回归测试不能随生产栈一起放宽。 |

Rust 官方对 async 状态机内嵌 sub-future 的说明见[《Async Book：Recursion》](https://rust-lang.github.io/async-book/07_workarounds/04_recursion.html)；此处引用的是状态机布局原理，**不是指本缺陷为递归**。

### 2.3 “debug 下整体 Box 先构造 238KiB 临时值”必须分层表述

**本项目配置事实：**

- `Cargo.toml:53-60`：本 crate dev 只改了 debuginfo，没有设置 `opt-level`，因此未被外部覆盖时采用 Cargo dev 默认 0；依赖包明确 `opt-level=1`。`line-tables-only` 是调试信息设置，不是启用优化。
- `Cargo.toml:62-67`：release 为 `opt-level="s"`、fat LTO、单 codegen unit、strip。不能把 debug 反汇编的两个 future 大小栈槽公式直接外推到 release。
- `.cargo/config.toml:1-16` 没有 profile/rustflags 覆写，只有 jobs、栈环境与 alias；本次没有读取当前服务进程环境，**环境级 profile 覆写未验证**。
- 普通 `cargo build` 默认 dev；`--release` 才选 release，见 [Cargo Profiles 官方说明](https://doc.rust-lang.org/cargo/reference/profiles.html#default-profiles)。

**部署证据必须分两类，不能只回答“生产是 release”：**

| 被讨论的制品/实例 | 原始证据 | 能下的结论 |
|---|---|---|
| GitHub/npm 分发制品 | `.github/workflows/release.yml:55-67` 前端 build 后 `cargo build --release --locked --target …`，从 target/<triple>/release 打包 | 仓库定义的正式发布制品为 release。没有对已安装用户制品取指纹，不泛化为所有实际实例。 |
| 当前事故所涉共享验收实例 | 诊断报告 `:5` 明确 target/debug；11-05 交接记录 `:258-264` 的 BIN=target/debug/aria，重建为 `cargo build --locked --bins` | 该事故适用 debug 证据，不能以“正式发布通常 release”排除其 P0 性质。交接记录不是对当前在线进程的重新核验。 |

**应采用的结论：**

1. 当前未修改的 debug 二进制确有 243,880B future 和 487,928B spawn 帧，这是已验证事实。
2. `Box::pin<T>(x: T)` API 按值收 x，并没有“直接在堆中原位构造 async 状态机”的语言保证（[标准库文档](https://doc.rust-lang.org/std/boxed/struct.Box.html#method.pin)）。debug 下保留栈临时值具有明确风险。
3. **改写成显式整体 Box 后恰好仍需 238KiB 临时值、峰值完全不变，尚未对本项目修改后二进制测量。** “可能仍先在栈上构造”成立为风险；“一定无效”不成立为已验证结论。
4. release 优化可能消除搬移/复用槽位，也可能不能；future 布局、同步 poll 帧与 inline 决策不可只从 profile 推出。**release 是否崩溃及 Box 的 release 净收益均未验证**；不能把切 release 作为根修。

依赖精确源码：[Tokio 1.52.3 spawn](https://docs.rs/tokio/1.52.3/src/tokio/task/spawn.rs.html#173-184)、[boxing 阈值](https://docs.rs/tokio/1.52.3/src/tokio/runtime/mod.rs.html#642-648)。Tokio 自带自动 boxing 已经存在，因此本次更重要的是**在哪条栈链上构造、在哪个 await 边界存储**。

### 2.4 体积账里容易被忽略的四笔

1. **裸 future 大小 ≠ 峰值栈大小。** 128KiB 减肥门禁不能约束临时搬移、child 构造帧、child poll 同步调用链；原现场约 2×F 是此二进制的测量，不是所有新函数的 ABI 定律。
2. **堆缓冲长度不是内联类型大小。** `StreamingProviderInput` 的 prompt 是 `String`（`streaming_provider/mod.rs:919-952`），baseline_tree 是包含 PathBuf/String 的 `BaselineTreeRef`（`:912-916`），不是把整段 prompt/整棵 git tree 内联进 future。Box 一个 String 只把字符串头部转为指针，不会节省 prompt.len() 字节的 future。大型内联 record、Option/enum 最大变体以及跨 await 的子 future 才应按类型量账。
3. **现有 Box 会被解包复制。** `provider_run.rs:319-322` 对 OutlineRebuild 的 `(**rebuilt).clone()` 形成未 box 的重建上下文；其 `ResolvedPlanningContext` 有 snapshot/resolution 等内联字段（`planning_context_resolver.rs:45-63`）。这是具体应量测的存活值，不能只看到 run_kind 字段有 Box 就认为已瘦身。
4. **宏/include 只是文本分文件。** legacy 臂 `provider_run.rs:402-406` 的 include、Revision 臂 `:1005-1018` 的宏以及 followups `:1641-1651` 都仍属于同一状态机。只满足行数门禁，future 不会因此改变；followups.rs:686-992 的 reviewer/outline/draft await 仍必须纳入体积与取消范围。

128KiB 预算的正确限制：若 `S=2,097,152`，`U>1,572,864`，按旧帧的近似 `M≈2F`，则 F=131,072 时 `S-U-M<262,144B`；**U 没有上界，连这点余量是否为正都不能从现证据证明**。应把 128KiB 当减肥目标并记录真实入口的高水位/小栈执行结果；不能声称留出 >1.2MiB。


## 三、并发与生命周期风险
### 3.1 必须区分三种守卫

| 资源 | 当前实际所有权/释放点 | 拆分必须保留 |
|---|---|---|
| engine `MutexGuard` | `provider_run.rs:315` 取得；drive 内多个 `drop(engine)`（如 `:599,1451,1653`）使其按值进入 drive；`cancel_settle.inc.rs:30-42` 的 pin 块结束时 drop 未完成 drive，再在 `:56-60` 执行取消兜底 | **drive 取消范围必须拥有锁**。不得外层一直持 guard、子函数仅借 `&mut WorkspaceEngine`，然后期待 drop 子 future 解锁；不得在内部 spawn 后只 drop JoinHandle（任务会脱离继续持锁）。 |
| idle `WorkspaceProviderDriveGuard` | `:284` begin；`:314` 外层 Option；drive 以 `.take()` 使用它（如 `:1556,1654`），取消兜底本身不 take。`state.rs:63-88` Drop 递减 depth | 从源码看 idle guard 留在外层、drive 借用可变 Option；**drop drive 本身并不等同立即 drop idle guard**。正常路径在 finish_run 前 take，强制取消路径随外壳收口后释放。不能把它和 engine guard 或实体 lease 混称“drive guard”。 |
| `entity_run_lease` | `:177-195` 在 engine/session 锁之前取得；`:311-313` 移入最外层任务，覆盖 drive、取消兜底与最后 store writes；`provider_workspace_runner.rs:93-106` 定义该合同 | 不要移进单独臂/child，使 drop child 提前放开删除准入；也不要把旧 lease 搬给 supersede 新 run。新 run 独立 acquire；旧 run 未完成取消落盘期间仍保护原实体。 |

因此，上下文中“取消臂 drop drive 后 provider_drive_guard 随之释放”若指 engine 锁，是必须保留的不变量；若按字面指 `WorkspaceProviderDriveGuard`，**并非当前源码的准确生命周期**。本报告按源码把两者拆开，避免实施者以错误表述为目标改变时序。

### 3.2 高风险变更与测试抓法

| 风险点（代码证据） | 可能怎么坏 | 怎么被测试抓到 |
|---|---|---|
| 取消 settle 窗口，`cancel_settle.inc.rs:34-47` | 改成取消一到就立即 drop，跳过 provider 内置收口；或把一段启动/followups 挪到取消范围外，裸 await 永不释放锁 | 保留 r16 `provider_run_events.rs:1163-1252` 的挂起 start 测试；补“drive 在窗口内自行结束”和“窗口超时强制 drop”两条 barrier 控制分支，检查 Failed/PrepareContext/durable turn 与帧，而非只等 starts>=2。 |
| manager 先 cancel 再取 engine，`provider_run.rs:220-225` | 拆分时先 lock engine 再 supersede，旧驱动持锁时新入口永远发不出取消 | r16 测试在 1s 内必须看到第二次 provider start；新测试同时查新 token 有效、旧 start future 的 Drop 探针已经触发。 |
| `finish_run` token 隔离，`manager/runs.rs:200-218` | 子臂自行读 active token 或无 token 清理，旧 run 迟到结束把接替 run 摘除、删新 scratch | 保留 `runs.rs:916-935` 的迟到 token 不删新 scratch；在真实 run supersede 测试补断言新 token、command channel、scratch 文件仍存在；`finish_run(old)` 后新 run 可继续响应。 |
| 完成信号与实体 lease，`runs.rs:211-230`、`provider_run.rs:311-313` | 过早报告 completion，终止/删除侧误以为没有 store 写；或取消失败帧发送期间实体 lease 已放开 | 保留 `runs.rs:523` completion 真结束测试及 `:550` 不持 engine 锁等待测试；补真实 holder 实体夹具，取消收口 barrier 前删除返回 entity_busy，全部 store 收口且旧新 run 均结束后才能删除。不要用“没有实体 owner”的 fixture 测 lease。 |
| idle depth，`state.rs:79-88,1059-1072` | 拆出单臂时提前 drop idle guard，followups/reviewer 仍运行却可被 idle 回收；或遗失 guard 长期禁止回收 | 以 registry.depth 的 1→2→1→0 轨迹测双 run overlap；每阶段用 barrier，不用 sleep 猜时序。原 depth 单测只覆盖手工 begin/drop，不覆盖真实 run。 |
| `command_rx` 与 token 一致性，followups.rs:726-738,760-772,818-821 | 错误 clone/重建接收端、换 sender 不做 token 核验，使 Abort 发给旧 reviewer、同时两消费者或遗失命令 | 在 author→review→draft 每次转换后发 Abort，断言唯一当前 provider 收到且 manager 当前 token 未变；同节点重复 relay 不二次 start。 |
| relay 去重与委托让位，entry.rs:15-44、followups.rs:698-717 | 把每个臂改成独立 spawn，ReviewOnly 接力和在途 followups 同时驱动 reviewer；把 WorkItemPlan 修复 relay 也误 drain | 保留 `provider_run_events.rs:248-374` ReviewOnly 不 supersede、`:748` SC revise 仅一次 review、`:1003` SC relay 仍 supersede、`:1088-1113` Evaluate/Generate 守卫；不得以“拆后不适用”删除。 |
| WS 因果顺序，inbound.rs:619-637、provider_run.rs:371-398,1473-1493 | 锁定帧尚未排入就启动；completed 在 durable Completed 前发；将发送放入独立 task 改成乱序；旧失败帧误覆盖新 turn | 真 WS 客户端记录帧，按 session/turn/node/run 做关联，检查 locked 先于该 run 输出、durable 完成后才接收 completed、取消失败最多一次。跨独立 sender/引擎广播不存在未经证明的全局总序，断言合同的偏序而非所有帧绝对相邻。 |
| early return / 宏 return，legacy inc.rs:271-278、provider_run.rs:597-603,730-736、followups.rs:925-931 | 原 return 退出整个 drive；抽到 helper 后只退出 helper，误跑 followups/再次 start/重复 mark/finish；统一清理时又吞掉原 typed 等待或重复失败帧 | 为普通失败、typed 等待、RetryOnce、结构拒收、正常完成逐类造 barrier fixture，断言后续 provider 启动计数、turn/gate/预算和错误帧。显式设计“继续 followups/终止 drive”返回合同，不能机械把所有 `return` 改成 `Ok(())`。 |

**特别限制：** 当前 engine 锁取得发生在外层 `:315`，取消观察开始于 `:1658` 调用。不能在这次纯减肥里顺便改锁粒度、把多个持锁区分离或规范全部既有提前 return；这些都是新的并发/行为变更，应另行批准。需要移动测试时保留原断言与回归背景，不以拆文件为由缩窄覆盖。


## 四、被忽略的方案与评估
| 方案 | 可行性与关键条件 | 独立成立/建议 |
|---|---|---|
| **小 capture trampoline：新任务首次 poll 内才构造 boxed 驱动体** | 外壳只捕获必要 owned/Arc 数据；在新任务内对具名 body 做 Box::pin 并 await，把大 body 构造从 axum→socket→inbound 深栈移到新任务 poll。**若直接 `body(...).await` 而不加间接层，巨型 child 又进入外壳，方案失效**；若构造/box 放在 spawn 外，也没离开深栈。 | 值得主设计纳入的较小切口，不引入新 runtime/调度体系；仍需测外壳 captures、body 构造与 poll 峰值。可与逐臂 box/提取结合。未实测，不能单凭结构宣布成功。 |
| 先压 socket/inbound 的 >1.5MiB 同步 poll 链 | 在最重 await 子树建立 heap 间接层、缩窄 record 的跨 await 存活、避免上下文副本，理论上直接增加 caller 栈余量。已知边界是 socket.rs:1829 的 inline inbound 与 inbound.rs:632-637 的 inline spawn 入口。 | 可独立救当前崩溃，**但具体热点份额尚未测量**；只把整个 inbound Box 放在原深栈上，仍可能有构造临时值。每条消息新 spawn 若不顺序 await 会破坏 WS 串行合同；即使顺序 await，也要保证断线取消/lease 所有权。建议作为度量驱动的后续优化，不与 P0 拆臂混成大改。 |
| 每个 run 独立 OS 线程 + 大栈 + `Handle::block_on` | 只有在新线程内部才构造大 future 才脱离深 caller 栈；把已构造 future 搬进 thread closure 仍保留老问题。必须保证原 runtime 的 I/O/time 驱动活着、使用正确 Handle、线程退出/join 与 run completion 对齐。 | 可以实现，不是“改一行 spawn”；每个长流式 run 占一条 OS 线程/栈，取消只能协作，不能安全强杀线程，关停与资源治理变复杂。不推荐本次 P0。 |
| 共享专用 provider runtime（若干大栈 worker） | 把小 command/context 发给专用 executor，由它构造/运行驱动；仅在原 WS worker 上先构造巨型 F 再 `other_handle.spawn(F)`，并没消除构造风险。共享 Mutex/通道通常可跨 runtime，但 I/O/子任务/timer 的归属与 shutdown 必须逐个核对。 | 比每 run 一线程更可控，可作为大量 provider 长任务隔离的架构项目；本次没有完成跨 runtime 资源审计，不能称可无风险直接迁移。 |
| `spawn_blocking` 内 `Handle::block_on` | 直接 `spawn_blocking(async move {…})` 类型不对，它要 FnOnce；须小 closure 在 blocking 线程构造驱动，再 block_on。默认 blocking 栈不因 API 名称就更大；其池接收同一 builder.thread_stack_size。 | 已启动 blocking 工作无法 abort，可能占满池并让 runtime shutdown 无限等待（Tokio 官方合同）；当前 cancel token 仍须原样合作。不是非阻塞流式 run 的合理默认承载，不推荐。 |
| 消息循环 + 小状态机/actor | manager 现在已有 command_tx/run token，followups 逐阶段更换 command_tx；可以演进为 engine owner 接命令、分阶段驱动，不再一把 MutexGuard 跨整段 provider I/O。 | 长期可从根本上约束状态体积和锁占用，但**enum 状态机若内联最大 child 仍一样大**，还需 boxed 状态/显式堆存储；事件、durable、lease、取消与接力协议全部受影响。本次恢复既有行为不应顺手升级为 actor 架构。 |
| 仅设置 `RUST_MIN_STACK=8MiB` 的部署止血 | 作为 Rust spawn 线程的启动环境可用；[std 文档](https://doc.rust-lang.org/std/thread/index.html#stack-size)明确显式 stack_size 优先，环境值必须在进程启动前给定。 | 可零产品代码回滚式缓解，前提是 owner 允许并记录**实际启动环境与新进程**；不能在已启动进程里 set_var 期待生效。repo 的 Cargo 环境 force=32MiB 会影响 cargo run/test，直跑二进制不会自动读 .cargo/config。不等于永久根修。 |

`spawn_blocking` 限制见 [Tokio 1.52.3 官方合同](https://docs.rs/tokio/1.52.3/tokio/task/fn.spawn_blocking.html)；builder 的 thread_stack_size 被 blocking pool 采用见 [pool.rs:210-229,457-467](https://docs.rs/tokio/1.52.3/src/tokio/runtime/blocking/pool.rs.html)。提高栈预留可能主要影响虚拟地址空间，但未来实际触页会消耗物理内存，不能承诺 RSS 永远不变。


## 五、更强的回归测试设计
### 5.1 原建议两条不够，至少有六个盲区

1. **环境把小栈测试变大栈。** `.cargo/config.toml:8-13` force 32MiB `RUST_MIN_STACK`，Tokio builder 默认 `thread_stack_size=None`（1.52.3 builder.rs:305-306），会沿用 std 线程环境默认；因此只写 multi_thread/worker_threads=1 不构成 2MiB 约束。诊断“libtest 8MiB”可描述事故证据，但不能描述本仓每次 cargo test 的当前有效栈配置。
2. **multi_thread 根 future 不保证在 worker 上。** Tokio test 展开是 `Builder…build().block_on(async { 测试体 })`（[官方宏说明](https://docs.rs/tokio/1.52.3/tokio/attr.test.html)）；必须把被测真实入口放到 `rt.spawn`/服务端 connection task 中，不是在 block_on 根 future 直接 await。
3. **直接 spawn helper 不包含真实 axum/socket/inbound 深链。** 它可检查 2MiB worker 上构造，但缺失已观测的 >1.5MiB 前置消耗；修前不一定必红，不能替代真实 WS 测试。
4. **size_of 只量当前被传值的类型。** 对 `Pin<Box<_>>`/BoxFuture 测得指针大小会恒绿；必须在 boxing 前量具名 body 和每个重量 child 的实际 future，并保留 wrapper 大小作为单独指标。不同 run_kind 的同一个异构 future 类型大小不会随字符串内容变。
5. **机械提取未必保持原 size_of。** capture/参数/字段存活布局会变化，不能事先声称提取后必为 243,880B；必须让实施者记录机械提取后的红基线，再做减肥。行为等价不等于布局等价。
6. **栈溢出会 abort 整个测试进程。** JoinError、catch_unwind、Rust timeout 不会捕获 SIGABRT；把预期红用例直接塞进全量 libtest 可能丢掉其余结果。

### 5.2 推荐四层回归，前三层进入常规门禁

| 层次 | 设计与可复用证据 | 明确验收 |
|---|---|---|
| A. 体积门禁（不 poll） | 在新测试子文件中，用既有 fixture 构造**未 box 的**实际任务体、最大的各臂/followups future；测外壳、body、child，标注编译 profile/target/toolchain；注册前入口 future 也记录，避免只缩 spawned F。131,072B 可先作 body 上限，具体每臂阈值必须在主设计/实测后定，不虚造数值。 | 修前基线记录真实体积；修后每个指定对象过阈值，断言失败输出实际字节数；不能对指针包装量大小代替底层体。未来阈值上调需要解释，不随行数登记自动放宽。 |
| B. 固定小栈 worker 路径 | 普通同步 test 内手工 `Builder::new_multi_thread().worker_threads(1).thread_stack_size(2*1024*1024).enable_all()`；测试根只等待，真正调用 `spawn_provider_run_from_handler`/relay/claim 的流程在 worker task 执行。用 HangingStartProvider 与 barrier 证实已进入 body，再触发取消。不要在 tokio 测试体内嵌套 block_on runtime。 | 记录测试线程与 worker 不同；在显式 2MiB 下 Author、最大 WorkItemPlan/SC 臂、followups 可启动与取消；1s 内 supersede 接替，旧锁和新 token 不误伤。**此层修前未必能触发现场崩溃，不能声称它等价现场。** |
| C. 真实 WS 深链（关键新增） | 复用 `tests/it_core/workspace_ws_integration/part_02.rs:209-283` 的真实 TcpListener→axum→WS→StartGeneration fixture，改运行在 B 的显式 2MiB runtime 上；补 hello(driver)，以可信 Origin 完成升级；使用已被 gateway 支持的 ScriptedStreamingProvider/ClaudeCode 名，不以 unsupported 拒绝冒充 run 成功。 | 真正跨过唯一 spawn 并看到 provider-start 探针、user message/timeline/durable 进展；不能只收到 locked 就过。随后同连接发 Abort/新 StartGeneration，验证及时读帧、supersede、健康请求仍响应；覆盖 author 与最重 SC/plan 入口。此层包含真实上游 poll 链，是对 gdb 条件的主要回归。 |
| D. 制品进程 smoke（部署验收） | 独立数据副本、独立端口，分别跑 debug 和正式 release 制品，执行真实 WS StartGeneration；同一份用例后续可用于 2MiB 对照与生产缓解栈配置。已有子进程模式见 harness.rs:12506-12531 的 CARGO_BIN_EXE_aria 与独立进程组。 | 服务进程未 abort、实际 provider start/预期终态、取消后可再生成、HTTP/WS 仍活；保留二进制指纹、profile、栈配置、stderr 和 durable 前后差异。若生产代码显式8MiB，D不替代C的固定2MiB门禁。 |

**隔离形式：** B/C 的易 abort 回归建议由父测试以 `std::process::Command` 启动当前测试二进制的**精确单个子测试**，子测试内部创建显式2MiB runtime；父进程收集退出状态、stderr、超时，任何 signal/异常退出都报该用例失败。具体子测试全路径属于待实现接口，实施后才可生成精确 `--exact` 参数。本报告未创建测试/脚本。进程隔离是为了留下失败证据，不是给被测 worker 加大栈。

**关联的行为回归不可省略：** r16 `provider_run_events.rs:1163`、ReviewOnly relay `:248`、SC revise `:748`、SC author relay `:1003`、Generate/Evaluate `:1088`；manager completion `runs.rs:523,550` 与旧 token scratch `:916` 必须保留，可移动而非删除。`anchor_and_trust_cases.inc.rs:57-132` 的序号断言仅证明 adapter start 先于 split run 完成落盘，**不是 socket 接收 WS 帧顺序测试**，不能拿它覆盖全部 WS 时序。

### 5.3 对“1秒复现脚本”的验收补强

- `/tmp/aria-diag-driver.ts:1-10` 已固定 4401、session_0003，400ms 后发 start_generation；可在获准后的**可写副本**上做短反馈。源脚本与 `/tmp/aria-diag` 在本次任务中只读，不修改、不运行。
- 该脚本 `:13-14` 在 close 和 12s timeout 都 `process.exit(0)`，所以 **exit 0 不是通过证据**，而且只打印帧前200字符，不断言 provider 真开始。
- 验收者应独立检查服务 PID/退出信号、完整帧、provider-start/新 durable 状态；最快1秒能确认“不再立即 abort”，**不能在1秒内宣称完整生命周期通过**。服务启动必须指向验收者自己的可写数据副本与空闲端口，不写共享目录。


## 六、迁移、回滚与发布建议
### 6.1 推荐提交/发布粒度（建议，不执行）

| 顺序 | 独立变更单元 | 验证与回滚边界 |
|---|---|---|
| 0 | 固化红证据、增加隔离小栈/真实WS测试和机械具名构造点（若测试需要） | 先记录改写前后布局差异；若机械提取使崩溃变化，不得据“行为零变化”忽略它。这个单元不能被单独当作可部署修复。 |
| 1（owner批准才上） | **单独的栈缓解**：启动环境，或保留现 main 语义的显式 builder；不混臂拆分 | 原一秒复现可快速确认不再立即 abort，再测取消/接力；部署回滚只撤回此单元或启动配置。提前标注它没有消除大 future。 |
| 2 | 根修：明确构造/间接存储边界 + 拆臂/必要重量 child、followups 边界；不混协议/锁粒度重构 | 同一批实际红测试转绿，全部行为矩阵和2MiB真实WS通过；根修提交不得依赖“缓解已抬栈，所以测试可以放宽”。单独回退根修，仍可留缓解做业务止血。 |
| 3 | 发布 debug验收与release制品，固定二进制/配置指纹 | 先隔离副本，再小范围流量，确认启动、取消、重连与relay；连续观察无异常才扩大。未经 owner批准不重启共享实例。 |
| 4（另行批准） | 是否撤销临时大栈缓解 | 仅根修在固定2MiB路径和两种profile均通过后考虑；该提交可独立回退，不与功能变化同发。 |

每个实施提交附对应测试调整和必要变更说明，形成可独立部署/回退的文件集合；**本报告不执行 git revert、不提交任何东西**。后续 owner 可批准对“根修单元”做定向 revert，避免回退整个11-01大型业务提交。无数据/schema变更的前提下二进制回退相对简单，但仍须先有序取消/排空在途 run，不能在旧新 driver 之间交接半个运行中的内存状态。

### 6.2 配置开关与 feature flag

- **不推荐同时维护“旧巨型驱动/新拆分驱动”功能开关。** 旧路径仍然是P0，保留两份业务臂与取消合同会增加未来漂移；不是干净回滚。
- 若 owner 需要可操作缓解，**worker 栈大小配置/启动环境**比双驱动开关小得多；目前没有现成产品配置项，本次只作决策建议，不虚构已存在的 `ARIA_*` 开关。配置须在进程启动前确定并记录，显式代码 stack_size 会覆盖 RUST_MIN_STACK。
- 若已有止血版本，根修出现行为回归时回到“旧行为 + 止血栈”的已验证制品，而不是回到“旧行为 + 2MiB”的已知崩溃制品。没有先发布止血时，回滚目标是否可用必须先回答，不能把 git revert 自动等同服务恢复。

### 6.3 观察什么

优先复用现有日志，不为本次复核擅自新增遥测：

- 进程存活、exit134/SIGABRT/stack overflow、健康请求与WS能否再连。
- `handler_run_supersede`（provider_run.rs:213-216）之后是否有实际启动/输出，而非 zombie running。
- `provider_run_cancelled_dropped_drive`（cancel_settle.inc.rs:52-55）的频率/是否伴随卡锁；正常取消与兜底取消的durable终态。
- `workspace run completion wait timed out`（manager/runs.rs:284-287）、旧 token 不误删新 scratch、当前 active token 与provider启动计数。
- WS locked/输出/turn completed或failed的因果顺序、重复review/重复start、删除entity_busy与idle depth是否收敛。
- 线程数/虚拟内存/实际RSS与取消延迟。`future`字节数与栈高水位若仓内尚无运行时指标，作为发布证据采样，不声称已有监控面。


## 七、行数门禁与登记上限
`tests/it_core/large_file_guard.rs` 的合同是按**路径**而非按模块继承：

- `:4-6`：src/tests/web/src 内 rs/ts/tsx 默认 **1200行**，包括 `.inc.rs`（其extension仍为rs）。
- `:265`：provider_run.rs 祖父 cap **1678**；当前1676，只有2行余量，不能先额外塞一大段helper再说后续拆。
- `:284-287`：provider_run_events.rs 祖父 cap **1257**；本次读到该文件恰1257行，不能继续直接追加体积/worker测试。
- `:361-366`：未登记路径一律回落1200；拆出来的新文件**不会继承旧文件1678/1257的cap**。`:388-401` 使用 `content.lines().count()`，空行/注释也计；不是只数有效Rust语句。
- `:344-355`：登记项必须严格排序、唯一且cap>1200。若旧文件拆到≤1200，推荐同提交删除其旧登记，不给将来重新长回1678留暗门；不得用新增祖父登记给新文件开后门。

主设计的新文件布局尚未在本报告写作时落盘，本报告不重复其切分/精确行数预估。独立复核建议是：新重量臂文件以**≤1000行目标、1200硬线**留注释/导入/测试维护余地；原620行左右SC manual revision臂（provider_run.rs:1020-1639）加签名一般有机会容纳，但这是行数预算推算，**最终新文件行数未验证**；followups.rs:686-992的宏和既有legacy inc不能因原文件变短而不计体积。

测试组织可沿用现有 `provider_run_events.rs:1116-1118` 的path子模块/include惯例：新建独立子文件放体积/小栈用例；若需要把r16测试与HangingStartProvider一起移动，保留原测试名称、断言与注释，原文件只新增必要模块声明并确保本身不增长超cap。不要塞进已达14835行的live harness（其登记cap14966，`:296-300`）只为复用几行启动逻辑；应复用/抽取最小夹具，避免制造新的祖父债。


## 八、需要 owner 拍板的决策项
| 决策 | 选项与代价 | 我的推荐 |
|---|---|---|
| P0是否一起止血 | A只做根修，少运行时配置变化，但在根修验收期间保留事故窗口；B先独立抬栈/启动环境缓解，再根修，增加一次部署与回退管理 | 若必须尽快恢复验收/业务，选B，且单独单元；不能把它宣布为永久修复。是否修改main或仅部署环境由owner决定。 |
| 128KiB如何使用 | A作为安全证明（不成立）；B作为裸future工程上限，加固定2MiB真实WS回归和构造/poll证据 | 选B；owner确认是否采用更保守的逐臂阈值，须由实施实测填充，不由行数推导。 |
| 正式验收profile | A只跑release；B保持事故debug入口并额外测release | 选B：发布流水线确为release，但事故运行实例确为debug，缺一条都不能覆盖交付。 |
| 是否本次压inbound/改actor/专用runtime | A纳入P0导致触碰连接串行、锁与关停架构；B仅根修必要边界，长期优化另立项 | 选B；若根修度量表明父体仍大，再定向批准必要inbound边界，不自动扩大。 |
| 回滚方式 | A运行时双驱动flag；B独立提交/制品回退，保留可单撤的栈缓解 | 选B，避免双实现。回退目标必须是已验证不立即爆栈的制品。 |


## 九、接地、验证与未验证项

**状态：DONE_WITH_CONCERNS（独立review-verdict）。** 已完成本任务要求的可行性、并发、遗漏方案、测试、回滚与门禁复核；不是对修复实现的通过结论。

- 实际接地：已读gdb诊断全文；定点核对provider_run外壳/各风险臂与尾部、cancel_settle、legacy inc、followups宏、entry去重、manager token/completion/scratch、registry guard、真实WS/子进程夹具、Cargo与发布workflow。源码证据行号均指本工作树本次读取内容。
- 工具：先用CodeGraph；`ast-grep outline`执行返回“outline is not supported by the built-in ast-grep”（exit2），已报告工具问题，之后降级定向行号读取；发现/枚举交由两条只读scout证据面，再由本复核定点查实关键profile、测试、生命周期片段。外部库API核对了Context7及锁定Tokio 1.52.3源码。
- 本次**没有**运行cargo、构建、测试、复现服务、objdump或gdb；没有访问/改动共享验收或部署目录；`/tmp/aria-diag-driver.ts`仅读取，没有执行。未改任何src/spec/tasks，没有git add/commit。
- 未验证：拆后实际future大小、每个Box改法的debug/release构造栈、release是否崩溃、真实WS固定2MiB测试结果、修改后guard捕获/drop布局、最终新文件行数。本文把这些都列为实施/验收证据要求，未用估算冒充实测。
- 主设计当时尚未成稿，本文复核的是给定候选及实际源码合同，不虚构已审阅最终主设计。提交实施前须确认主设计已吸收本报告四条硬修正及守卫生命周期区分。
