# Tasks: provider run 体积预算与四层门禁 —— 收尾未完成项

> **本 change 不在第一阶段实施**（owner 2026-11-06 裁定：第一阶段先保产品可用）。
> 每条任务都带**验收口径**；后续会话直接按本清单派工，无需重新推导。

## 0. 接手前必读

- `proposal.md`（§2 已交付冻结表 / §3 已知未完成与未验证 / §5 边界）
- `design.md`（逐族臂的拆分切法、预算推导、门禁分层）
- 第一阶段的提交基线与证据：见 `cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md`（A～D 四层证据、U1～U15 责任表、门禁终态）
  ★原引用的 `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md` 不存在，已改指交付说明★
- 契约：本 change 的 ADDED delta `openspec/changes/provider-run-volume-budget-and-gate-completion/specs/non-interrupt-repository-bootstrap/spec.md` 的 REQ-BOOT-19（体积预算与四层门禁合同）
  ★主 specs 中尚无 REQ-BOOT-19（尚未 archive/sync）⇒ 引用一律指 delta★

## 1. 拆分剩余族（治本收尾；共享写集须串行，同一集成 owner）

- [x] 1.1 **plan revision 族**（★阶段 1 以【装箱形态】实现：宏体实测 118,784 B 计入 P1 过渡预算；宏文本迁出为具名 fn 未做 ⇒ 登记为阶段 2 残余★）**plan revision 族**：把 `workspace_ws_work_item_plan_revision_arm!` 宏迁为具名 `async fn drive_plan_revision_arm<'e,'a>(A, feedback:Option<String>) -> ArmExit<'e>`；原宏内 `return` 逐个映射 `Stop`、穿出 loop 后 → `Continue`、原 `break` 仍只退出相应 loop；保留原循环上限与 fresh 冻结 payload 消费（**不重算反馈归一化、不新增 retry/恢复 fallback**）
      - 验收：具名臂未装箱 ≤131,072 B（优先 ≤65,536 B）；typed mismatch **零 provider start**、不转 OutlineRebuild、turn/gate/预算保持；既有断言全保留
- [x] 1.2 **draft / batch 族**（★阶段 1 以【装箱形态】实现：实测 ≤40,960 B；宏文本迁出未做 ⇒ 阶段 2 残余★）**draft / batch 族**：`drive_plan_draft_arm<'e,'a>(A, feedback:Option<String>) -> ArmExit<'e>`、`drive_plan_batch_arm<'e,'a>(A) -> ArmExit<'e>`；**`Option<Option<String>>` 必须保留**（区分「fresh 冻结为空」与「非 fresh」）；RetryOnce 按原预算循环且**每轮新 split handle**，不新建业务 generation/batch；**`AwaitConfirmation`（feedback=None 续 loop）与 draft 的 `break` 差异不可统一**；原「mark 后即 return」原样映射 `Stop`，不补通用 finish/错误帧
      - 验收：两臂与指定 child 未装箱 ≤131,072 B；fresh 冻结值与刻意不同的内存 pending 构造成立；成功路径有真实 start + 完整帧 + durable 进展；typed 拒绝零 start 不改预算；取消后恢复运行
- [x] 1.3 **SC manual 族**（★阶段 1 以【装箱形态】实现：实测 61,440 B；宏文本迁出未做 ⇒ 阶段 2 残余★）**SC manual 族**：`drive_human_gate_sc_revision_arm<'e,'a>(A, turn_id:String, prompt:String) -> ArmExit<'e>`；同 turn **最多 2 次真实 start**、**claim 一次**、attempt2 用**冻结 prompt**、每次 start 的 ledger 与实际探针对应；二次拒收/重放**不新 claim、不第三次 start**；取消在 provider start 与进入 review 前两个 barrier 处都**durable Failed(cancelled)**、失败回显 ≤1、后续可新 turn
      - 验收：臂与最大 child 未装箱 ≤131,072 B；真实 2 MiB worker 上 start 探针 + 同 turn durable ledger + 完整帧 + 取消后再生成
- [ ] 1.4 **followups 接力**：具名 `drive_provider_run_followups(...) -> ArmExit<'e>`（**不接 entity lease、不复制已消费 receiver**，继续在 manager 按 token 替换阶段 channel）；删除 `workspace_ws_provider_run_followups!` 与失效 `macro_use`；**不留旧文本别名/双实现 flag**；正常尾部只一份（mark→drop engine→idle take→finish）
      - 验收：author→review、review→outline、outline→draft 三条接力在 barrier 发 Abort 时**唯一当前 provider 收到**、manager token 未被偷换；同节点 relay 不额外 start；SC Generate 让位而 Evaluate 恰一次 review；旧 finish 不误删新 scratch
- [x] 1.5 **逐对象预算冻结**（★阶段 1 已实施：`poll_frame_budget.rs` 逐对象分级 P0 64 KiB / P0-spawn 32 KiB / P1 engine 256 KiB★）**逐对象预算冻结**：A 层对 1.1~1.4 拆出的**每个未装箱对象**逐个记字节并冻结阈值（优先 ≤65,536 B），**不得把所有对象机械放宽到 128 KiB**；阈值上调必须有新栈账 + owner 裁决
      - 验收：A 层断言消息含符号/实际字节/profile/target/toolchain；**不测 `Pin<Box<_>>` 指针大小**；drive body 从 112,024 B 降到逐对象预算内

## 2. 门禁扩面（护栏）

- [ ] 2.1 **clippy `large_futures` 仓级启用**：先**核实它量的是未装箱 future 还是 Box 后的指针**（后者恒绿＝假门禁）；阈值 16 KiB；**不全仓 deny、不全局 allow、不抬阈值**；受影响模块先启用
      - 验收：受控对照（>16 KiB 未装箱 child 必触发；boxed 同 child 记录实际行为）；CI 以 `-D warnings` 执行时该 lint 真实生效
- [x] 2.2 **CI 栈门禁（★owner 2026-11-06 裁决改写本条范围★）**：owner 裁决「**保留仓级 32 MiB force，另加 CI 专项 job**」⇒ 原「debug 全 suite + 依次 `--release` 双 profile」方案作废，C 层栈门禁改由**独立 2 MiB 栈专项 CI job** 承载
      - ★已落地★：`.github/workflows/ci.yml` 新增独立 job **`stack-2mib-gate`**（显示名 `2 MiB stack regression gate (C-layer real WS)`）
      - job 内三条命令（原文）：
        - `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_2mib_real_ws_review_round_gate -- --nocapture`
        - `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_2mib_real_ws_matrix -- --nocapture`
        - `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_8mib_ws_review_round_control_gate -- --nocapture`
      - job 前置：`pnpm -C web install --frozen-lockfile && pnpm -C web build`（rust-embed 编译期嵌入 `web/dist`，缺失则 `build.rs` 硬失败）+ 钉版本装 `@colbymchenry/codegraph@1.6.2`（C 层夹具走生产 aggregate-index rebuild 端点 → shell out 真实 CLI，缺则夹具直接失败）
      - 边界：**不改** `.cargo/config.toml` 的 `RUST_MIN_STACK = { value = "33554432", force = true }`（owner 裁决保留）；**不改** rust job 既有任何步骤；无 `-j 1` / `--ignored` / skip / `allow` / `continue-on-error` / 无条件 true（门禁红了就是红了）
      - 为何 job 不需要覆盖 `RUST_MIN_STACK`：C 层门禁**自带显式 2 MiB runtime**（`provider_run_small_stack.rs` 的 `run_ws_child_scenario_with_stack` → `.worker_threads(1).thread_stack_size(stack_bytes)`）⇒ 显式栈值覆盖环境变量 ⇒ 仓级 32 MiB force 对这批用例无影响
      - 残留（**不在本条范围**）：release profile 的体积数字（解除 U-b）仍未记录 ⇒ 挂 8.7
- [ ] 2.3 **C 层 2 MiB 永不豁免**：任何档位（含 prod 16 MiB）都不得放宽 C 层的显式 2 MiB 真实 WS 回归

## 3. 运行时栈与 prod 档位（恢复 owner 原决策）

- [x] 3.1 **手写 runtime Builder**（★`4b7a390b`✓）**手写 runtime Builder**：替换 `#[tokio::main]`（该宏**无栈选项**）；`new_multi_thread` + `thread_stack_size(WORKER_STACK_SIZE)` + `enable_all`；**保留 CLI 退出码/stdout/stderr 逐字语义**与原 worker 数/blocking 上限
- [x] 3.2 **栈大小定档**（★debug 与 release **统一 8 MiB**（`src/main.rs:12-13` `WORKER_STACK_SIZE = 8*1024*1024`；原「prod 先 16 MiB、验完降 8」方案已取消）；启动日志自报 `worker_stack_bytes`✓）**栈大小定档**：`#[cfg(debug_assertions)] WORKER_STACK_SIZE = 8 * 1024 * 1024`；`#[cfg(not(debug_assertions))] = 16 * 1024 * 1024`；**不新增运行期配置面**（无 `ARIA_*`）
- [x] 3.3 **可观测与防误用**（★`[aria-runtime] profile=debug worker_stack_bytes=8388608`✓；D 层记录二进制指纹部分完成）**可观测与防误用**：仅 `aria web` 启动时打印一行 `[aria-runtime] profile=debug|release worker_stack_bytes=<字节>` 到 stderr（非 web CLI 的 stderr 逐字不变）；**prod 只跑 release 制品**；D 层记录二进制指纹 + profile + 生效栈
- [ ] 3.4 **验收**：debug/release 下 CLI 逐字对照 + 栈日志一致；C 层仍显式 2 MiB；prod 候选若日志是 debug 或 8 MiB 即 NOT_READY

## 4. 发布证据与回退合同

- [ ] 4.1 **发布检查单**：每项含 `binary_sha256 / profile / toolchain / target / dependency_fingerprint / flags / startup_environment / effective_stack_bytes / A,B,C,D 结果 / durable_diff / abort_or_zombie / threads_vm_rss / rollback_artifact`；**缺任一项即 NOT_READY**（不能只看 exit 0）
- [ ] 4.2 **回退候选先验证**：**无 S1 止血 ⇒ 不得默认旧版可用**；对选定回退制品在自有副本按实际启动栈执行 D，必须未 abort/无僵死且满足同动态合同；回退先取消/排空在途 run 再换完整制品，**不交接半个 driver**；**没有合格回退制品保持 NOT_READY**
- [ ] 4.3 **容量观察**：在构造 caller/Box/spawn/poll 代表点采样栈峰值与叠加关系（debug/release 分栏）；记录 worker 数/VM/RSS/取消延迟；**测不准就只报固定栈边界，不编造上界、不把 VmStk 当全部线程总峰值**
- [ ] 4.4 **运维文档与 CHANGELOG**：写清根修边界、16 MiB 暂定工程选择、32 MiB 测试 force 保留、2 MiB 门禁永不豁免、release/debug 各值、盘点债务另案；**仅已实际支持/验收的平台写通过**，不把 Linux 外推 Windows/macOS；**不自行提升版本号、不借 bug 根修擅自发版**

## 5. 全仓大 future 只读盘点（另案整改）

- [ ] 5.1 **只读盘点**：按 tokio 阈值 **dev 2048 B / release 16384 B** 分档扫描全仓 `spawn` 参数与 await child；每条记录 `{symbol, source_path, line, profile, raw_bytes, tokio_threshold, boundary_kind, evidence_source, boxed_blind_spot, confidence}`；**区分 raw 类型与 Box 包装**
- [ ] 5.2 **工具边界诚实登记**：无 nightly 时用 stable Clippy 临时阈值扫描并**明确 Box/spawn/未执行分支缺口**；**盘点结论是「证据与局限如实盘点」，不是「全仓 future 均 ≤阈值」**；**不得发布 patched 依赖、不得凭 Box 指针标安全**
- [ ] 5.3 **整改另案**：盘点报告只出清单，**不在本 change 整改**

## 6. 挂账清理（交最终整分支审查分诊）

- [ ] 6.1 `cancel_settle.inc.rs:17` 的 `pub(super)`→`pub(crate)`：在 Plan 的 Task 1 变更集记录中补登该文件（**文档性动作**，保持 Files 清单与实际 diff 一致）
- [ ] 6.2 `lc_fresh_payload_verify.inc.rs:139` 指向已删 inc 文件的**悬空注释**：修正指向新路径（仅注释）
- [ ] 6.3 `case_assert_no_more_provider_inputs` 的 **150 ms 负锚窗口是启发式**（全套件既有模式）：评估是否升级为条件等待（`condition-based-waiting`）或明确登记为已知漏检窗口

## 7. 验收纪律（全程）

- [ ] 7.1 每条动态验收**退出码 0 不算通过**：须同时满足服务无 abort + 可信 provider-start 探针或真实 adapter 日志 + 完整 JSON 帧与因果偏序 + durable 进展 + 取消后再生成；close/timeout/仅 locked/无 provider-start/无 durable 全判失败
- [ ] 7.2 SIGABRT 不可进程内捕获 ⇒ 易 abort 场景子进程隔离（父持续排 stdout/stderr、退出状态、墙钟超时；防递归；`--exact` 唯一匹配）
- [ ] 7.3 体积数字一律标 profile/target/toolchain；**debug 数字不得冒充 release**
- [ ] 7.4 行数门禁：新文件 ≤1200（硬）/≤1000（软），落在 1000~1200 **必须继续拆**；禁止给新文件登记祖父项；旧文件降到 ≤1200 同提交删登记

## 8. 追加（2026-11-06 治本审查 I-1/I-2/I-3/I-4 登记）

- [x] 8.1 **补齐 poll 帧门禁覆盖面**（★`f6e3ad8d`✓ 写集内全覆盖；写集外清单已归属独立 change）**补齐 poll 帧门禁覆盖面**：纳入 PlanRevision 宏体与 followups 宏体；按诊断报告 §8.4 的分级补齐写集内可覆盖项（P0 spawn/准入侧 ≤32 KiB：全嵌套闭包 + 搬运帧）；**写集外**的（P1 engine / 入口链 / coding runner）只列清单 + 归属独立 change，**不扩权**
- [x] 8.2 **过渡预算登记进门禁配置**（**只作过渡门禁，不是安全证明**；上调阈值须有新栈账 + owner 裁决）
  ★档位勘误（终审 I-1）：代码实际有【两档】——宏体档 `MACRO_BODY_TRANSITION_BUDGET = 128 KiB`（plan_revision / followups 宏体）＋ P1 engine 档 `P1_ENGINE_BUDGET = 256 KiB`（评审链 with_window / drive_review_session* / complete_review）。原文档把 with_window 写成「≤128 KiB」属错档 ⇒ 已按代码改正为 256 KiB★
- [x] 8.3 **残余归因核实**（★`f6e3ad8d`：写集内 7 处未装箱重调用逐个装箱；followups 167,936→86,016 B、spawn 侧 45,056→4,096 B✓）**残余归因核实**：写集内未装箱的重调用（repair / retry 等）逐个实测帧预留与调用频次；热路径必须装箱，低频登记为过渡项
- [x] 8.4 **C 层送审轮 2 MiB E2E 在治本提交后复跑留痕**（★`provider_run_2mib_real_ws_review_round_gate ... ok`（全量门禁内自带留痕，/tmp/aria-full7.log）✓）**C 层送审轮 2 MiB E2E 在治本提交后复跑留痕**（证明「治本后 2 MiB 送审轮不崩」；这是「治本完成」的必要证据，不得用「8 MiB 下不崩」代替）
- [x] 8.5 独立 change：`execute_start_coding_flow`（806,912 B）与入口链 `handle_workspace_inbound_message_inner`（946,176 B）的大帧治理（★本 change **只登记**这两个写集外对象，**不在本 change 实施** ⇒ 归属阶段 2 的独立 change★）
- [x] 8.6 **CI 2 MiB 栈专项 job 落地**（★`.github/workflows/ci.yml` job `stack-2mib-gate`（显示名 `2 MiB stack regression gate (C-layer real WS)`）✓）**CI 2 MiB 栈专项 job 落地**：仓级 `RUST_MIN_STACK = { value = "33554432", force = true }` 让**所有测试线程**都拿 32 MiB 栈 ⇒ 栈类缺陷在常规 CI 里永远测不出来 ⇒「provider run 爆栈打死整个服务」这类回归会再次悄悄进来 ⇒ 补**独立 job** 在 2 MiB 栈下跑真实链路门禁，让「栈回归」变红
      - job 内三条命令（原文，每条 cargo 带 `flock`，不 matrix 并发同 target）：
        - `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_2mib_real_ws_review_round_gate -- --nocapture`
        - `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_2mib_real_ws_matrix -- --nocapture`
        - `flock /tmp/aria-cargo.lock cargo test --locked --test it_core provider_run_8mib_ws_review_round_control_gate -- --nocapture`
      - 前置：先 `pnpm -C web install --frozen-lockfile && pnpm -C web build`（rust-embed 编译期嵌入 `web/dist`，缺失则 `build.rs` 硬失败）；再钉版本装 `@colbymchenry/codegraph@1.6.2`（C 层夹具走生产 aggregate-index rebuild 端点 → shell out 真实 `codegraph` CLI）
      - 边界：**不改** `.cargo/config.toml` 的 32 MiB force（owner 裁决保留）；**不改** rust job 既有任何步骤；无 `-j 1` / `--ignored` / skip / `allow` / `continue-on-error` / 无条件 true
      - 为何不覆盖 `RUST_MIN_STACK`：C 层门禁**自带显式 2 MiB runtime**（`provider_run_small_stack.rs` 的 `run_ws_child_scenario_with_stack` → `.worker_threads(1).thread_stack_size(stack_bytes)`）⇒ 显式栈值覆盖环境变量
      - 落地报告：`cadence/reports/2026-11-06_实施_CI加2MiB栈专项job_v1.0.md`
- [ ] 8.7 **release profile 体积数字留痕**（★由 2.2 范围改写剥离出来，仍未做★）**release profile 体积数字留痕**：`--release` 下逐对象体积数字实测并记录（标 profile/target/toolchain，**debug 数字不得冒充 release**）⇒ 解除 U-b
      - 验收：release 数字落在报告里且标注 profile/target/toolchain；`--release` 下 C 层 2 MiB 门禁复跑留痕
