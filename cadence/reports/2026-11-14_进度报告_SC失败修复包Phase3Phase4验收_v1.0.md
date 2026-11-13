# SC 失败修复包与完整失败源自含教学 —— Phase 3/4 验收报告

- 日期：2026-11-14
- 分支：`feat-wig-optimize-11-08`
- change：`deliver-sc-failure-repair-package`（REQ-WSC-33/34/35）
- 口径：只写已真实执行并读到输出的事实；未执行/受阻项单列。

## 一、结论

| 面 | 状态 | 依据 |
|---|---|---|
| A4 失败修复包（逐轮证据 + 严格发布 + 只读 JSON） | 通过（代码/接口/前端） | 89 条 sc_failure 用例全绿；隔离实例 REST 读回两个真实包 |
| A3 完整失败源（读回→归一化→核验→262144 硬闸） | 通过 | 35 条 sc_teaching 用例全绿，含超预算零新增启动 |
| 默认页面呈现真实包 | **未通过** | 受合成夹具缺 LC bootstrap 阻断（`aggregate_index_member_coverage_failed`），非产品缺陷 |
| 严格门禁 | 见第四节 | clippy / 全量 lib / guard / OpenSpec / fmt 逐项列证 |
| 8 臂对照测量 | 有效但全负 | after 侧 IR failed 3/4、compile 0/8、confirmed 0/8 ⇒ 不得宣称效果提升 |

## 二、阶段 3 真实结果

### 2.1 后端行为矩阵
```
sc_failure  : 89 passed; 0 failed
sc_teaching : 35 passed; 0 failed
```
- 本轮修正的三条用例全部是**用例读取面错**、产品行为正确，已在提交信息中逐条说明：
  1. 包用例：`session_state` 不内联 AuthorRun 节点详情 ⇒ `artifact_ref` 改从 durable node detail 读，并按版本 `source_node_id` 校验 `artifact_id`/`version`。
  2. 教学审计：prompt 槽按 disposition 分键 ⇒ 发送后应为 `prepared` + `sent` 两条且正文同源。
  3. replay 分类：Waiting 前提是该轮已有 RoundBound + provider_start 审计 ⇒ 先造轮、记审计、再重放。
- 一次真实缺陷修在用例侧之前：临时 `/tmp` 导出挂载会让全量跑必失败，已删除（提交 `2731383c`）。

### 2.2 接口（隔离实例，真实 publisher 落盘）
| 请求 | 结果 |
|---|---|
| `GET /api/system/health` | 200 |
| `GET /api/workspace-sessions/workspace_session_0001/artifact-versions/1` | 200，markdown，`is_current=true` |
| `…/artifact-versions/2` | 200，`sc_failure_repair_package`，`is_current=false`，`complete`，1 轮，`terminal_class=compile` |
| `…/artifact-versions/3` | 200，`incomplete`，`missing=[teaching_prompt]`，`disposition=unavailable` |

### 2.3 前端
- `tsc -b` + `vite build` exit 0。
- `sc-failure-repair.test.ts` + `ArtifactPane.test.tsx` + `ChatCockpitPage.plan.test.tsx` = **40 passed**（含默认 JSON 视图与历史版本切换）。

### 2.4 页面层受阻（诚实记录）
逐层推进到第 4 层被夹具阻断，每层均是产品**真实**报错：
1. `product_data_schema_unsupported` → 导出器先建 schema 解决；
2. `product_store_not_found: logical_codebase_54ef…` → 落别名 LC record 解决；
3. `repository_routing_target_missing` → 落 issue selection 解决；
4. `aggregate_index_active_missing` → 调 `POST …/aggregate-indexes/rebuild` 返回 **422 `aggregate_index_member_coverage_failed`**（成员仓为空，无真实源码）。

结论：产品正确拒绝为无成员的 LC 建空索引；要出截图需补「成员仓 bootstrap」。详见 `.superpowers/sdd/…/phase3-ui-blocker.md`。

### 2.5 本轮修掉的真实阻断：fresh 恢复零启动

显式 fresh 授权后 `sc_author` 不 spawn，对外帧为 `single candidate author native resume superseded (sc_failure_fresh_operation_not_running)`，而 durable 状态其实已是 `FreshRunning`。三处真实缺陷：

1. 轮身份锚读**引擎内存镜像**（由 run 入口快照构建，停在 `FreshReserved`），把 dispatch 已 CAS 提交的 `FreshRunning` 读成「未运行」；改为按其契约读 **durable** 记录。
2. 旧链**尚无任何 RoundBound**（首次尝试即被 supersede、provider 从未物理启动）被当作「无法归属」拒绝 ⇒ 这类会话永远无法恢复；改为按「链首轮、ordinal 0」新建链身份启动，不为未启动节点伪造 RoundBound。
3. `spawn` 携带的是 CAS **之前**的快照；改为携带 post-CAS 记录。

真实回归：`anchor_t17_sc_fresh` 由失败转 **1/1**；`sc_round_bound_is_durable` 1/1、`sc_bound_write_failure_blocks` 1/1、`fresh_operation_binding` 7/7 同步保持绿。

## 三、8 臂对照（有效样本，全负）

- 4×before(`a953327d`) + 4×after(`07a0df79`)，ABBA，固定输入（Story+Design 双确认、13 个编号、6AC 语义），provider 真实为 `tydic-anthropic/MiniMax-M3.1-Flash`。
- 结果：**compile 通过 0/8、confirmed 0/8、IR failed 3/4（after）**；reviewer 轮次 7/8 为 0。
- 结论：不得宣称本轮改动提升成功率或降低返修；样本可用于说明"失败可解释性"。

## 四、严格门禁

| 门禁 | 结果 |
|---|---|
| `openspec validate deliver-sc-failure-repair-package --strict` | 通过（exit 0） |
| `openspec validate --specs --strict` | 通过（81 passed / 0 failed） |
| `cargo fmt --all -- --check` | 通过（exit 0） |
| `cargo clippy --all-targets --all-features -D warnings` | 通过（0 violation，无输出） |
| `cargo test --locked --lib`（全量） | 通过（**6595 passed / 0 failed / 7 ignored**） |
| `cargo test --locked --test it_core large_file_guard` | 通过（**2 passed / 0 failed**；5 个顶格文件已按职责搬移） |

## 五、已知取舍（需评审知悉）

1. **两处 `large_enum_variant` 为带理由 allow**：`SingleCandidateAuthorCompletion`（141 处构造/解构点）、`FreshRoundAnchor`（一次决策返回一次）。理由写在代码注释里；boxing 不改行为只改栈字节。
2. **两处 `too_many_arguments` 为带理由 allow**：统一失败收口函数与两个测试夹具构造器（每个参数是独立真实事实）。
3. 以上 4 处 allow 与既有的 `result_large_err` 先例同型，**不是**掩盖缺陷。

## 六、未闭合

- 页面级真实浏览器截图（需成员仓 bootstrap）。
- 8 臂缺少成功臂 ⇒ "成功率/返修轮次"类结论不可得。
- TTFT 指标 durable 侧无时间戳，当前不可测。
