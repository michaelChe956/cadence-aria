# WP5 残留扫描（T5 Step 5 / D5 残留归零断言）

> 状态：**待定稿**——it_web 残余测试退役+全量终跑后出具终版。本文件登记快照与口径。

## 断言口径（v1.1 oracle P3-3：豁免=文件级）

- 豁免文件：`tests/it_core/workspace_ws_integration/part_07.rs`（退役消息负向测试的字符串字面量）、`cadence/reports/**`（本 change 报告目录）。
- `human_confirm`：stage/node_type 只读串（服务端 stage 值、timeline 节点类型、历史夹具呈现）=「历史只读」豁免面；断言采用 wire 形态精确口径 `type: "human_confirm"` / `"type":"human_confirm"` 归零。
- 生产代码内退役注记（「退役留档」注释）不以行内豁免计——定稿前将注释中的精确符号名改写为描述性措辞，保证 9 符号在豁免文件外零命中。

## 9 符号快照（2026-09-19，已滤 part_07；未滤生产注释）

| 符号 | 命中 | 构成 |
|---|---|---|
| HumanConfirmDecision | 5 | conversational_gate.rs 文档注释×1+tests/conversational_gate.rs ×2+退役注释×2 → 定稿时改写 |
| SelectWorkItemGenerationMode | 3 | 退役注释×3 → 改写 |
| WorkItemDraftDecision | 3 | 退役注释+`WorkItemDraftDecisionDto` 历史引用 → 改写/核实 |
| WorkItemBatchDecision | 11 | 多为 `WorkItemBatchDecisionOutcome`（保留枚举）后缀误中+退役注释 → `\b` 边界定稿 |
| ReviewDecisionResponse | 1 | protocol.rs 退役注释 → 改写 |
| select_work_item_generation_mode | 9 | it_web 残余 staged 测试（待退役）+退役注释 |
| work_item_draft_decision | 18 | it_web 残余 staged 测试（待退役） |
| work_item_batch_decision | 20 | it_web 残余 staged 测试（待退役） |
| human_confirm | 349 | 绝大多数=stage/node_type 只读串（豁免）；wire 形态 `type: "human_confirm"` 待终版计数 |

## 终版断言脚本（接手人执行后留档输出）

```bash
cd <repo 根>
for sym in HumanConfirmDecision SelectWorkItemGenerationMode WorkItemDraftDecision WorkItemBatchDecision ReviewDecisionResponse; do
  echo "== $sym =="
  grep -rnE "${sym}\b" src web/src web/e2e tests --include="*.rs" --include="*.ts" --include="*.tsx" \
    | grep -v "tests/it_core/workspace_ws_integration/part_07.rs"
done
echo "== wire: human_confirm/draft/batch/mode =="
grep -rnE 'type: ?"(human_confirm|work_item_draft_decision|work_item_batch_decision|select_work_item_generation_mode|review_decision_response|select_revision_path|request_outline_revision|save_human_presentation_revision|revert_work_item|author_decision)"' \
  src web/src web/e2e tests --include="*.rs" --include="*.ts" --include="*.tsx" \
  | grep -v "tests/it_core/workspace_ws_integration/part_07.rs"
```

Expected: 每符号零输出（`WorkItemBatchDecisionOutcome` 等保留枚举经 `\b` 边界区分后不计）。

## §退役（legacy 回归测试族退役记录）

- 退役依据：T1 证据矩阵「legacy 路径回归全绿」子项（`wp1-gate-retest/evidence-matrix.md:22`，全量 cargo test 10 结果块 4334 passed legacy 面 0 failed）。
- 已退役：引擎单测 57+（author_revision_loop 13/part_06 8/part_20 5/part_03 族 22/part_04 5/part_02 2/author_revision_review_routing 2/policy_routing 3/part_15 1/tool_policy 1/part_05 2/part_07 1/part_03/part_03 1）；handler/ws_types（tests.rs 4/roundtrip 4/scope_rejection 2/human_presentation 整文件/stage+protocol 重钉保留）；it_core part_01-06b 13 测（accept_author_output 夹具链）；it_web staged 族 51 测（staged_flow 2/serial 19/batch 7/compile 11+runtime_projection 1/mode 9/recovery_consistency 2）；前端 vitest 17 测+2 文件级。
- 待退役（it_web 残余红名单）：web_work_item_author 族/outline context_blocker 族/batch_generation_invokes…/mode outline_human_confirm…（见 handoff §剩余-1）。
