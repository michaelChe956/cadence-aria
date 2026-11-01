# Wave4 F-24/F-23 修复报告：choice 卡可靠重投面 + 跨重启僵尸 run 恢复

- **缺陷**：F-24（choice 卡不送达用户——0459/0482/0483/0484 同根楔死）+ F-23（跨重启僵尸 run 中止无效）
- **提交**：`a1a479a1`（8 文件，+566/−32）

## 0. 勘误（对排查方向的前提修正）

任务给的三断点候选（①engine 广播缺口 ②前端 handler 缺 case ③cockpit 缺组件）经核**全部已存在且无缺口**：
provider→`EngineEvent::ChoiceRequest`→session router `broadcast`（含 journal+event_seq）→前端
`workspace-ws-message-handler.ts` choice_request case→`ChoiceRequestEntry` 渲染→cockpit
`onChoiceResponse`→`choice_response` 入站。F-22 报告 §遗留 1 的判断在现架构下需要重新定位。

**真实断点在投递可靠性层**（见 §1），且 F-23 的引擎恢复 API 早已存在但**零调用方**（死代码）。

## 1. F-24 根因：挂起 choice 是「单次 try_send、无重投」的孤儿帧

0484 标本（timeline_node_004）：`provider_choice_wait_timeout … pending=["af94892a…"]` 自证引擎收到了
ChoiceRequest 并挂起等待界；用户 900s 全程在线却没看到卡。链路解剖（现架构 workspace_session）：

1. **degraded 丢弃无重投**：router 对每连接 `try_send`（设计上不允许 router 等待）。连接出站队列
   （cap 64）满（典型：浏览器后台冻结/TCP 零窗口）→ choice 帧被丢弃、连接标 degraded。恢复只补
   `session_state` 基线——**基线不含 choice**；且 choice 悬置期 provider 静默、无后续广播触发恢复。
   连接全程在线 → 永不重连 → journal 回放永远不发生 → **卡永久丢失**（0484 精确形态）。
2. **挂起 choice 不随 attach/重订阅恢复**：`pending_author_choice_request_message` 只覆盖 TextFallback；
   provider choice 依赖 journal 窗口（截断/快照分支即丢）。

### 修法（三重投面，帧随 run 生命周期）

- `ActiveRun.pending_choices: Arc<Mutex<Vec<WsOutMessage>>>`——完整 wire 帧登记（到达注册、应答移除、
  run 终态随 run 消亡，无 stale 卡复活面）；router 与 legacy 转发（mapping.rs）双注册。
- **degraded 恢复补发**：恢复成功后在基线之外以**独立等待任务** `send().await` 补发挂起帧——队列腾出
  即送达、不依赖后续广播、绝不阻塞 router；多次恢复的重复帧由前端 upsert 幂等吸收。
- **attach 初帧 + cursor 重订阅三分支**（Replay/ActiveRunWindow/Snapshot）统一补发，已回放同 id 去重。
- inbound stale 校验等价迁移到帧 id 移除（`CHOICE_ID_UNMATCHED` 语义与既有测试不变）。

## 2. F-23 根因：僵尸态的唯一收口 API 是死代码

0482 形态：v28→v29 切换（进程重启）时 author run 在途 → durable 会话停 `running`+active 节点。
引擎侧 `recover_stale_active_run_after_disconnect()`（落 AbortedByDisconnect 终态+整流回
prepare_context，正是为「进程重启后的 stale run 恢复链」写的）**没有任何调用方**。manager 创建
（每 session 单例、惰性）只恢复 human gate turns 与 WorkItemPlan outline；story/design 的
running 僵尸无人触达——abort 消息落在 `abort_active_run()==false` 分支即蒸发（无处落地），
UI 永久 running（0482 三小时）。

### 修法

`recover_on_creation` 末臂 `recover_stale_run_if_zombie`：恢复臂之后仍 `Running/CrossReview/Revision`
且无活跃 run 的 durable 会话＝进程重启孤儿（registry 单例契约保证创建时无幸存 run），调用既有引擎
API 收口 + 广播快照。outline/human-gate 可恢复的 Running 不受影响（它们先 spawn run，守卫跳过）。
abort 不再需要专属落地：首次 attach 即解楔，retry_interrupted_run 恢复面（依赖 AbortedByDisconnect
节点）自此可达。

## 3. TDD 证据（红→绿）

| 测试 | 红 | 绿 |
|---|---|---|
| `degraded_attachment_recovery_redelivers_pending_provider_choice`（F-24 主断点） | 补发断言失败（只收基线无卡） | ✔ |
| `attach_and_cursor_resubscribe_redeliver_pending_provider_choices` | 初帧/快照重订阅无卡 | ✔ |
| `manager_creation_recovers_stale_running_session_after_process_restart`（F-23，durable 僵尸 fixture） | stage 停 `Running` | ✔ prepare_context + 节点 Failed + AbortedByDisconnect 标记 |
| `workspace_ws_second_connection_receives_and_answers_pending_choice`（集成，0484 现场形态：次连接收卡→代答→run 完成） | —（回归钉） | ✔ 1.97s |

红为桩态行为红（`register_pending_choice_frame` 先以 no-op 桩落盘跑出三红，再实现转绿）。
过程修复两处自伤：state 锁重入死锁（pending_choice_frames 在持锁段内调用）、测试断言与引擎
active_node_id 契约对齐（失败路径保留指向失败节点，同 `finish_active_run_with_failed_node`）。

## 4. 回归与验证

- `cargo test --lib`：**3425 passed / 0 failed**（3 ignored 既有）。
- `cargo test --test it_core workspace_ws_integration`：**45 passed / 0 failed**（44 既有 + 1 新）。
- `cargo clippy --lib --tests`：零告警；`cargo fmt` 归一后提交。
- 前端零改动：choice 渲染/应答链已存在，服务端帧到达即闭环（次连接集成测试证明）。

## 5. 变更面

- `src/web/workspace_session/manager.rs`：ActiveRun 挂起帧存储+注册/移除/读取 API；初帧与重订阅补发；
  `recover_stale_run_if_zombie` 创建期恢复臂；`choice_frame_id`/`choice_ids_in_frames` 辅助。
- `src/web/workspace_session/router.rs`：choice 注册先行于广播；degraded 恢复等待式补发任务；
  `broadcast_current_session_state`。
- `src/web/workspace_ws_handler/mapping.rs`：legacy 转发注册等价迁移。
- `src/web/workspace_ws_handler/decisions/inbound.rs`：stale 校验帧化。
- 测试：`workspace_session/tests.rs` +3；`it_core/part_03.rs` +1；ActiveRun 构造点 ×6 适配。

## 6. 遗留（登记备查）

1. v29 在跑服务器（PID 3218062）仍是旧二进制——本修需重编译重启后对 0484 形态做实况复验；
   重启动作本身即 F-23 恢复面的天然演练。
2. choice 悬置等待界 900s（F-22）保留为兜底：卡片送达后无人应答仍会在 15min 转可诊断失败。
