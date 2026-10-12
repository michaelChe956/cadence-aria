# author/reviewer 启动臂接入 TrustMissingGuidance 结构化帧

- 日期：2026-10-12
- 关联：trust 卡简化 a38f1b46（`cadence/reports/trust-card-simplify.md`）、分类桥下沉 e2c15c78（T6R2）
- 状态：已落地（TDD 红先→绿→回归全绿）

## 背景与缺口

a38f1b46 把 trust 缺失指引卡简化为「两行说明＋可复制命令窗」，并新增 wire 帧
`WsOutMessage::TrustMissingGuidance { providers, canonical_root, reason_code,
retry_action, command_markdown }`（前端 store/投影/卡面/复制已全支持）。但发送面
只覆盖 **SC admission waiting 路径**（`single_candidate.rs` → `provider_run.rs`
的 `AdmissionWaiting` 分支）；author/reviewer 启动臂（分类桥
`classify_launch_trust_failure_details` 的消费者）错误仍只有平铺文本——owner 在
多仓 story 生成现场撞到旧样式卡（前端被迫走平铺文本识别回落，无命令窗结构化
数据）。

## 改动面（9 文件）

- **分类桥结构化变体**（`provider_gateway.rs`）：新增
  `LaunchTrustFailureClassification { details, trust_waiting }` 与
  `classify_launch_trust_failure()`——平铺 details 与同源等待投影一次产出；
  `classify_launch_trust_failure_details()` 改为薄委托（判据/文案单一事实源，
  既有 4 消费位签名零变化）。
- **引擎事件**（`types.rs` + `mapping.rs`）：新增
  `EngineEvent::TrustMissingGuidance { waiting }`，`map_engine_event` 映射为
  `WsOutMessage::trust_missing_guidance_from_waiting`（生产 socket 转发与测试
  forward 任务同链路）。
- **author 臂**（`provider_drive/author_root_launch.inc.rs`）：Story 通用 Author
  链（StartGeneration → Author run → `drive_author_provider_session_via_gateway`）
  在分类桥输出处捕获 `trust_waiting`，驱动收尾（内部已发
  `EngineEvent::Error`）后经新增引擎助手 `emit_launch_trust_guidance` 发出——
  保证 error 帧先清旧指导、指导帧紧随重建（与 SC 路径同 pattern）。
- **reviewer/revision 臂**（`review/drive.rs`）：
  `map_gateway_error_to_adapter_with_trust` 返回 `(mapped, trust_waiting)` 对；
  `start_review_session_via_gateway` 返回 `(session, trust_waiting)`（启动成功恒
  None）。reviewer 首发 `Failed` 分支在 `finish_review_provider_run_failure`
  （error 事件）之后发出；author revision LC 位点在 `drive_provider_session`
  收尾后发出。修复轮/静默重试轮解包丢弃（首发成功后 trust 恒已通过）。
- **测试**：`drive_tests.rs` 两用例适配新返回形并补结构化断言（trust 分型携带
  waiting＋命令；无 trust 面恒 None）。

## TDD 证据（红→绿）

新增三用例（`web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`）：

1. `story_generation_trust_missing_emits_structured_guidance_frame`——多仓
   Story Author 启动、Kimi entry 缺失：error 帧保留 `provider_trust_missing`
   稳定判别码，紧随 `trust_missing_guidance` 帧（providers=`["kimi_code"]`、
   canonical_root=冻结权威根、reason_code=`trust_entry_missing`、
   command_markdown=同源 mkdir 单行 shell）。**红**（2s 超时无帧）→**绿**。
2. `reviewer_launch_trust_missing_emits_structured_guidance_frame`——reviewer
   臂同型（Codex entry 缺失：providers=`["codex"]`、command_markdown=同源 TOML
   片段含 config.toml 路径）。**红**→**绿**。（测试勘误：初版在断言前误调
   `forward.abort()` 关闭出站通道，属测试 bug，非产品缺陷。）
3. `story_generation_other_unavailable_emits_no_guidance_frame`——负例：Codex
   trust 已预登记＋availability gate 不健康 → 错误不带
   `provider_trust_missing`，有界窗口内零 guidance 帧（既有守恒，实现前后均
   绿）。注：负例前提必须是「已登记＋不健康」——entry 缺失时 trust 复验先于
   availability 拒绝，分类为 trust 是正确行为。

回归：`workspace_ws_handler` + `product::workspace_engine` + `logical_codebase`
+ trust 族共 **1534 passed / 0 failed**（含 review 臂
`reviewer_gateway_*`/`drive_review_session_via_gateway_*` 与分类桥既有直测）。

## 兼容性

- 平铺 error message 与 `manual_guidance` 原样保留（旧客户端零感知；帧顺序保证
  「新 error 清旧指导」配对纪律）。
- 非 trust 错误（availability/policy/target 漂移、无 trust 面 provider）零结构化
  帧发出（`trust_waiting=None`）。
- Coding 共享启动口（`provider_stream/launch.rs`）与 WIP plan author 家族
（`gateway_start.rs`）仍走平铺面——前端旧文本识别回落保留覆盖；后续按需同型
  接线即可（桥已备好结构化输出）。
