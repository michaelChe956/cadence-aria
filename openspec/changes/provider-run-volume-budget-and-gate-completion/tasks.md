# Tasks: provider run 体积预算与四层门禁 —— 收尾未完成项

> **本 change 不在第一阶段实施**（owner 2026-11-06 裁定：第一阶段先保产品可用）。
> 每条任务都带**验收口径**；后续会话直接按本清单派工，无需重新推导。

## 0. 接手前必读

- `proposal.md`（§2 已交付冻结表 / §3 已知未完成与未验证 / §5 边界）
- `design.md`（逐族臂的拆分切法、预算推导、门禁分层）
- 第一阶段的提交基线：见 `cadence/reports/2026-11-06_验收报告_provider_run根修证据_v1.0.md`（A～D 四层证据与 U1～U15 责任表）
- 契约：`openspec/specs/non-interrupt-repository-bootstrap/spec.md` 的 REQ-BOOT-19（体积预算与四层门禁合同）

## 1. 拆分剩余族（治本收尾；共享写集须串行，同一集成 owner）

- [ ] 1.1 **plan revision 族**：把 `workspace_ws_work_item_plan_revision_arm!` 宏迁为具名 `async fn drive_plan_revision_arm<'e,'a>(A, feedback:Option<String>) -> ArmExit<'e>`；原宏内 `return` 逐个映射 `Stop`、穿出 loop 后 → `Continue`、原 `break` 仍只退出相应 loop；保留原循环上限与 fresh 冻结 payload 消费（**不重算反馈归一化、不新增 retry/恢复 fallback**）
      - 验收：具名臂未装箱 ≤131,072 B（优先 ≤65,536 B）；typed mismatch **零 provider start**、不转 OutlineRebuild、turn/gate/预算保持；既有断言全保留
- [ ] 1.2 **draft / batch 族**：`drive_plan_draft_arm<'e,'a>(A, feedback:Option<String>) -> ArmExit<'e>`、`drive_plan_batch_arm<'e,'a>(A) -> ArmExit<'e>`；**`Option<Option<String>>` 必须保留**（区分「fresh 冻结为空」与「非 fresh」）；RetryOnce 按原预算循环且**每轮新 split handle**，不新建业务 generation/batch；**`AwaitConfirmation`（feedback=None 续 loop）与 draft 的 `break` 差异不可统一**；原「mark 后即 return」原样映射 `Stop`，不补通用 finish/错误帧
      - 验收：两臂与指定 child 未装箱 ≤131,072 B；fresh 冻结值与刻意不同的内存 pending 构造成立；成功路径有真实 start + 完整帧 + durable 进展；typed 拒绝零 start 不改预算；取消后恢复运行
- [ ] 1.3 **SC manual 族**：`drive_human_gate_sc_revision_arm<'e,'a>(A, turn_id:String, prompt:String) -> ArmExit<'e>`；同 turn **最多 2 次真实 start**、**claim 一次**、attempt2 用**冻结 prompt**、每次 start 的 ledger 与实际探针对应；二次拒收/重放**不新 claim、不第三次 start**；取消在 provider start 与进入 review 前两个 barrier 处都**durable Failed(cancelled)**、失败回显 ≤1、后续可新 turn
      - 验收：臂与最大 child 未装箱 ≤131,072 B；真实 2 MiB worker 上 start 探针 + 同 turn durable ledger + 完整帧 + 取消后再生成
- [ ] 1.4 **followups 接力**：具名 `drive_provider_run_followups(...) -> ArmExit<'e>`（**不接 entity lease、不复制已消费 receiver**，继续在 manager 按 token 替换阶段 channel）；删除 `workspace_ws_provider_run_followups!` 与失效 `macro_use`；**不留旧文本别名/双实现 flag**；正常尾部只一份（mark→drop engine→idle take→finish）
      - 验收：author→review、review→outline、outline→draft 三条接力在 barrier 发 Abort 时**唯一当前 provider 收到**、manager token 未被偷换；同节点 relay 不额外 start；SC Generate 让位而 Evaluate 恰一次 review；旧 finish 不误删新 scratch
- [ ] 1.5 **逐对象预算冻结**：A 层对 1.1~1.4 拆出的**每个未装箱对象**逐个记字节并冻结阈值（优先 ≤65,536 B），**不得把所有对象机械放宽到 128 KiB**；阈值上调必须有新栈账 + owner 裁决
      - 验收：A 层断言消息含符号/实际字节/profile/target/toolchain；**不测 `Pin<Box<_>>` 指针大小**；drive body 从 112,024 B 降到逐对象预算内

## 2. 门禁扩面（护栏）

- [ ] 2.1 **clippy `large_futures` 仓级启用**：先**核实它量的是未装箱 future 还是 Box 后的指针**（后者恒绿＝假门禁）；阈值 16 KiB；**不全仓 deny、不全局 allow、不抬阈值**；受影响模块先启用
      - 验收：受控对照（>16 KiB 未装箱 child 必触发；boxed 同 child 记录实际行为）；CI 以 `-D warnings` 执行时该 lint 真实生效
- [ ] 2.2 **双 profile 常规 CI**：debug 全 suite + 依次 `--release` 跑 A/B/C 指定门禁（体积硬断言 / 2 MiB worker 矩阵 / 真实 WS）；每条 cargo 带 `flock`；**不 matrix 并发同 target**
      - 验收：两 profile 全绿；**release 的体积数字被记录**（解除 U-b）
- [ ] 2.3 **C 层 2 MiB 永不豁免**：任何档位（含 prod 16 MiB）都不得放宽 C 层的显式 2 MiB 真实 WS 回归

## 3. 运行时栈与 prod 档位（恢复 owner 原决策）

- [ ] 3.1 **手写 runtime Builder**：替换 `#[tokio::main]`（该宏**无栈选项**）；`new_multi_thread` + `thread_stack_size(WORKER_STACK_SIZE)` + `enable_all`；**保留 CLI 退出码/stdout/stderr 逐字语义**与原 worker 数/blocking 上限
- [ ] 3.2 **编译期分档**：`#[cfg(debug_assertions)] WORKER_STACK_SIZE = 8 * 1024 * 1024`；`#[cfg(not(debug_assertions))] = 16 * 1024 * 1024`；**不新增运行期配置面**（无 `ARIA_*`）
- [ ] 3.3 **可观测与防误用**：仅 `aria web` 启动时打印一行 `[aria-runtime] profile=debug|release worker_stack_bytes=<字节>` 到 stderr（非 web CLI 的 stderr 逐字不变）；**prod 只跑 release 制品**；D 层记录二进制指纹 + profile + 生效栈
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
