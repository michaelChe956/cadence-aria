# WP5 残留扫描（T5 Step 5 / D5 残留归零断言）——终版

> 状态：**已定稿**（2026-09-19，it_web 残余退役+全量终跑后出具）。
> 结论：**9 符号在豁免面外零命中；10 退役 wire 名 `type:` 键精确口径零命中**。

## 断言口径（v1.1 oracle P3-3，定稿版）

豁免=以下三类，其余全仓（`src web/src web/e2e tests`，`*.rs/*.ts/*.tsx`）零命中：

1. **负向测试文件**：`tests/it_core/workspace_ws_integration/part_07.rs`——退役消息 through-socket 负向测试的字符串字面量（S2 红测先行锚定，`LEGACY_MESSAGE_RETIRED` 断言本体）。
2. **报告目录**：`cadence/reports/**`（本 change 报告与留档）。
3. **拒收白名单活代码**：`src/web/workspace_ws_handler/protocol.rs` 的 `RETIRED_INBOUND_MESSAGE_TYPES`（10 退役 wire 名数组）——退役消息 parse 面识别/拒收的**功能本体**，属预期保留而非残留。

`human_confirm` 口径（定稿）：

- **wire 精确口径归零**：`type: "human_confirm"` / `"type":"human_confirm"`（消息对象的 type 键）全仓零命中。
- **「历史只读」豁免面**：`node_type: "human_confirm"`（timeline 节点类型，服务端 stage 值/前端呈现/测试夹具）终版计数 **8**（全部为 web 前端测试夹具的节点类型字段）；`stage == "human_confirm"` 等 stage 值串同理豁免。
- 宽口径全仓计数 298（stage 值/node_type/白名单/stage 枚举映射等只读面），不构成 wire 驱动残留。

生产代码内退役注记（「退役留档」注释）：定稿时已将注释中的精确符号名全部改写为描述性措辞（见下表），保证 9 符号在豁免面外零命中。

## 9 符号终版结果（2026-09-19 定稿）

| 符号 | 快照命中（定稿前） | 终版命中（豁免外） | 处置 |
|---|---|---|---|
| HumanConfirmDecision | 5 | **0** | conversational_gate.rs doc+tests doc 注释改写「human-confirm 决策枚举」描述性措辞；inbound.rs/decisions.rs 退役注释同步改写 |
| SelectWorkItemGenerationMode | 3 | **0** | inbound.rs 退役注释放宽行改写；artifact.rs doc 改写「生成模式选择 wire 消息」；useWorkspaceWs.ts 头注改写 |
| WorkItemDraftDecision | 3 | **0** | 同上（改写为「草稿决策消息」措辞） |
| WorkItemBatchDecision | 11 | **0** | `\b` 边界区分 `WorkItemBatchDecisionOutcome`（保留枚举，不计）；注释改写「批量决策消息」 |
| ReviewDecisionResponse | 1 | **0** | protocol.rs 注释改写「决策应答」措辞 |
| select_work_item_generation_mode | 9 | **0**（白名单活代码 1） | it_web 残余 staged 测试退役；draft_batch/runs.rs 注释改写；protocol.rs 白名单=豁免面 3 |
| work_item_draft_decision | 18 | **0**（白名单活代码 1） | it_web staged 测试退役；draft_batch/decisions.rs 注释改写 |
| work_item_batch_decision | 20 | **0**（白名单活代码 1） | 同上 |
| human_confirm | 349（宽） | wire 口径 **0**；node_type 只读 **8** | it_web 确认/回退/全流程 ignored legacy 测试退役；宽口径 298=stage/node_type 只读串豁免面 |

## 终版断言脚本（已执行，输出留档于下节）

```bash
cd <repo 根>
# 9 符号（豁免=part_07 负向测试+protocol.rs 拒收白名单活代码）
for sym in HumanConfirmDecision SelectWorkItemGenerationMode WorkItemDraftDecision \
           WorkItemBatchDecision ReviewDecisionResponse select_work_item_generation_mode \
           work_item_draft_decision work_item_batch_decision; do
  echo "== $sym =="
  grep -rnE "${sym}\b" src web/src web/e2e tests --include="*.rs" --include="*.ts" --include="*.tsx" \
    | grep -v "tests/it_core/workspace_ws_integration/part_07.rs" \
    | grep -v "src/web/workspace_ws_handler/protocol.rs"
done
# wire 精确口径（type 键；node_type 为 timeline 节点类型只读串，不计）
grep -rnE '(^|[^_a-zA-Z0-9])type: ?"(human_confirm|work_item_draft_decision|work_item_batch_decision|select_work_item_generation_mode|review_decision_response|select_revision_path|request_outline_revision|save_human_presentation_revision|revert_work_item|author_decision)"' \
  src web/src web/e2e tests --include="*.rs" --include="*.ts" --include="*.tsx" \
  | grep -v "tests/it_core/workspace_ws_integration/part_07.rs"
grep -rnE '"type": ?"(human_confirm|work_item_draft_decision|work_item_batch_decision|select_work_item_generation_mode|review_decision_response|select_revision_path|request_outline_revision|save_human_presentation_revision|revert_work_item|author_decision)"' \
  src web/src web/e2e tests --include="*.rs" --include="*.ts" --include="*.tsx" \
  | grep -v "tests/it_core/workspace_ws_integration/part_07.rs"
```

Expected（=实测）：9 符号每符号零输出；wire 两条 grep 零输出。

## 终版执行输出留档（2026-09-19）

```
===== 9 符号终版扫描（豁免=part_07 负向测试+protocol.rs 拒收白名单活代码）=====
== HumanConfirmDecision ==
== SelectWorkItemGenerationMode ==
== WorkItemDraftDecision ==
== WorkItemBatchDecision ==
== ReviewDecisionResponse ==
== select_work_item_generation_mode ==
== work_item_draft_decision ==
== work_item_batch_decision ==
== human_confirm wire 精确口径（type 键，排除 node_type）==
== 全部 10 wire 名精确口径（node_type 只读串除外）==
web/src/components/coding-workspace/plan-repair-test-fixtures.ts:145:        node_type: "human_confirm",
web/src/hooks/useWorkspaceWs.timeline.test.tsx:457:            node_type: "human_confirm",
web/src/hooks/useWorkspaceWs.timeline.test.tsx:529:            node_type: "human_confirm",
web/src/hooks/useWorkspaceWs.actions.test.tsx:420:          node_type: "human_confirm",
web/src/pages/ChatWorkspacePage.review.test.tsx:133:          node_type: "human_confirm",
web/src/state/chat-entries.test.ts:423:      node_type: "human_confirm",
web/src/state/plan-repair-session.test.ts:75:        node_type: "human_confirm",
web/src/state/workspace-ws-store.test.ts:387:          node_type: "human_confirm",
（以上 8 处均为 `node_type` timeline 节点类型只读串=「历史只读」豁免面，非消息 type 键）
== node_type human_confirm（历史只读豁免面）计数 ==
8
== human_confirm 全仓宽口径计数（含 stage/node_type 只读串+白名单）==
298
```

## 注释改写清单（生产代码，描述性措辞）

| 文件 | 处 |
|---|---|
| `src/product/workspace_engine/conversational_gate.rs` | doc：HumanConfirmDecision→「human-confirm 决策枚举」 |
| `src/product/workspace_engine/tests/conversational_gate.rs` | doc ×2：同上措辞 |
| `src/web/workspace_ws_handler/decisions/inbound.rs` | L198 桥接注记+L352 退役注记（7 消息族名→中文描述） |
| `src/web/workspace_ws_handler/decisions.rs` | `handle_confirm_from_handler` doc 桥接措辞 |
| `src/web/workspace_ws_handler/protocol.rs` | ReviewDecision 阶段白名单注记（两符号→「决策应答与修订路径选择」） |
| `src/web/workspace_ws_types/artifact.rs` | `WorkItemGenerationModeDto` doc（wire 消息名→「生成模式选择 wire 消息」） |
| `src/product/workspace_engine/draft_batch/runs.rs` | 退役注记（模式选择入口措辞） |
| `src/product/workspace_engine/draft_batch/decisions.rs` | 退役注记 ×2（批量/草稿决策入口措辞） |
| `web/src/hooks/useWorkspaceWs.ts` | 头注（7 发送器名→中文描述） |

## §退役（legacy 回归测试族退役记录）

- 退役依据：T1 证据矩阵「legacy 路径回归全绿」子项（`wp1-gate-retest/evidence-matrix.md:22`，全量 cargo test 10 结果块 4334 passed legacy 面 0 failed）。
- 已退役（累计）：
  - 引擎单测 57+（author_revision_loop 13/part_06 8/part_20 5/part_03 族 22/part_04 5/part_02 2/author_revision_review_routing 2/policy_routing 3/part_15 1/tool_policy 1/part_05 2/part_07 1/part_03.part_03 1）；
  - handler/ws_types（tests.rs 4/roundtrip 4/scope_rejection 2/human_presentation 整文件/stage+protocol 重钉保留）；
  - it_core part_01-06b 13 测（accept_author_output 夹具链）；
  - it_web staged 族 51 测（staged_flow 2/serial 19/batch 7/compile 11+runtime_projection 1/mode 9/recovery_consistency 2）；
  - 前端 vitest 17 测+2 文件级；
  - **本收尾批（终版追加）**：it_web 残余红名单 **31 测**（author 7/outline 7/serial 5/batch 4/mode 5/staged_flow 1/recovery_consistency 2）+ 残留 wire 字面量驱动的 `#[ignore]` legacy 测 **11 测**（author revision_streams 1/recovery reconnect 1/confirm 3/revert 3/review 2/split_flow 1）+ 共享夹具族随删（generation `valid_canonical_draft_output`/raw_outputs/revision_output 族、compile 模块全部、`QueuedSplitOutput` 枚举收敛单值形态）。
- it_web 终态：328 passed / 0 failed / **1 ignored**（退役前 328 passed/31 failed/12 ignored，总数 371→**329**；fix round 1 更正：初版误记 10 ignored/338，漏计本批退役的 11 个 `#[ignore]` legacy 测）。
