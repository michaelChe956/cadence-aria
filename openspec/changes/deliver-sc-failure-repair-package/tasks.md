# 工作包

> 顺序固定：先契约 → 后实现 → 再门禁。每包是独立可验收的高层包；精确修改步骤、命令、测试与提交拆分留给后续已确认 Plan。共 26 包（≤41）；消费闭集W11a–g各≤3文件／≤3h，见design §7。

## 一、先契约

- [x] **W01 契约与来源范围确认**（REQ-WSC-33/34/35）：核对本 change 四件 OpenSpec 产物、归档件 A3/A4 取代关系、三类终局边界与两项窄例外；验收：严格 change validate 通过，非 SC legacy 明确为负面场景。
- [x] **W02 wire schema 与证据状态冻结**（REQ-WSC-33/34）：冻结 `ScFailureRepairPackage`、round evidence、available/missing、incomplete、locator 版本和 SHA-256/bytes 口径；验收：字段清单能覆盖所有 spec 场景且不含未裁定 None。
- [x] **W03 运行时顺序与失败优先级冻结**（REQ-WSC-33/34/35）：冻结 RoundBound→provider→raw回读→FailureObserved→prompt审计→claim→启动、写读屏障和错误码；验收：故障矩阵能唯一决定零启动/包不完整/发布不完整。
- [x] **W04 1200 行容量与测试闭集审阅**（REQ-WSC-33/34/35）：核对 `large_file_guard` 及新旧落点净增／腾挪账；验收：每个顶格文件有负增或净零办法，非 SC 不触碰。

## 二、后实现

### A4 证据与修复包

依赖顺序：W05→W06→W07；W12在W08/W09之前，W09依赖W06/W08；W10写侧与W11g读侧均完成后才能验收A4发布。W11a→W11b→W11c，其它消费切片按design共享文件顺序接入。A3必须在A4底座可用后接线；下方W编号是追踪身份，不用编号大小覆盖这些真依赖。
- [x] **W05 证据 DTO 与 ArtifactPayload 变体**（REQ-WSC-34）：新增 SC 修复包与 round evidence 类型、serde 版本、available/missing 状态；同步 Rust 穷尽匹配。验收：untagged/flatten 顶层键为 `sc_failure_repair_package`，旧 artifact 往返不变。
- [x] **W06 逐轮 evidence store**（REQ-WSC-33/34）：新增 session-scoped append JSONL、prompt immutable text、RoundBound/FailureObserved/PromptUnavailable/ChainTerminal 事件及严格回读；验收：同键同文幂等、坏行/跨scope/断号显式失败。
- [x] **W07 SC split run identity**（REQ-WSC-33）：只在 SC 原轮与教学轮统一分配并返回真实 handle／role_run_seq；接入 raw structured_output 回读。验收：两轮一一绑定；非 SC legacy 无新身份且行为不变。
- [x] **W12 A4 结构失败透传**（REQ-WSC-33/34）：使 sealed `PlanningCompileFailure` 与 source ref 随 Rejected 返回，结构 prompt/组包读取失败自身 ref；验收：双 revision fixture 永不使用 last-valid source。
- [x] **W08 失败原因全量投影**（REQ-WSC-33）：在 compile、IR 预检、结构门三个决定点采集未截断 blocking_reasons；保留现有 candidate-attempts 行、failure_id 与统计身份。验收：历轮完整数组可回读，不能只依赖 first_round_failure 或固定结构文案。
- [x] **W09 A4 终局组包**（REQ-WSC-34）：在三类终局及教学中断出口组装 round 顺序、末轮 raw、prompt refs/digest、失败 source hash/bytes、missing/incomplete；不改成功/Routed/waiting/Superseded。验收：六类失败出口均有可读包，非终局零包。
- [x] **W10 A4 严格写读发布屏障**（REQ-WSC-34）：同一raw文件/锁内max+1与集中save保磁盘A4；A4非current，文件写读核验→ref写读→receipt写读→广播。update_artifact改Result，锁/raw/分配/save错误在内存变更前上浮；按Plan7文件14生产/19测试闭集显式处理，void既有Error+Failed节点return、PlanRepair真实Store(Io)桥接。验收：pending夹mapping旧表回写/普通update/mark/CAS仍保version/created_at、分配无撞号、失败无成功事件、同包重放零新增provider；无高域overlay。
- [x] **W11a A4 wire类型镜像**（REQ-WSC-34）：新增TS DTO并扩归一化union、收敛workspace.ts重复联合；验收：旧wire全保留，新键类型可解析，1198行文件净负增。
- [x] **W11b A4 三入口归一化**（REQ-WSC-34）：同步WS消息、session_state/store与REST历史版本识别；验收：同一包在三入口deep-equal，未进入markdown分支。
- [x] **W11c A4 只读摘要与JSON入口**（REQ-WSC-34）：补summary、label严格switch和默认JSON tab；验收：显示真实complete/incomplete摘要与全文JSON，不新增恢复按钮。
- [x] **W11d A4 服务端摘要与重连白名单**（REQ-WSC-34）：迁出summary helper并新增A4分支、session_state白名单；验收：旧变体summary不变，A4重连可见且session_state.rs净负增。
- [x] **W11e A4 测试穷尽匹配腾挪**（REQ-WSC-34）：迁出tests_policy_routing.rs的artifact_variant helper并补新变体；验收：原路由快照不变，1200行宿主净负增。
- [x] **W11f A4 前端消费契约测试**（REQ-WSC-34）：覆盖三入口、新summary/label/default JSON与旧变体；验收：新键不丢失、不崩溃，旧fixture无回归。
- [x] **W11g A4 待发布版本读屏障**（REQ-WSC-34）：两store公读保持raw，engine恢复/内部写者不丢pending；receipt/digest过滤仅进session_state/公开REST最终视图，live只在publisher成功后广播；mark_latest跳A4。验收：无receipt不公开呈现、坏receipt显式错、普通current/metadata不变；内部socket business-ref stale校验保持raw。

### A3 完整 source 教学

- [x] **W13 source resolver 与 ref 消费**（REQ-WSC-35）：compile/IR 从绑定 split `structured_output.json` 恢复并归一化，结构从 sealed source revision ref 读取；核验 scope/hash/bytes；历史 None 显式缺项。验收：来源失败零启动且错误可见。
- [x] **W14 source block renderer**（REQ-WSC-35）：复用 `[previous_source]` 语义，支持最长反引号+1围栏、首尾空白与 UTF-8 无损，附 ref/hash/bytes/origin。验收：从 prompt 提取正文逐字等于 source。
- [x] **W15 compile/IR prompt 接口接线**（REQ-WSC-35）：扩展 `TeachingReredriveFacts`、compile builder 和 IR facade，source 块放失败事实与输出指令之间；Result 错误显式上浮。验收：两类最终 prompt 均自含完整 source，无无source生产旁路。
- [x] **W16 structure prompt 接线**（REQ-WSC-35）：扩展 `author_structure_repair_prompt` 使用同一 source block renderer，保留环选项/保护项/REVISION_TEACHING。验收：结构 prompt source 来自 failure ref，source move 不丢正文。
- [x] **W17 final prompt hard budget**（REQ-WSC-35）：最终装配后以既有 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES=262144` 按 UTF-8 bytes 检查；IR 尾段后复检；超限拒绝、不截断、不claim、不启动并审计 rejected_budget。验收：262144 通过、262145 拒绝。
- [x] **W18 prompt evidence 与教学链顺序**（REQ-WSC-33/35）：prompt 正文先不可变写读，新窗口prompt_ref填真实正文locator而非builder名，后claim/启动；source缺项、审计写读失败、预算失败进入missing与A4包。验收：故障注入零新增调用，错误带轮次，ref正文与digest一致。

## 三、再门禁

- [x] **W19 定向行为矩阵**（REQ-WSC-33/34/35）：补齐 split轮次、三类失败、raw/source绑定、完整 blocking reasons、strict artifact barrier、prompt边界、非终局与 legacy 负面场景；验收：每个 spec 场景有可执行测试映射。
- [x] **W20 全量契约与仓库门禁**（REQ-WSC-33/34/35）：真实运行 `openspec validate deliver-sc-failure-repair-package --strict` 与 `openspec validate --specs --strict`，再运行受影响 Rust/前端与 `large_file_guard`；验收：两条 OpenSpec 命令退出码 0，输出归档到交付说明。

## 验收记录（2026-11-14，真实执行输出）

| 门禁 | 命令 | 结果 |
|---|---|---|
| 契约（change） | `openspec validate deliver-sc-failure-repair-package --strict` | exit 0，`is valid` |
| 契约（specs） | `openspec validate --specs --strict` | **81 passed / 0 failed** |
| 格式 | `cargo fmt --all -- --check` | exit 0 |
| 静态检查 | `cargo clippy --all-targets --all-features --locked -- -D warnings` | 无输出（0 violation） |
| 全量单测 | `cargo test --locked --lib` | **6595 passed / 0 failed / 7 ignored** |
| 行数门禁 | `cargo test --locked --test it_core large_file_guard` | **2 passed / 0 failed**（5 个顶格文件已按职责搬移） |
| SC 失败面子集 | `cargo test --locked --lib sc_failure` | 89 passed / 0 failed |
| 教学面子集 | `cargo test --locked --lib sc_teaching` | 35 passed / 0 failed |
| 前端 | `tsc -b && vite build`（`/tmp/sc-failure-a4a3-web`） | exit 0；消费契约测试 40 passed |

接口级真实验证（隔离实例 + 真实 publisher 落盘，详见 `cadence/reports/2026-11-14_进度报告_SC失败修复包Phase3Phase4验收_v1.0.md`）：
`health 200`；`artifact-versions/1` markdown `is_current=true`；`/2` 为 `sc_failure_repair_package`（`is_current=false`、`complete`、1 轮）；`/3` 为 `incomplete`（`missing=[teaching_prompt]`、`disposition=unavailable`）。

**W11c 说明（诚实边界）**：组件级渲染与 REST 读取已验（含默认 JSON 视图、历史版本切换）；**页面级真实浏览器截图仍受阻**——合成夹具缺「成员仓 + 真实源码 + aggregate index」这一整段 LC bootstrap，产品对无成员 LC 正确拒绝（422 `aggregate_index_member_coverage_failed`），不是产品缺陷。
