# Stage 4 C3-T2 + codegraph pin k3 审查报告（合并一轮）

> 审查人：C3T2K3 · 2026-09-19 · 工作树 `.worktrees/feat-b-0808-add-monorepo`
> 对象：`7e64abc8`（C3-T2 typed abandon 重承载，16 文件）+ `ff4d8b92`（codegraph pin 1.5.0→1.6.0，11 文件）
> 结论：**两 commit 均 PASS，0 findings（P0-P3 计 0）**

## 一、7e64abc8（C3-T2）：PASS

| 审查项 | 判定 | 证据 |
|---|---|---|
| 白名单双处 | ✓ | protocol.rs `is_message_valid_for_stage_with_flow` SC HumanConfirm 分支显式 `+\| WsInMessage::AbandonHumanGate { .. }`（:151 区）；`message_type` 补 `"abandon_human_gate"`（:282 区）。计划 v1.1 双审修订（k3 P2）明确要求两处，均已落地并被 3 条测试钉住（whitelist unit 正反放行、roundtrip wire 名、wire-boundary 负向） |
| HumanGateCloseDecision 接线 | ✓ | conversational_gate.rs 新枚举（与 legacy 枚举零共用）+4 处签名收敛（termination/close/两 translate）；inbound 三路：`Confirm` SC 分流→Approve、新 `AbandonHumanGate` 臂→Abandon、`HumanConfirm` SC 桥接映射式（Confirm→Approve/Terminate→Abandon/RequestChange 显式拒绝） |
| 幂等键族 | ✓ | in-arm `validate_command_id` 与 HumanGateFeedback/Advance 同款同门（空白→`INVALID_COMMAND_ID`，测试钉死）；关门幂等由 durable CAS 单飞承载（先到者赢+迟到者翻译），wp2-l0 §4.2 明示「不另设 per-command 记录」——文档化决策，非缺陷 |
| 双轨零删除 | ✓ | `HumanConfirmDecision` 枚举三变体原样（in_.rs:165-171）；legacy 流路由（HumanConfirm else 分支、RequestRevision→RequestChange 桥）一字不动；非 SC 白名单分支不动；`handle_human_confirm_from_handler` 不动 |
| RequestChange 文本归一 | ✓ | 引擎旧 RequestChange 臂错误文本逐字上移 inbound（`single-candidate human gate does not support request-change; submit feedback through HumanGateFeedback`——两处字面量比对一致）；engine 类型面不可再表达=结构性拒绝。wire 可达路径行为不变：门开启 stage 拒为 STAGE_INVALID（旧白名单本就只放行 Terminate 模式）+错误 stage 拒为 socket 层 INVALID_MESSAGE_FOR_STAGE，均为既有行为；唯一文本差异在测试专用直呼 inbound 的错误 stage 角落，报告 Concerns 已披露且通道/非关门行为不变 |
| Box future 装箱 | ✓ | 整体 `Box::pin` 私有 inner，公开签名 `Pin<Box<dyn Future<Output=()> + Send>>`；唯一调用点 socket.rs:758 `.await` 兼容、零调用面改动；动机三点定性（基线 PASS/改动必现 SIGABRT/16MB 栈 PASS）留档，语义零变化 |
| 红绿真实性 | ✓（实跑复核） | 编译红 7 errors 与 E0433×3（HumanGateCloseDecision 未定义）+E0599×4（变体未定义）形态自洽；Stage A 运行红形态与 through-dispatch 测试 panic 锚逐字对应（白名单去项即真实链路 STAGE_INVALID，非 mock 断言）；**绿面本机实跑 6/6**：abandon_human_gate 族 3/3 + legacy_request_change/whitelist/roundtrip 3/3 全过 |
| 越界（16 文件 vs 计划） | ✓ 无行为越界 | 计划 Modify 4+Create 1+v1.1 补 protocol.rs 全中；`workspace_engine/mod.rs`（re-export）与 `coding_workspace_engine/tests/campaign_stage3_amendment.rs`（2 行构造面）为类型收敛的编译必然连带；8 个测试文件属计划「就近既有测试文件扩展」；`stage4-c3-t2-report.md` 为 controller 面文档 |
| 消费面分发核查 | ✓ | 新 wire 变体跨界的全部消费点已读：socket.rs 外层 stage 校验（错误 stage→INVALID_MESSAGE_FOR_STAGE，fail-closed）、inbound 边界（门 stage→STAGE_INVALID）、`message_type`/白名单/`requires_stage_validation`（不豁免→必校验）、`single_candidate_generation_decision_error`（不涉）——无通配静默丢弃点 |

不构成 finding 的已核事项（均为报告披露的显式决策）：command_id 不落 per-command 审计记录（幂等由 CAS 承载，§4.2）；AbandonHumanGate 调用点内层 Box::pin 与整体装箱并存（双层堆分配，栈缓解，正确性无涉）；campaign 探针 300ms 窗口（测试鲁棒性设计，失败形态仍会以 loop 超时暴露）。

## 二、ff4d8b92（codegraph pin）：PASS

| 审查项 | 判定 | 证据 |
|---|---|---|
| pin 双处 | ✓ | codegraph_cli.rs:16 `CODEGRAPH_EXACT_VERSION="1.6.0"`；`verify_v1_5_0`→`verify_version` 版本中立更名，operation.rs:310 调用点同步 |
| clean cutover 无 1.5 残留 | ✓ | 全库 grep：`verify_v1_5_0` 零残留；src/tests 零功能性 `1.5.0`（唯一残留为 evidence_index/tests/query_hit.rs:5 历史注释「实测过程（真实 CLI，v1.5.0）」——fixture 产源记载而非 pin，模块外，非功能残留） |
| fixture 同步 | ✓ | 11 文件逐 hunk 核毕：5 处 scripted `--version` 回显（freshness/operation_tests/web_lc_operations_api/planning/planning_p0）、cli 自带 3 fixture（含 mismatch 样本 1.6.1、status JSON）、4 处 snapshot `codegraph_version` 字面量（types/evidence_mediator_tests/evidence_query part_01） |
| denylist 契约不变 | ✓ | exclude.rs 仅注释行 v1.5.0→v1.6.0，`BUILTIN_EXCLUDES` 数组零改动；提交信息已核 1.6.0 init/sync/index 仍无 allowlist 选项 |
| 2 例红转绿采信 | ✓（实跑复核） | 本机 `codegraph --version`=1.6.0；实跑 `aggregate_index_rebuild_endpoint_returns_active_projection`+`stale_story_planning_read_syncs_index_and_returns_normal_response` 2/2 绿（真实二进制，3.85s）——红→绿声明与本机现状一致 |

## 三、总判定

- 7e64abc8：**PASS**（0 findings）
- ff4d8b92：**PASS**（0 findings）
- 复核手段：逐 hunk diff、commit 时刻文件快照、计划 v1.0/v1.1 对照、全部 WsInMessage 消费点分发核查、6+2 条关键测试本机实跑全绿。
