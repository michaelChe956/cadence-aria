# 调研报告：Rust 异步大 future 栈溢出与栈取值（v1.0）

| 项 | 内容 |
|---|---|
| 日期 | 2026-11-06 |
| 用途 | 为「tokio worker 线程上 spawn 大 future 爆栈（exit 134/SIGABRT）」修复技术方案提供有出处的社区/上游依据 |
| 本项目实测参数（代入值） | future 实测 **243,880 B**（dev 构建）；`tokio::spawn` 搬运需 ≈**2×sizeof ≈ 488 KB** 连续栈；上层 axum→socket→inbound poll 链已耗 **>1.5 MiB**；tokio **1.52.3**（Cargo.toml 声明 `1.52.1`，`^` 语义下 1.52.3 相容）；`#[tokio::main]` 裸写（无 worker_threads/thread_stack_size）；toolchain = `stable`（未锁小版本，edition 2024 ⇒ rustc ≥ 1.85） |
| 证据分级说明 | 每条结论标注：**【有出处】**（一手 URL）/ **【社区共识】**（多来源相互印证）/ **【推断】**（依据链明确但无人直说）/ **【未找到】** |

---

## 0. 摘要（TL;DR）

1. **复核方说法属实但需限定**：tokio 1.52.3 的 `spawn` 确实会自动装箱大 future（源码逐行核实），阈值 `BOX_FUTURE_THRESHOLD` = **dev 构建 2048 B / release 构建 16384 B**。但装箱发生在 `spawn` **内部**、future 已在**调用点栈上构造完成之后**——它只消除 task 堆分配阶段的二次拷贝，**不能消除调用点构造 ≈1–2×sizeof(future) 的栈需求**。本项目 243,880 B future 在 >1.5 MiB 深栈上爆栈，与该行为完全自洽。【有出处】
2. 社区/上游的标准修法是**组合拳**：把巨型 async 块拆成小的 async fn，并在**浅栈处以 `Box::pin(call()).await` 做边界**（有实测数据：main 栈占用 280K→90K）；参数瘦身/不跨 await 的局部收窄；必要时**提高 worker 线程栈**。【社区共识】
3. 栈取值：std spawn 线程默认 **2 MiB**（Tier-1 全平台官方原文），主线程不由 Rust 决定（Linux 通常 8 MiB）；`#[tokio::main]` **不能**配置 `thread_stack_size`，必须手动 `Builder`。社区真实案例（Linux）：**2 MiB、4 MiB 均崩，8 MiB 存活**；8 MiB 是多个独立案例收敛的取值。【有出处】
4. **dev 崩不代表 release 不崩，反之亦然**：两个方向都有真实事故（有 dev-only 崩、也有 release-only 崩）。体积随 opt-level、panic 策略、内联/LTO、rustc 小版本漂移。唯一可靠判据是**双 profile 实测 `size_of_val` + clippy `large_futures` 门禁 + 子进程崩溃回归测试**。【社区共识】

---

## 1. 主题 1：async future 太大导致栈溢出——机制与既有解法

### 1.1 机制：future 体积为何膨胀（官方依据）

- 每个 `.await` 会把被调 future 的状态**合并进**调用者的状态机枚举；跨 await 存活的局部变量与 async fn 的**参数捕获**全部进入 future 类型。深层嵌套时可雪球式膨胀为"整个程序的状态"，且最坏**指数增长**——Rust 官方 2026 项目目标页给出 playground：64 KiB 缓冲嵌套 4 层后 future ≥ 1,000,000 B。【有出处】
  - Rust Project Goals 2026「Async Future Memory Optimisation」（承认"unexpected or even unexplainable stack-overflow that is hard to debug"，指数膨胀跟踪 issue rust-lang/rust#62958，修复方案 rust-lang/rust#135527 与 nightly `-Zpack-coroutine-layout`）：https://goals.rust-lang.org/2026/async-future-memory-optimisation.html
- kornel（Rust 社区资深成员）对状态合并机制的通俗完整解释：https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/8 【有出处】
- Microsoft 官方 Rust 编码指南 M-ASYNC-STACK-SIZE：讲清"跨 await 的局部/参数进入 future 类型、只在两个 await 之间存活的仍在真栈上"，并给出热路径监控/瘦身指引：https://github.com/microsoft/rust-guidelines/blob/main/src/guidelines/performance/M-ASYNC-STACK-SIZE.md 【有出处】
- 深度技术分析（`-Zprint-type-sizes` 逐变体拆解 + 汇编级证据 + 嵌套泛型链每层保留参数副本导致体积按层数放大 + `Box::pin` 后编译器仍预留 ~2× 缓冲的观察）：Swatinem「The size of Rust Futures」https://swatinem.de/blog/future-size/ 【有出处】
- 关联的 rust-lang 跟踪 issue（来自上文的引文）：内存用量 #69826、**参数跨 yield 点重复 #62958**、低效 codegen/memcpy #99504。【有出处】

### 1.2 `tokio::spawn` 的栈语义与「自动装箱」核实（本节为本次检索的核心核实项）

**结论：tokio 1.52.3 存在"大 future 自动装箱"，复核方说法属实；但它救不了本项目这种"在深栈上构造大 future"的场景。**【有出处，源码逐行核实】

1.52.3 tag 源码 `tokio/src/task/spawn.rs`（原文）：

```rust
pub fn spawn<F>(future: F) -> JoinHandle<F::Output> ... {
    let fut_size = std::mem::size_of::<F>();
    if fut_size > BOX_FUTURE_THRESHOLD {
        spawn_inner(Box::pin(future), SpawnMeta::new_unnamed(fut_size))
    } else {
        spawn_inner(future, SpawnMeta::new_unnamed(fut_size))
    }
}
```

阈值定义在 `tokio/src/runtime/mod.rs:643`（1.52.3 tag）：

```rust
pub(crate) const BOX_FUTURE_THRESHOLD: usize = if cfg!(debug_assertions) {
    2048        // dev（debug_assertions 开）
} else {
    16384      // release
};
```

- 源码直读：https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/task/spawn.rs 、https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/mod.rs 【有出处】
- 历史脉络：早期（≤1.32 时代）装箱只在 debug 构建生效，release 直接走栈导致 **release-only 爆栈**（tokio#6057，2023-10 报告：`Box::pin(future())` 存活、裸 `spawn(future())` 仅 release 崩）；mhils（mitmproxy）提 PR #6826 于 2024-09 修复，把装箱改为**无条件按尺寸触发**，issue 评论给出 1.40.0 崩 / 1.41.0 存活的对照。#6826 同时把 `spawn_blocking` 的装箱同样改为无条件。
  - https://github.com/tokio-rs/tokio/issues/6057
  - https://github.com/tokio-rs/tokio/pull/6826
- **为什么自动装箱仍爆栈**：`Box::pin(future)` 在 `spawn` 内部执行，而实参 `async move {…}` 块作为表达式**先在调用方栈帧上构造**（dev 下未内联、大对象经栈临时量传递/移动）。装箱消除的是 task 堆分配（Harness）阶段对 future 的再一次整尺寸栈拷贝，调用点 1–2×sizeof(F) 的构造/搬运仍在。佐证：Swatinem 实测 `Box::pin` 后编译器仍预留 ~2× 缓冲的栈（https://swatinem.de/blog/future-size/ ）；kornel："Rust is not very smart with Box…at this point it's too late to move a too-large stack object onto the heap. **Box it while it's not too large yet**"（https://users.rust-lang.org/t/causes-for-thread-tokio-runtime-worker-has-overflowed-its-stack/117224/8 ）。本项目实测 ≈2×sizeof ≈ 488 KB 与此一致。【社区共识 + 本项目实测】
- 顺带澄清一个常见误区：`std::boxed::Box::new` 大数据本身就可能"先栈构再 memcpy 进堆"，这是 2018 年至今开放的 rust-lang/rust#53827（含 release 下的复现、`vec![..].into_boxed_slice()` / `Box::new_uninit` / `bytemuck::zeroed_box` 等官方 workaround，以及 nagisa 的定性："优化永远不是此问题的修复，因为不保证发生"）：https://github.com/rust-lang/rust/issues/53827 。Stack Overflow 27 赞回答（godbolt 证据：非零初值大数组即使 release 也整块栈构再 memcpy；debug 模式必拷）：https://stackoverflow.com/questions/77934697/will-boxnew-make-a-copy-from-stack-to-heap 【有出处】

### 1.3 惯用解法清单（按社区使用频率排序）

| # | 解法 | 要点/坑 | 依据 |
|---|---|---|---|
| 1 | **拆分 + 边界装箱 `Box::pin(call()).await`** | 把大 async 块提为独立 async fn，在**多个调用边界**装箱，使外层 future 缩小。注意：直接 `call().await`（不装箱）会把被调状态机**并入**外层，拆函数本身不减体积；装箱要"趁小装"。 | kornel 多帖；rsland 实证"在多处 Box::pin 后 2 MiB 就够"：https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/4 ；Osterman 实测 main 栈 280K→90K：https://users.rust-lang.org/t/stack-overflow-in-async-main-function-due-to-excessive-stack-allocation-not-from-recursion/109571 【社区共识】 |
| 2 | **在浅栈处构造**：`tokio::spawn(async move { Box::pin(inner(args)).await })` | 让巨型块的实际构造发生在新任务的浅栈（而非 axum→hyper 深链上）；spawn 的自动装箱处理剩余搬运。这是 #1 与 1.2 节机制的组合应用。 | 【推断】依据链：spawn 自装砠除二次拷贝（1.52.3 源码）+ kornel "too late to box" 论述 + tokio 文档"spawn 保证不同步 poll"（spawn.rs 文档注释） |
| 3 | **参数/局部瘦身** | 大参数改传引用 `&(A,B)` / `&mut Option<T>` + `.take()`；把只在那两个 await 之间用的大局部收窄作用域 `{ let big; … }`；大数组装 `Vec`/`Box`。返回 `impl Future` 并把非 async 的准备工作提出 async 块外（避免参数被捕获进 future）。 | Swatinem（引用节）；kornel：https://users.rust-lang.org/t/stack-overflow-in-async-main...109571/5 ；MS 指南 M-ASYNC-STACK-SIZE（`Either`/`.then()` 组合示例）【有出处+社区共识】 |
| 4 | **提高线程栈**（`Builder::thread_stack_size` / `RUST_MIN_STACK` / 专用 std 线程 `block_on`） | 简单兜底，社区有真实取值数据（见主题 2）。 | cisaacson 案例（8 MiB）；steffahn 推荐（std `Builder::stack_size` 或 stacker）：https://users.rust-lang.org/t/stack-overflow-in-async-main...109571/3 【有出处】 |
| 5 | **`spawn_blocking` 换线程模型** | ⚠️ 澄清：tokio blocking 线程**默认同样是 std 2 MiB**，仅当配置了 `thread_stack_size` 时才应用（1.52.3 `blocking/pool.rs` 只有 `if let Some(stack_size)` 才调 `builder.stack_size`）。换 `spawn_blocking` **不会自动获得大栈**。 | 源码：https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/blocking/pool.rs （L465–467）【有出处】 |
| 6 | **`stacker` crate 动态扩栈** | `stacker::maybe_grow(red_zone, stack_size, \|\| { … })`：剩余栈不足时切换到堆上新栈；`grow` 无条件换栈；`remaining_stack()` 可查询。文档示例参数 `maybe_grow(32*1024, 1024*1024, …)`。steffahn 在论坛作为通用栈不足方案推荐（"对递归尤其有用"）。⚠️ 文档无 tokio/async 专项示例，用于 poll 前扩栈属组合用法。 | https://docs.rs/stacker/latest/stacker/ ；https://users.rust-lang.org/t/stack-overflow-in-async-main...109571/3 【有出处（用法组合为推断）】 |
| 7 | 修 `select!`/`pin_mut!` 栈固定 | `pin_mut!`（及 futures `select!`）把 future **钉在栈上**；应改 `Box::pin` 钉堆。wg-async 官方"status quo story"（源自 Krustlet 真实事故）整篇讲这个坑。 | https://rust-lang.github.io/wg-async/vision/submitted_stories/status_quo/alan_runs_into_stack_trouble.html （原始博客：https://deislabs.io/posts/a-heaping-helping-of-stacks/ ）【有出处】 |

### 1.4 诊断手段

| 手段 | 说明 | 出处 |
|---|---|---|
| `std::mem::size_of_val(&fut)` | 运行时测 future 体积的惯用写法；**微软指南直接给出"体积断言测试"模板**（`#[test] fn has_reasonable_size() { assert!(size_of_val(&f) < LIMIT) }`），可进 CI | https://github.com/microsoft/rust-guidelines/blob/main/src/guidelines/performance/M-ASYNC-STACK-SIZE.md 【有出处】 |
| `cargo +nightly rustc -- -Zprint-type-sizes` | 打印每个类型（含编译器生成的 future/closure）尺寸与逐变体字段；**nightly 专属不稳定 flag**（Unstable Book 收录："no tracking issue…internal to the compiler"）。变体含义（`Unresumed`/`SuspendN`/`__awaitee`/upvar）的解读指南见 kpreid 的帖子 | https://doc.rust-lang.org/unstable-book/compiler-flags/print-type-sizes.html ；用法与解读：https://users.rust-lang.org/t/stack-overflow-in-async-main...109571/7 【有出处】 |
| clippy `large_futures` | **默认阈值 `future-size-threshold` = 16384 B**，可在 clippy.toml 调低。kornel 在多帖推荐用它在装箱后"找出还剩哪些大 future"。相邻 lint：`large_stack_arrays`、`large_stack_frames`（后者偏保守，shaw 案例中它标出 "2639056 bytes on the stack" 定位成功） | https://rust-lang.github.io/rust-clippy/master/index.html#large_futures ；https://doc.rust-lang.org/clippy/lint_configuration.html#future-size-threshold ；案例：https://users.rust-lang.org/t/stack-overflow-in-debug-even-after-stack-increase-ok-in-release/125212/7 【有出处】 |
| `cargo-call-stack` | 全程序静态栈用量分析；**README 自述主要面向 no_std 嵌入式**（std 程序大量间接调用/潜在递归使其"of very limited use"），且依赖 nightly `-Z emit-stack-sizes`、要求 fat LTO。对本项目（tokio+axum）参考价值有限 | https://github.com/japaric/cargo-call-stack 【有出处】 |
| 逐步降栈复现 | 手动把 `thread_stack_size` 降到稳定复现的值再定位（drewtato 建议） | https://users.rust-lang.org/t/causes-for-thread-tokio-runtime-worker-has-overflowed-its-stack/117224/2 【有出处】 |

---

## 2. 主题 2：栈大小取值——默认值、配置途径与推荐值

### 2.1 各类线程的默认栈（官方原文）

- **std spawn 的线程：2 MiB**。std::thread 模块文档「Stack size」原文："The default stack size is platform-dependent and subject to change. Currently, it is **2 MiB on all Tier-1 platforms**." https://doc.rust-lang.org/std/thread/#stack-size 【有出处】
- **主线程：不由 Rust 决定**（同页原文："the stack size of the main thread is *not* determined by Rust"）。Linux 下通常为 ulimit 的 8 MiB——经典 SO 问答（19 赞/33 赞采纳）以实测解释"cargo test 崩、cargo bench 不崩"：test 每个用例跑在**新线程（2 MiB）**，bench 跑在主线程（约 8 MiB）。https://stackoverflow.com/questions/42955243/cargo-test-release-causes-a-stack-overflow-why-doesnt-cargo-bench 【有出处】
- 推论（与本项目直接相关）：`#[tokio::main]` 的 async main 体经 `block_on` 在**主线程**上 poll（宏展开等价于手动 builder + block_on，见 attr.main 文档展开示例），而 **axum/hyper 的连接处理是 tokio 任务，跑在 worker 线程上**——这正是"事故发生在 worker（2 MiB）而非 main（约 8 MiB）"的结构性原因。【推断】依据链：attr.main 展开示例 https://docs.rs/tokio/1.52.3/tokio/attr.main.html + 多线程调度器 work-stealing 模型 https://docs.rs/tokio/1.52.3/tokio/runtime/index.html + tokio::spawn 文档中"accept 循环 + 每连接 spawn 一个任务"的官方示例 https://docs.rs/tokio/1.52.3/tokio/task/fn.spawn.html
- **hyper/axum 无"每连接一线程"**：连接 = 调度到 work-stealing 线程池的任务，handler、hyper 状态机、socket I/O 的 poll 全部叠加在 worker 栈上（官方 runtime 文档："The multi-thread scheduler executes futures on a thread pool, using a work-stealing strategy. By default, it will start a worker thread for each CPU core"）。https://docs.rs/tokio/1.52.3/tokio/runtime/index.html 【有出处】

### 2.2 `RUST_MIN_STACK` 的确切语义（官方原文，设计可直接引用）

来自 https://doc.rust-lang.org/std/thread/#stack-size ：

- 取值为**字节数的整数**；
- 只影响**未显式设置栈大小的 spawn 线程**；显式 `Builder::stack_size` **覆盖**它（"setting Builder::stack_size will override this"）；
- ⚠️ **"changes to RUST_MIN_STACK may be ignored after program start"**——必须在进程启动前注入环境变量，不能指望运行中 `env::set_var` 生效；
- 不影响主线程。
- 作用范围推论：tokio 的 worker/blocking 线程在未配置 `thread_stack_size` 时以 `std::thread::Builder` 默认值创建（1.52.3 builder.rs：`thread_stack_size: None`，注释 "Do not set a stack size by default"），故**未配置时 `RUST_MIN_STACK` 对它们生效；一旦显式配置 `thread_stack_size` 则被覆盖**。【推断】依据链：std 文档 + tokio builder.rs 默认值 + blocking/pool.rs 仅在 `Some` 时调用 `.stack_size()`。【未找到】tokio 官方文档没有一句话直接承诺"RUST_MIN_STACK 会作用于 tokio worker"。

### 2.3 tokio 的栈配置途径

| 途径 | 说明 | 出处 |
|---|---|---|
| `Runtime Builder::thread_stack_size(val)` | 值为**字节**；字段注释原文 "Stack size used for **threads spawned by the runtime**"；**默认 None ⇒ 继承 std 2 MiB**。源码证实 blocking 线程消费该值（`if let Some(stack_size) = … builder.stack_size(stack_size)`）；作用含 worker 与 blocking 线程 | 源码：https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/builder.rs （字段 L83-84、默认 L305-306、setter L670）；https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/runtime/blocking/pool.rs ；文档页：https://docs.rs/tokio/1.52.3/tokio/runtime/struct.Builder.html#method.thread_stack_size 【有出处】 |
| `#[tokio::main]` 宏 | **不提供 thread_stack_size 选项**（attr.main 文档全部可配置项中无任何 stack 相关项；仅 flavor/worker_threads 等）。要配栈必须放弃宏、手动 builder（文档同时给出宏的等价展开代码） | https://docs.rs/tokio/1.52.3/tokio/attr.main.html 【有出处】 |
| `RUST_MIN_STACK` | 见 2.2；作为不改代码的部署侧兜底（如给 CI/dev 环境） | https://doc.rust-lang.org/std/thread/#stack-size 【有出处】 |

### 2.4 取值建议与代价

**社区真实取值数据（Linux x86_64 服务端）：**

- 案例A（商业服务器，54K 行代码生成源码触发）：默认 2 MiB **崩**；4 MiB **崩**；**8 MiB Linux/Mac 均存活**；最终代码 `thread_stack_size(1024 * 8000)`（≈7.8 MiB）+ `rt.block_on(Box::pin(inner_start_server(addr)))`。同帖还显示 Mac 与 Linux 崩点不同（Mac 降到 ~450 KiB 才崩）——**跨平台崩溃点不可移植**。https://users.rust-lang.org/t/causes-for-thread-tokio-runtime-worker-has-overflowed-its-stack/117224 【有出处】
- 本项目账目：>1.5 MiB 链上占用 + ≈0.5 MiB spawn 搬运 ≈ 2 MiB 已顶满默认栈；再计 release/编译器版本漂移余量（见主题 3，实测崩溃点漂移 100 倍的案例存在）⇒ **8 MiB 是有实测支撑的收敛值；4 MiB 有明确失败案例，不建议**。【推断，依据如上】
- 代价：`thread_stack_size` 按线程数线性预留**虚拟地址空间**（N workers × 8 MiB），Linux 线程栈按需触页，未触及页不计 RSS——此为 Linux/glibc 线程栈的通用语义。【社区共识】（通用 OS 手册层面依据未逐字核对：proc(5)/pthread_create(3)，https://man7.org/linux/man-pages/man5/proc.5.html ）
- 另一条社区路径：不调栈，**靠 `Box::pin` 多处装箱把需求压回 2 MiB 内**（rsland 案例"在多处 Box::pin 后 2 MiB 够用"；mitmproxy 全量钉 `Box::pin` 修复 release 崩溃）。设计上可作为"调栈"与"拆分装箱"的对照方案。https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/4 ；https://github.com/tokio-rs/tokio/issues/6057#issuecomment-2331098720 【有出处】

---

## 3. 主题 3：dev 与 production 的差异处理

### 3.1 差异的四个来源（都有实证）

1. **opt-level**：dev 默认 0（本项目 `[profile.dev.package."*"]` 给依赖 opt-level=1，但**本地 crate 仍为 0**——1360 行 async 块正是本地代码）。scottmcm："opt-level = 0 is telling the compiler to *not even try*"（LLVM 连 memcpyopt 都不做）；**三个独立案例中 `[profile.dev] opt-level=1` 即消除爆栈**。https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/10 、https://users.rust-lang.org/t/stack-overflow-in-debug-even-after-stack-increase-ok-in-release/125212/3 【有出处】
2. **debug_assertions**：改变 tokio 自动装箱阈值（dev 2048 / release 16384，见 1.2）。dev 下更多 future 会在 spawn 内被装箱（仍不救调用点构造）。【有出处（源码）】
3. **panic 策略 + 溢出检查**：bjorn3（rustc 贡献者）解释——`panic=unwind` 下为保证"分配失败可恢复"语义，大结构**先在栈上构造再搬入 Box**；`panic=abort` 允许堆上原地构造。同一程序 unwind+1 GiB 栈不够、abort+2 MiB 够。本项目两个 profile 均未设 panic ⇒ 默认 unwind ⇒ 双倍搬运语义在 release 同样存在。https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/5 【有出处】
4. **rustc 版本 / 依赖微调**（体积漂移，两方向都发生过）：
   - rustc 1.66→1.67 升级即触发爆栈（#107358，多人复现，有人被迫钉 1.66.1；最终靠 `RUST_MIN_STACK` 解决）；https://github.com/rust-lang/rust/issues/107358
   - 同帖某人：`#[tracing::instrument]` 注解使 future 超限，去掉即好（**tracing 跨 await 捕获会放大 future**——本项目 handler 若有 instrument 需注意）；
   - rsland：Rust 1.83→1.87 + 全量升依赖后，**无任何 Box::pin 且 2 MiB 栈就够了**（自愈）；https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819/11
   - mitmproxy：tokio 升级带来的"niche-optimization"使某 future **微增**即把 Windows release 推过边缘；https://github.com/tokio-rs/tokio/issues/6057#issuecomment-2331098720
   - 官方层面：future 布局优化是进行中的编译器工作（2026 项目目标、`-Zpack-coroutine-layout`），**体积不具跨版本稳定性**。https://goals.rust-lang.org/2026/async-future-memory-optimisation.html 【有出处】

**关于本项目的 `opt-level="s"` + `lto=true` + `codegen-units=1`（release）对 future 体积是变大还是变小：【未找到】系统性研究。** 机制上双向皆可能：跨函数内联会把被调 async fn 的状态机并入外层（变大，Swatinem 文中 `#[inline]` 与状态机关系的讨论），优化也可能消除死状态/memcpy（变小）。社区案例双向都有（release-only 崩的 #6057/mitmproxy；debug-only 崩的 125212）。**可执行判据：在两个 profile 下分别用 `size_of_val` 实测并加断言，不做理论假设。**【社区共识】

### 3.2 「dev 崩，release 会不会崩」的判据

- 没有任何上游保证两方向的一致性；tokio#6057 的立论本身就是"debug 与 release 行为不一致"（且历史上是 release 更危险——装箱曾只在 debug 生效）。**唯一可靠做法：dev/release 各测一次体积与压测**，并以 clippy 门禁 + 体积断言测试固化。【社区共识】
- 同理跨平台也不一致（案例A：Mac 与 Linux 崩点不同；#6057 报告者在 mac/win 复现、mitmproxy 仅 Windows CI 崩）。CI 若只跑单平台需注意。【有出处】

### 3.3 生产监控/取证清单

- **崩溃形态**：`thread 'tokio-runtime-worker' has overflowed its stack` + `fatal runtime error: stack overflow`，随后 **SIGABRT（exit 134 = 128+6）**，进程级 abort 不可被 `catch_unwind` 捕获。原始输出样例：https://github.com/rust-lang/rust/issues/107358 （含 cargo 报告 `signal: 6, SIGABRT: process abort signal`）；https://github.com/tokio-rs/tokio/issues/6057#issuecomment-456856521 【有出处】
- 线程名即崩溃线程归属（`tokio-runtime-worker` = worker；测试线程名 = 测试用例名，见 #107358 的 `thread 'batches::…::test'`）。【有出处】
- 常规观测项：`/proc/<pid>/status` 的 `VmStk`（栈虚拟内存）、`Threads`（线程数）、`VmRSS`——通用 Linux 手册级手段（proc(5)，本次未逐字核对原文，标【社区共识】）：https://man7.org/linux/man-pages/man5/proc.5.html
- 上游没有提供"栈高水位"式的 async 栈监控；`stacker::remaining_stack()` 可在关键路径自报剩余栈（https://docs.rs/stacker/latest/stacker/ ），可埋点做预警。【有出处（组合用法为推断）】

### 3.4 CI 回归测试怎么做（可执行）

1. **体积门禁**（不触发崩溃、稳定、可并行）：
   - 微软指南模板：`#[test] fn … { let f = hot(); assert!(size_of_val(&f) < LIMIT); }`（LIMIT 首跑定值）；https://github.com/microsoft/rust-guidelines/blob/main/src/guidelines/performance/M-ASYNC-STACK-SIZE.md
   - clippy `large_futures`（默认阈值 16384 B，可经 clippy.toml `future-size-threshold` 调严，kornel 建议"装箱后用它找漏网大 future"）：https://doc.rust-lang.org/clippy/lint_configuration.html#future-size-threshold 【有出处】
2. **小栈崩溃回归**（需子进程隔离，因 SIGABRT 不可进程内捕获）：
   - `cargo test` 的**每个测试用例本来就跑在 2 MiB 新线程上**（SO 42955243）——"小栈场景"在测试线程上是天然复现环境；要放大用 `RUST_MIN_STACK=8388608 cargo test`（同帖给出命令），要缩小时同样受它影响，**注意测试 harness 自身噪声**；https://stackoverflow.com/questions/42955243/cargo-test-release-causes-a-stack-overflow-why-doesnt-cargo-bench
   - 断言方式：把被测对象放入**子进程**跑，父进程断言 stderr 含 `has overflowed its stack` 且退出码 134（或 `signal: 6`）——#107358 展示了 cargo/系统层的完整输出形态可直接对拍；https://github.com/rust-lang/rust/issues/107358
   - `RUST_MIN_STACK` 必须在**子进程启动前**注入其环境（std 文档警告"启动后修改可能被忽略"，见 2.2）；【有出处】
   - `#[tokio::test]` 的 future 在测试线程上 poll（#107358 的事故就发生在 `tokio::test` 上）——回归测试与生产的线程模型差异要在用例注释中写明。【有出处】
3. **dev 差异的低成本缓解**（若设计允许）：`[profile.dev] opt-level = 1`（三案例实证可消除 debug 爆栈；本项目当前本地代码 opt-level=0）。注意这只是缓解，**不替代体积门禁**。https://users.rust-lang.org/t/stack-overflow-in-debug-even-after-stack-increase-ok-in-release/125212/3 【有出处】

---

## 4. 主题 4：同类真实案例（均有可点击出处）

| 案例 | 场景 | 处置与教训 | 出处 |
|---|---|---|---|
| **tokio#6057**（2023，已修复） | release 构建裸 `spawn(future())`（200 KB future）爆栈；debug 正常 | 根因：装箱曾只在 debug_assertions 生效；PR #6826 改为无条件按尺寸装箱。教训：**dev 通过 ≠ release 安全** | https://github.com/tokio-rs/tokio/issues/6057 |
| **mitmproxy_rs**（mhils 亲述） | Windows CI release-only 爆栈；依赖小版本升级（tokio niche 优化）使 future 微增即崩 | 全量 `Box::pin` 钉住修复；并临时 patch tokio 对 `size_of::<T>() > BOX_FUTURE_THRESHOLD` panic 以**盘点全仓库超标 future** | https://github.com/tokio-rs/tokio/issues/6057#issuecomment-2331098720 |
| **商业代码生成服务器**（Rust Forum 117224） | `tokio-runtime-worker` 爆栈（54K 行生成代码） | Linux：2/4 MiB 崩、8 MiB 活；`thread_stack_size(1024*8000)` + `block_on(Box::pin(…))`；Mac/Linux 崩点不同 | https://users.rust-lang.org/t/causes-for-thread-tokio-runtime-worker-has-overflowed-its-stack/117224 |
| **Microsoft 工程师 Osterman**（Forum 109571） | Windows 1 MiB 主线程爆栈（main 栈帧 281K） | 多处 `Box::pin(call()).await` 后 main 栈 280K→90K；帖子含 `-Zprint-type-sizes` 完整教学 | https://users.rust-lang.org/t/stack-overflow-in-async-main-function-due-to-excessive-stack-allocation-not-from-recursion/109571 |
| **shaw**（Forum 125212） | **debug 崩、release 正常**（解析函数 2.6 MB 栈帧） | clippy `large_stack_frames` 定位；`[profile.dev] opt-level=1` 修复 | https://users.rust-lang.org/t/stack-overflow-in-debug-even-after-stack-increase-ok-in-release/125212 |
| **rust#107358** | rustc 1.66→1.67 升级即爆栈（测试线程）；100 MiB 栈都救不了的一例；`#[tracing::instrument]` 诱发 | 最终 `RUST_MIN_STACK` 环境变量解决；有人以"heap 分配 future + join→spawn"修复 | https://github.com/rust-lang/rust/issues/107358 |
| **rsland**（Forum 129819） | panic=unwind 下 1 GiB 栈不够，abort 下 2 MiB 够 | unwind 的 Box 双倍栈构语义（bjorn3 解释）；opt-level=1 可救；升 Rust 1.83→1.87 后自愈 | https://users.rust-lang.org/t/stack-overflow-with-panic-unwind-but-ok-with-panic-abort/129819 |
| **rust#53827**（2018 至今开放） | `Box::new(大数组)` 双倍栈占用（含 release 复现） | 官方 workaround：`vec![..].into_boxed_slice()`、`Box::new_uninit`、`bytemuck::zeroed_box`；"优化不是保证" | https://github.com/rust-lang/rust/issues/53827 |
| **Krustlet**（wg-async 官方叙事） | `select!` + `pin_mut!` 把 future 钉在栈上 | 改 `Box::pin` 钉堆解决 | https://rust-lang.github.io/wg-async/vision/submitted_stories/status_quo/alan_runs_into_stack_trouble.html |
| **Sentry symbolicator / moka-rs#212** | 嵌套泛型 async 链导致 future 体积按层放大（get_with 约 7×） | Swatinem 以 `Box::pin` 解决并写成长文 | https://swatinem.de/blog/future-size/ ；https://github.com/moka-rs/moka/issues/212 |

---

## 5. 与本项目参数直接相关的可执行结论（可直接抄进设计方案）

**核实结论（回应复核方）**

1. 「tokio 1.52.3 自动 box 大 future」**属实**：`spawn` 对 `size_of::<F>() > BOX_FUTURE_THRESHOLD` 无条件 `Box::pin`；阈值 dev **2048 B** / release **16384 B**（1.52.3 源码：`task/spawn.rs` + `runtime/mod.rs:643`；历史与修复：#6057/#6826）。**但装箱发生在调用点构造之后**，243,880 B future 在 >1.5 MiB 深栈上的构造/搬运（≈488 KB）不受其保护——本次爆栈与该机制自洽，修法不能依赖"spawn 会装箱"。

**修法选项（上游依据强度排序）**

2. 首选：**拆分 1360 行 async 块 + 在多个边界 `Box::pin(call()).await`**（实测可把栈占用 280K→90K；kornel"趁小装"原则：装箱点必须在对象还小时）。配套：大参数传引用；不跨 await 的大局部收窄作用域或入 `Vec`。
3. 结构等价替代：`tokio::spawn(async move { Box::pin(inner(args)).await })`——把巨型块构造挪到新任务的浅栈（推断级，机制依据见 1.3 表 #2）。
4. 兜底（可与其他项叠加）：手动 runtime builder 设置 `thread_stack_size(8 * 1024 * 1024)`（8 MiB 为社区实测收敛值；**4 MiB 有失败案例**；`#[tokio::main]` 不可配此项，须改手动 builder）。代价 = N_workers × 8 MiB 虚拟预留，RSS 按需。
5. 明确**不要**指望的：`spawn_blocking` 换大栈（blocking 线程默认同样 2 MiB）；`RUST_MIN_STACK` 覆盖显式 `thread_stack_size`（官方：显式设置优先）；运行时 `env::set_var("RUST_MIN_STACK", …)`（官方：启动后修改可能被忽略）。
6. 可选增强：`stacker::maybe_grow(32 * 1024, 1024 * 1024, \|\| { … })` 在 poll 边界动态扩栈（组合用法，无 tokio 官方示例）。

**取值与门禁（数字）**

7. 栈账目：默认 worker 2 MiB（std 官方）< 本项目实测需求（>1.5 MiB + ≈0.5 MiB）；建议 **8 MiB**，并在 CI 中用 `#[test] assert!(size_of_val(&f) < LIMIT)`（微软模板）+ clippy `large_futures`（默认阈值 **16384 B**，可调严）双门禁。
8. dev 差异缓解：`[profile.dev] opt-level = 1`（三案例实证；当前项目本地代码为 0，仅依赖为 1）。
9. release 不豁免：`opt-level="s"+lto` 对体积的影响**无系统研究**，必须双 profile 实测；崩溃形态对拍字符串 `has overflowed its stack` / `fatal runtime error: stack overflow`、退出码 **134（SIGABRT）**，回归测试须子进程隔离。
10. 版本漂移风险：rustc 小版本、依赖微升级即可翻转崩溃点（#107358、mitmproxy、rsland 三案）；修复后的体积断言测试是唯一长期护栏。

---

## 6. 证据分级汇总

| 结论 | 等级 |
|---|---|
| tokio 1.52.3 spawn 自动装箱 + 阈值 2048/16384 | 有出处（源码 + #6057/#6826） |
| 自动装箱不消除调用点 1–2× 构造栈 | 社区共识（Swatinem 实测 + kornel + #53827）+ 本项目实测一致 |
| std spawn 线程默认 2 MiB；主线程非 Rust 决定；RUST_MIN_STACK 语义 | 有出处（std 官方文档原文） |
| `#[tokio::main]` 无 thread_stack_size 选项 | 有出处（attr.main 文档全量可选项） |
| thread_stack_size 默认 None（⇒2 MiB）、作用于 runtime 线程（含 blocking） | 有出处（builder.rs / blocking/pool.rs 源码） |
| RUST_MIN_STACK 影响未配置的 tokio worker | 推断（std 文档 + tokio 默认值链） |
| worker 栈推荐 8 MiB；4 MiB 不够 | 有出处（Forum 117224 实测）+ 推断（本项目账目） |
| `Box::pin` 边界装箱有效（280K→90K） | 有出处（Forum 109571） |
| opt-level=1 消除 dev 爆栈 | 有出处（三个独立案例） |
| unwind 的 Box 双倍栈构语义 | 有出处（Forum 129819，bjorn3） |
| panic=abort 允许堆上原地构造 | 有出处（同上，个案级解释） |
| opt-level="s"/LTO 对 future 体积方向 | 未找到（无系统研究；社区案例双向） |
| 跨平台崩溃点一致 | 不成立（有出处：Mac/Linux 崩点不同） |
| tracing/instrument 放大 future | 有出处（#107358 个案） |
| Linux 线程栈"虚拟预留、RSS 按需" | 社区共识（proc(5)/pthread 手册层面，未逐字核对） |

## 7. 来源清单

**保留（一手/官方）**
- tokio 1.52.3 源码三处（spawn.rs / runtime/mod.rs / builder.rs / blocking/pool.rs，raw.githubusercontent tag 直读）
- tokio#6057 及 mhils/sharifhsn 评论、PR#6826 diff —— 自动装箱的历史、动机、修复与真实事故
- doc.rust-lang.org：std/thread（栈默认与 RUST_MIN_STACK 原文）、Unstable Book（print-type-sizes）
- docs.rs：tokio attr.main / Builder::thread_stack_size / runtime index / task::spawn（1.52.3 锁版 URL）、stacker
- goals.rust-lang.org 2026「Async Future Memory Optimisation」（#62958/#135527/-Zpack-coroutine-layout）
- microsoft/rust-guidelines M-ASYNC-STACK-SIZE（体积断言测试模板、瘦身指引）
- rust-lang：#53827（Box::new 双倍栈）、#107358（版本回归 + RUST_MIN_STACK 结案）
- wg-async「Alan runs into stack trouble」（Krustlet 案例）
- Swatinem「The size of Rust Futures」（机制长文）
- Rust Forum 四帖：117224 / 109571 / 125212 / 129819（实测数字与多人互证）
- Stack Overflow 两帖：42955243（cargo test 2 MiB 线程 vs bench 主线程）、77934697（Box::new 拷贝行为）
- clippy：lints index #large_futures + lint_configuration #future-size-threshold（默认 16384）
- japaric/cargo-call-stack（README 自述适用边界）
- moka-rs/moka#212（体积膨胀实拍 print-type-sizes 输出）

**排除**
- cprimozic「Fixing Rust+WebAssembly Memory Access Out of Bounds」（WASM 场景，非 tokio 线程模型，仅背景相似）
- 各搜索结果中的 docs.rs 镜像/重复页（otter-api-tests 等 noise）
- rust-guidelines 仓库其余条目（与本问题无直接关系）

## 8. Gaps（未能确证，供设计方案注意）

1. **`opt-level="s"`+`lto`+`codegen-units=1` 对 future 体积的方向性影响：无系统研究**。只有机制讨论与双向个案。下一步：在 dev/release 两 profile 各跑一次 `size_of_val`（或 `-Zprint-type-sizes`）拿到本项目的真实两组数字。
2. **`RUST_MIN_STACK` 是否被 std 缓存一次**：官方仅一句"may be ignored after program start"，未找到缓存时机的权威描述。使用上按"仅在进程启动前设置"处理即可规避。
3. **tokio worker 线程消费 `thread_stack_size` 的精确源码行**：本次在 1.52.3 的 worker.rs/runtime.rs 未 grep 到（可能在重构后的其他模块）；以 builder.rs 字段注释"threads spawned by the runtime"+ 官方文档页佐证，结论不受影响。
4. **`stacker` 与 tokio 组合的官方示例**：stacker 文档不含 async 示例；如设计采用，需自行验证 `maybe_grow` 包裹 poll 的行为（red_zone 参数需大于 poll 期间的瞬时需求）。
5. **panic=abort 的堆上原地构造**为个案级解释（bjorn3 推理），未找到编译器团队正式文档背书；本项目两 profile 均默认 unwind，设计可按"双倍搬运在 release 同样存在"保守假设。
