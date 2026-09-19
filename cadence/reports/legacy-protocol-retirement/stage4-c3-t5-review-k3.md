# 阶段 4 C3 · T5 k3 审查报告（d4c39752..c73a1269，10 commits）

- **审查人**：k3（独立子代理）；worktree `feat-b-0808-add-monorepo`，HEAD=c73a1269。
- **结论**：**FAIL（有条件）**——删除面本体正确、守卫完整、残留归零复验通过；但 preflight 收敛存在一处 REQ-WSC-08 条款一致性缺口（P2）+ 3 条记录级 P3。核心删除语义可合并，P2 建议下一周期修复。

## 1. 归属抽验（实读消费链，14+ 符号，超 ≥6 要求）

| 符号 | 归属表判定 | 复核结果 |
|---|---|---|
| HumanConfirm+HumanConfirmDecision（必删） | 删 | ✅ `handle_human_confirm` 零定义；wire parse 拒收→`LEGACY_MESSAGE_RETIRED`（protocol.rs:26-48+socket.rs:584-590 实读）；part_07 through-socket 真实拒收断言（10 wire 名逐个：code+received+stage 上下文+durable 消息零新增+stage 不漂移+连接存活） |
| SelectWorkItemGenerationMode/WorkItemDraftDecision/WorkItemBatchDecision/ReviewDecisionResponse/SelectRevisionPath（必删） | 删 | ✅ 9 个决策 handler 零定义（`fn handle_review_decision`/`handle_work_item_draft_decision`/`handle_work_item_batch_decision`/`select_work_item_generation_mode`/`map_revision_path`/`skip_work_item_plan_optional_findings` 全仓零命中）；`map_revision_path` 确认随删 |
| AuthorDecision+枚举 | 删 | ✅ `handle_author_decision`/`handle_work_item_plan_outline_decision` 零定义；`AuthorDecisionOutcome` 整型删除（types.rs diff 实证） |
| RequestOutlineRevision | 删 | ✅ `request_work_item_plan_outline_revision` 零定义 |
| SaveHumanPresentationRevision | 删 | ✅ engine fn 零定义；出站 `HumanPresentationRevisionSaved/SaveFailed` 变体删除（out.rs diff 实证） |
| RevertWorkItem | 删 | ✅ `apply_revert_mark` 零定义；前端类型归零 |
| **RequestRevision（保留）** | 仅 WorkItemPlan | ✅ inbound.rs:525-633：plan 分支五Outcome 全链 spawn 保留；非 plan→`REQUEST_REVISION_WORKSPACE_INVALID` protocol error；socket.rs AuthorConfirm+WorkItemPlan 特例放行（:715-721）；`request_work_item_plan_revision` 仅返 OutlineRevision/StartRevision 两分支 |
| **Confirm（保留+改直连）** | SC 门/非 SC 直连 | ✅ inbound.rs:189-204：SC→human gate Approve；非 SC→`handle_confirm_from_handler`→engine `handle_confirm`；错误码 `INVALID_HUMAN_CONFIRM_ACTION` 保留（decisions.rs:100），前端 GATE_REJECTION_CODES 依赖实证（cockpit-action-routing.ts:77） |
| **amendment 三族（保留）** | 保留 | ✅ inbound.rs:635-677 三臂完整 |
| **WorkItemPlanCompileRecoveryAction（保留）** | 保留 | ✅ inbound.rs:356 路由+protocol.rs HumanConfirm(Legacy) 白名单+前端 union 保留 |
| ContextBlocker 判定 (b) | 决策面删/状态机保留 | ✅ `handle_work_item_plan_context_blocker_decision`+`append_*_resolution`+`human_confirm_payload_*` 零定义；`enter_work_item_plan_context_blocker` 3 调用点存续（authoring.rs:186/221/291，legacy 大纲完成路径）✅ 与归属表一致 |
| draft/batch 内部函数 | 仅删 wire handler | ✅ accept/rewrite 内部族保留；`begin_work_item_batch_review_run` `#[cfg(test)]` 保留实证（runs.rs:299-301） |

## 2. SC 改道两处守卫（正确性）

- review/routing.rs:659-680 `_` 臂：`ReviewGate::RequiresRevision if self.is_single_candidate_plan()`（:612，type+flow_kind 双条件）→`enter_human_confirm`；非 SC 保留 `enter_review_decision`（在途限制）✅；F5-B repeated_findings 闸门共存。
- plan_repair_review.rs:222-265：`route_to_human_gate` 同款双条件；SC 下缺结构化 review 与全部非 pass 分支均落 human gate；非 SC RequiresRevision 族保留 review_decision ✅。
- 其余 `enter_review_decision` 调用点（routing.rs:768/778/824/906=outline/draft/batch staged 臂）实读确认仅 legacy 节点类型可达（SC 无 staged 节点；plan-repair PlanReview 阶段被首臂拦截改道）——与归属表「SC 不触达」结论一致。
- protocol.rs:146 `ReviewDecision => false`+88e3dc20 三处重钉（首轮/异指纹/advisory-only→HumanConfirm）实证。

## 3. preflight 与 REQ-WSC-08 一致性

- **flag on 路径 ✅**：`Ineligible`→durable Failed（`mark_single_candidate_prepare_failure`：system message+status Failed）+`SINGLE_CANDIDATE_PREFLIGHT_FAILED`；`flow_kind` 恒 SingleCandidate；负向测试（0/2 仓）断言终态+原因+零 SC 工件落盘。LegacyFallback 变体已删。
- **⚠️ flag off 路径（P2 finding 1）**：见 findings——CLI 默认 off+README 无 flag 启动态下，多仓/零仓 logical Issue 跳过 preflight → prepare 200+SC 会话创建 → 生成启动时 `TargetAmbiguous`/`TargetMissing`（workspace_repository.rs:256-271）报错，而非条款要求的 prepare 期 durable 终态；patch 前 flag off=Legacy 流（多仓可用）。重钉测试注释「flag 仅随会话持久化作历史记录」与存活 gating 自相矛盾。未入 6 条已知偏差。

## 4. 残留终版口径（独立复验）

- 9 符号+10 wire 名精确口径脚本**独立重跑=全零**（豁免 part_07+protocol.rs 白名单）✅。
- 口径合理性：`\b` 边界正确排除 `WorkItemBatchDecisionOutcome`（保留枚举）；豁免面三处均为功能本体/归档非残留；`node_type/stage: "human_confirm"` 8+298 只读串=timeline 节点类型/stage 枚举历史值（服务端 `WorkspaceStage::HumanConfirm` 未改名，前端 stage 判断属 typed 门呈现）——豁免成立。
- 前端 union 复验：`WsInMessage` 20 分支与 in_.rs 19 变体一一对应（hello/ping 含），10 退役名零残留；发送器零残留（测试内 `sendHumanConfirm` 为 `sendConfirmGate` 局部 mock 变量名，非 wire 面）。

## 5. 退役依据与非放松边界

- 依据实证：`wp1-gate-retest/evidence-matrix.md` §1 表「legacy 路径回归全绿」行（全量 cargo test 10 结果块，lib 3370/it_web 408/it_core 175 全 0 failed）在案 ✅。
- 边界抽验：红名单 31 测逐名核对断言链均落已删面（如 `story_design_work_item_plan_recovery_consistency` 的 workitem plan 腿=staged author 管线等待 outline artifact+author_confirm——SC 流下不再成立；author/serial 名义锚 start_generation 实为 staged 断言链）；保留面测试齐备（part_07 负向/preflight 三重钉/SC gate 重钉/protocol 单测）。**退役非放松成立**。备注：story/design 恢复一致性读侧覆盖随混合测共退役，属 REQ-RET-03 限制族可接受（未单列登记）。

## 6. 六条已知偏差评估

1. WorkItemGenerationModeDto→artifact.rs：✅ 合理（历史 outline candidate 载荷反序列化值类型+SC 内部诊断；wire 面零）。
2. Confirm 直连+INVALID_HUMAN_CONFIRM_ACTION：✅ 实证合理。
3. it_core choice/reconnect 13 测：承接面存在（campaign_stage3 恢复矩阵+disconnect-strategy e2e）✅。
4. 孤儿面+cfg(test) 保留：✅ 实证。
5. story/design 决策通道终止+HTTP confirm 通道：✅（workspace_session confirm handler 在测）。
6. flaky 登记：静态不可复判，如实登记形态合规。

## 7. 越界检查

10 commits 文件面=引擎/ws handler/ws types/lifecycle preflight/前端 28 文件/测试/报告目录——全部落于计划 Task 5 与 D2/D3/D5 删除面；openspec/web e2e/配置零触碰；无越界项。

## Findings（4 条）

### F1（P2，confidence 0.78）rollout flag-off 跳过单仓 preflight，多仓 Issue 绕过 REQ-WSC-08 终态收敛
`src/web/handlers/lifecycle.rs:662-707`。`if rollout_snapshot` 门控下，flag off（CLI 默认，cli.rs:229-231 opt-in；README:141/256/451 启动命令均无该 flag）时 Logical 多仓/零仓 Issue 不评估 preflight：prepare 返回 200 并创建 SingleCandidate 会话，随后生成启动在 `workspace_repository_for_session`→`unique_target`（workspace_repository.rs:256-271）以 TargetAmbiguous/TargetMissing 失败——而非 REQ-WSC-08「多仓确定性 preflight 失败 SHALL 收敛新路径 durable 终态（含原因）」的 prepare 期终态。patch 前该配置走 Legacy 流（多仓可用），属本批引入的行为回退+条款适用空洞；重钉测试注释「flag 仅随会话持久化作历史记录」（lifecycle_tests.inc.rs:548-550）与存活 gating 矛盾。修复方向：去 gating（preflight 恒评估）或 flag 默认 on，并同步注释。

### F2（P3，confidence 0.9）residue 终版 it_web 终态数字自相矛盾
`wp5-residue-scan.md:113`：「328/0/10 ignored（退役前 328+31 failed+12 ignored，总数 371→338）」——同句「ignored legacy 11 测退役」下 12−11=1（非 10）、371−31−11=329（非 338）；T5 报告 :24 行与树实测（tests/it_web 现存 `#[ignore]` 属性恰 1 处：web_work_item_plan_revert.rs:98）均为 1 ignored/329 总。定稿断言文档的口径数字需更正为 1 ignored/329。

### F3（P3，confidence 0.85）报告 S3 归属措辞与树不符两处
`stage4-c3-t5-report.md:12`：「`ReviewDecisionOutcome::HumanConfirm` 等变体退役」——该变体仍存（types.rs:385，inbound.rs:611 仍在 match；实际退役的是 `WorkItemPlanOutlineRevisionSource::HumanConfirm` 变体与 `AuthorDecisionOutcome` 整型，types.rs diff 实证）。另归属表 §5「inputs.rs Default flow_kind 翻 SingleCandidate」与实现（保持 Legacy+注释，inputs.rs:229-234）相反——handoff v2 记录了最终决定但归属表/报告未回改。行为合规（生产唯一创建路径显式 SC），纯记录漂移。

### F4（P3，confidence 1.0）删除 case 残留不可达 `break;`
`web/src/hooks/workspace-ws-message-handler.ts:373-375`。两个 presentation case 体删除时保留了两条 `break;` 中的一条，退役注释后悬挂不可达语句（TS 不报错）。删除该行即可。

## 总评

删除面本体（wire/引擎/前端/负向测试/守卫改道/残留归零）**复核全部成立**，归属判定表采信度高；唯一实质缺陷为 preflight flag-off 分支的条款一致性（F1），以及三处记录级瑕疵。建议：F1 下一周期修复并补 flag-off 多仓负向测试；F2-F4 顺手更正。
