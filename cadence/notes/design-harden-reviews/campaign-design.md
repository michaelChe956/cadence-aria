我已完成全部定向阅读与核实。以下为完整输出。

---

## B1 story campaign 基础设施盘点（复用/定制矩阵）

先声明关键事实：`cadence/reports/story-weak-model-campaign/` 下**实际存在**的文件为 `corpus/`（5 形态 + `README.md` + `digests.txt`）、`golden/`（`issue_0001.golden.json` + `digests.txt`）、`golden_diff.py`、`test_golden_diff.py`、`gate-manifest.json`、`baseline-sanity.md`、`optimized-sanity.md`、`gate-report.md`。`validate_manifest.py` 与 `manifest.schema.json` **不存在**（tasks.md 7.2 要求落盘但实际未落盘，全文 `rg` 仅命中 gate-manifest.json 自身），这是 story 遗留缺口。

| 资产 | story 现状（核实） | Design 处置 | 结论 |
|---|---|---|---|
| corpus 形态定义 | 5 个 API-creatable Issue 描述 .md，每形态一个文件；README 说明冻结规则 | 需 6 形态 + **每形态配一个冻结上游 Story Spec fixture**（Design 的 canonical_inputs 来自已确认 Story Spec，见 §B3） | **定制** |
| digest 冻结 | SHA-256 over UTF-8 bytes，`corpus/digests.txt` 记 `文件名  hexdigest`；golden 记 `hexdigest  路径`（两处格式不统一） | 沿用 SHA-256，统一为 `相对路径  hexdigest` 一种格式 | **复用+统一** |
| golden 规范化 | `golden_diff.py` 的 `normalize()` 是 Story 专属（REQ/AC/NFR + 专属 `## 用户确认决策` 章节）；`compare()` 是字段无关的 diff 框架 | `compare()`/`read_markdown()` 框架复用；`normalize()` 必须重写为 DEC/CMP/API 版 | **框架复用 / normalize 定制** |
| golden 字段 | `schema_version/heading_set/req_ids/ac_ids/nfr_ids/ac_req_links/source_id_coverage/user_decisions` | 映射为 `heading_set/dec_ids/cmp_ids/api_ids/dec_req_links/source_id_coverage/user_decisions(+referenced_req/ac_ids)` | **定制** |
| pass/fail 规则 | forbidden_missing / forbidden_added / forbidden_link_lost / forbidden_source_lost / forbidden_decision_{lost,reversed,binding_changed} | 语义同构复用，仅字段名与 decision 章节位置不同 | **复用规则框架** |
| gate-manifest.json | 顶层 `{schema:"gate-campaign/1", run_kind:"revised", samples:[30]}`；sample 含 provider/shape_id/shape_file/repetition/case_id/issueId/sessionId/storySpecId/firstArtifactSec/authorConfirmSec/reviewCompleteSec/finalizeSec/choices/permissionApprovals/nodes/reviewVerdicts/error/failureClass/finishedAt/elapsedSec 等 | 顶层与 sample 结构复用，`storySpecId` → `designSpecId`；**补上 story 缺失的 model/version/strategy 字段**（见 B6 问题2） | **复用+补字段** |
| manifest 校验器 | **不存在** | 新建 `validate_manifest.py` + `manifest.schema.json`，先服务 design，可回填 story | **新建（补 story 缺口）** |
| test_golden_diff.py | 指向 `.aria/.../issue_0001/.../story_spec_0001/version_0001.json` 为 SOURCE | 改写 SOURCE 指向 design 版本 fixture（`SpecVersionRecord.markdown` 键 Story/Design 共用，读取无需改） | **定制** |
| baseline/optimized-sanity.md | 记录 API 发现 + 运行记录表 + 对照结论 + 120s/300s 窗口口径 | 模板复用，替换为 design 端点与两阶段驱动说明 | **复用模板** |
| WS driver 协议 | `hello→prepare_context→start_generation→author_confirm→(author_decision=accept_with_review)→reviewer→accept_finalize`，choice 自动回传首选项 | 协议完全复用（node 标题 Story Spec 生成→Design Spec 生成）；**新增「先确认 Story Spec」前置步** | **复用+前置步** |
| token usage | 最终 manifest **无任何 usage 字段**（gate-report 明示「按裁决记录该事实」） | 新建记录规则 + usage_unavailable 标记，不得造假（见 B4） | **新建规则** |

要点：Design 与 Story 的差别不在 runner/manifest 骨架，而在（1）语料需要冻结上游 Story Spec，（2）normalizer 字段与 decision 章节位置，（3）manifest 校验器需补建。

---

## B2 design golden normalizer 字段规范（含 pass/fail 规则）

**契约来源**（已核实 `artifact_constraint_spec_for(WorkspaceType::Design)`，artifact_constraints.rs:126-158）：

- 必需二级 heading（7）：`设计范围 / 设计决策 / 公共组件 / API 契约 / 数据模型 / 风险 / 追踪关系`（含英文别名 Design Scope / Design Decisions / Shared Components / API Contract / Data Model|Data Entities / Risks / Traceability）。
- 必需稳定 ID 三族：`[DEC-*]`、`[CMP-*]`、`[API-*]`（至少各 1）。
- 必需追踪 token：`source id`（literal）。
- 禁止 heading：`Work Item Plan / 任务拆分 / 开发任务 / 执行 checklist`；禁止 token：`[TASK-*]`、`WI-*`。
- reviewer 边界（reviewer_must_fix_rules）：测试计划/范围/场景/文件/模块/框架/夹具/命令/构建命令/执行 checklist/测试或验证职责分配 → **must_fix**；纯抽象 `[DEC-*]→[REQ-*]/[AC-*]` 且不写「如何测试」的追踪 → **不得 must_fix**。
- choice→decision 绑定（prompts.rs:498 `structured_interaction_artifact_decision_contract(Design)`）：结构化交互审计回答必须写入 **`## 设计决策` 或 `## 追踪关系`**，保留 `author-decision-*` 或映射到 `[DEC-*]`，并绑定来源 `[REQ-*]/[AC-*]/[DEC-*]`。**与 Story 的「专属 `## 用户确认决策` 章节」不同——Design 无独立决策章节，且 ID 可双形态。**

**normalizer 输出 schema**（`schema_version: 1`，新增 `workspace_type: "design"` 防串门）：

```json
{
  "schema_version": 1,
  "workspace_type": "design",
  "heading_set": ["设计范围","设计决策","公共组件","API 契约","数据模型","风险","追踪关系"],
  "dec_ids": ["DEC-001","DEC-002"],
  "cmp_ids": ["CMP-001"],
  "api_ids": ["API-001","API-002"],
  "referenced_req_ids": ["REQ-001"],
  "referenced_ac_ids": ["AC-001"],
  "dec_req_links": {"DEC-001": ["REQ-001","AC-001"], "DEC-002": ["DEC-001"]},
  "source_id_coverage": {
    "DEC-001": ["story_spec_0001#功能需求","issue_0001#背景"],
    "CMP-001": ["story_spec_0001#范围"],
    "API-001": ["issue_0001#范围限制"]
  },
  "user_decisions": {
    "author-decision-001": {
      "answer": "选用 MessagePack，压缩率优先",
      "dec_id": "DEC-001",
      "bindings": {"dec_ids": ["DEC-001"], "req_ids": ["REQ-001"], "ac_ids": []}
    }
  }
}
```

**pass/fail 规则**（迁移 story `compare()` 的 kind 语义）：

| 字段 | 规则 | 与 story 差异 |
|---|---|---|
| `heading_set` | 精确集合：forbidden_missing（缺 7 之一）+ forbidden_added（出现 `测试计划` 等禁止 heading） | 7 个 heading，比 story 多 1 个且无 `待确认项` |
| `dec_ids/cmp_ids/api_ids` | forbidden_missing + forbidden_added | 三族替代 REQ/AC/NFR 两族+一档 |
| `referenced_req_ids/ac_ids` | 仅 forbidden_missing（上游 REQ/AC 引用不得丢失） | 新增；不查 added（design 可多引用） |
| `dec_req_links` | forbidden_link_lost（golden 的 DEC→X 链接不得丢） | 对齐 story `ac_req_links` |
| `source_id_coverage` | forbidden_source_lost | 复用 story 的 `source id:` 正则，源可含 `story_spec_*#…` |
| `user_decisions` | forbidden_decision_lost / forbidden_decision_reversed / forbidden_decision_binding_changed | **抽取位置与键双形态**（见下） |

**decision 抽取定制**（Design 与 Story 唯一实质差异）：

- Story：在 `## 用户确认决策` 章节按 `author-decision-*` 键 + `**用户选择**：…` 抽取。
- Design：在 `## 设计决策` ∪ `## 追踪关系` 两章节扫描；decision 键为 `author-decision-*`（若存在）否则锚定到 golden 记录的 `dec_id`（`[DEC-*]`）；答案抽取沿用 `**用户选择**：…`；绑定集合取块内 `[REQ-*]/[AC-*]/[DEC-*]`。
- golden 的 `user_decisions` 键**统一用 `author-decision-*`**，并以 `dec_id` 记录等价映射；normalizer 用「键命中 或 dec_id 命中」二者之一定位，二者都无 → `forbidden_decision_lost`。

`read_markdown()` 无需改（Design 版本 JSON 同用 `SpecVersionRecord.markdown` 键）；`compare()` 框架与 kind 字符串可复用；仅 `normalize()` + `DECISION_RE` 抽取范围需 Design 版。

---

## B3 Design corpus 形态清单（≥5，含语料草案）

**结构性前提（相对 story 的最大定制）**：Design 的 `generate_design_specs` 要求 `story_spec_ids` 非空且每项 `Confirmed`（lifecycle.rs:937-966）；其 `[canonical_inputs]` 的「关联上下文」即已确认 Story Spec 的 latest `markdown`（workspace_context/entity.rs:35-46, linked_story_context）。因此**每个 Design 语料形态必须配对一份冻结的上游 Story Spec fixture**（含稳定 `[REQ-*]/[AC-*]` 供追踪锚定），由 runner 以 Confirmed 状态种入再触发 design-specs:generate。单仓 = Legacy routing，`aggregate_codebase=None`，`involved_repository_ids/change_order` 恒空，author `structured_output_contract=None`（红线确认，prompts.rs build_streaming_input）。

| # | 形态名 | 需求正文草案（<100字） | 考察点 | 预期 DEC/CMP/API 数量级 | 含 choice | 含返修轮 |
|---|---|---|---|---|---|---|
| D01 | 单仓 API 设计 | 为订单服务新增「按状态查询订单列表」HTTP 接口并分页返回；需定义请求/响应字段、错误码与幂等语义，不实现代码。 | 7 heading 齐全；`API 契约` 至少 1 个 `[API-*]`；source id 覆盖 issue#背景 与 story#功能需求；无测试计划 heading/token。 | DEC 1-2 / CMP 1 / API 2-3 | 否 | 否 |
| D02 | 单仓数据模型设计 | 为多租户报表系统设计数据模型：报表定义表、生成任务表、租户隔离字段；给出实体关系与索引策略，不实现迁移。 | `数据模型` 实体/字段；`[CMP-*]`≥1；`[DEC-*]` 记录建模决策；source id 逐约束覆盖。 | DEC 2-3 / CMP 1-2 / API 0-1 | 否 | 否 |
| D03 | 含用户确认 choice 映射 DEC | 为缓存层选择序列化方案（JSON 或 MessagePack），两者均可；设计前先向用户确认选型，并把选择写入设计决策并绑定来源需求。 | author 必触发 AskUserQuestion（`detect_author_choice_request` 仅 Story/Design 生效）；回答后决策写入 `## 设计决策`/`## 追踪关系`，`author-decision-*` 或 `[DEC-*]`，绑定 `[REQ-*]`；golden 校验 answer+binding。 | DEC 1-2（其一来自 choice）/ CMP 1 / API 0-1 | **是（必触发）** | 否（若未落章 → review must_fix，为隐性返修） |
| D04 | 抽象追踪正例（review 不应强返修） | 为订单结算新增抽象设计：定义结算计算策略与金额舍入规则，并把设计决策追溯到已确认 Story 的 `[REQ-*]/[AC-*]`，只写关联不写测试。 | 正文含纯抽象 `[DEC-*]→[REQ-*]/[AC-*]`；reviewer 不得误判 must_fix（假阳性=0）；golden 校验 `dec_req_links`。 | DEC 2 / CMP 0-1 / API 0 | 否 | 否（若误判 must_fix → 记边界假阳性 failure） |
| D05 | 测试越界反例（review 应 must_fix 返修） | 设计订单导出功能的组件与文件职责，并说明如何用自动化测试验证导出正确性，给出测试文件路径与运行命令。 | 故意引诱 author 在 Design artifact 写测试计划/文件/命令/职责分配；reviewer 必须 must_fix 并触发返修（假阴性=0）。 | DEC 1-2 / CMP 1 / API 1（artifact 内埋越界内容） | 否 | **是（review must_fix → 返修轮）** |
| D06 | review-revision 多约束 | 设计缓存预热任务：必须复用现有调度器、限定改动目录、子任务失败不取消其他租户任务；覆盖公共组件与风险，决策保持可追踪。 | 多约束在 `公共组件/风险/追踪关系` 固化；弱模型漏约束 → review must_fix 返修；source id 覆盖每条约束。 | DEC 3-4 / CMP 2 / API 0-1 | 可能 | 可能 |

覆盖要求核对：单仓 API（D01）✓、单仓数据模型（D02）✓、choice 映射 DEC（D03）✓、抽象追踪正例（D04）✓、测试越界反例（D05）✓，另补多约束返修形态（D06）。每个形态配套 `NN-story-fixture.md`（冻结 Story Spec），与 Issue 描述、`corpus/digests.txt` 一并冻结；`corpus/README.md` 注明「campaign runner 不得改写语料与上游 fixture」。

---

## B4 campaign 判定口径建议

**Provider 组合**：沿用 story 的 3 组合 `claude_code / kimi_code / pi`（author=reviewer 同 provider）。不新增 provider（避免扩范围）。每个组合在 manifest 补记 `model/version/strategy`（story 缺失，见 B6）。

**样本数与门槛（沿用 vs 调整，给理由）**：

- **推荐沿用 3 组合 × 10 样本/组合 = 30 样本，95% 门槛 = 10/10**。具体排布：6 形态按「4 常规形态（D01/D02/D03/D06）× 2 重复 + 2 边界形态（D04/D05）× 1 重复」= 10 样本/组合。
- 理由：
  1. 本任务要求与 story **同等口径**；10/10 门槛与样本量保持完全可比，是最直接的对照基线。
  2. Design 每样本比 story 多一个上游依赖（种入已确认 Story Spec + 多一次前置校验），单样本成本更高，不宜盲目加量。
  3. 边界分类是**辅助信号**（非 release gate）：D04/D05 跨 3 组合共 6 次观测，足以支撑「是否需要 P1 判例加固」的决策；若要把它升级为 95% gate，应另立 mini-campaign（2 边界形态 × 4 重复），不混入主 gate。

**四口径指标**：

1. **author 一次通过率**：首轮 author turn 产出通过 `validate_workspace_artifact_constraints` 的完整 artifact（7 heading + DEC/CMP/API 各≥1 + `source id` + 无禁止项），无 artifact retry、无结构阻塞；分母=样本数。
2. **reviewer syntax+schema 通过率**：review completion 无 `ReviewCompletionError::Syntax|Schema`（结构化 JSON 解析成功 + verdict/findings severity 仅三档合法）；分母=启动 review 的样本数。
3. **full-chain 一次成功率**：`finished=true` 且 `reviewVerdicts` 非空且 `verdict=pass`（无 revise/needs_human 返修轮）；同时记录 verdict 分布 `Counter({pass/revise/needs_human})` 与 retry 分布。
4. **边界分类正确率**：D04 假阳性率（纯抽象追踪被误报 must_fix，目标 0）+ D05 假阴性率（测试越界被漏报，目标 0）；分母=边界样本（各 3）。作为 P1 判例加固的触发指标，不单独设 95% gate。

**token usage fresh/resume 分列**（story 未做，design 补规则）：

- 来源：provider 事件（`timeline_node_002` author_run / `timeline_node_004` reviewer_run）暴露的 input-token usage。
- `fresh` = author 首轮 turn；`resume` = 返修/复核轮（revision、review）。按 `(provider, shape, case_id)` 分列，fresh/resume 分别统计均值。
- token gate 沿用「优化后 fresh 均值 / 基线 fresh 均值 ≤ 0.60」；若基线无可用 usage（story 即如此），**照 story 先例记 `usage_unavailable` + 原因于 ledger，不得以 0 冒充、不得入分母**。
- 未暴露 usage 的 provider（如 kimi ACP）记 `usage_unavailable: true`。

**超时与 retry 记录规则**：

- author 单轮上限 600s（story revised 口径）、reviewer 单轮上限 600s、WS driver 每样本总上限 900s。
- 300s 内未完成先记 driver-timeout 并复跑 600s，仍失败才记 failureClass（story 先例：驱动超时≠模型失败）。
- kimi 登录过期 → 重登录 + 全量重跑（story 先例）。
- choice 自动回传首选项（`q0_opt_0`/`opt_0`），prompt/picked 写入 `choices[]`。
- retry = review verdict revise/needs_human 触发的 author 返修轮 + artifact retry 次数；retry **计入统计**（不剔除样本、不重复计分母），manifest 记录 `reviewVerdicts` 长度与 `toolFailures`。

---

## B5 工作包分解与文件清单

目标目录：`cadence/reports/design-weak-model-campaign/`（与 story 平行，互不干扰）。

**新建文件**（运行时产物标 ⚙）：

- `corpus/README.md` — 冻结规则 + 上游 Story fixture 配对说明
- `corpus/01-api-design.md` … `06-multi-constraint.md`（6 形态）
- `corpus/01-story-fixture.md` … `06-story-fixture.md`（6 份冻结上游 Story Spec）
- `corpus/digests.txt` — SHA-256 冻结
- `golden/design_0001.golden.json`（首个 golden，建议锚 D03 的 choice 绑定） + `golden/digests.txt`
- `design_golden_diff.py` — Design normalizer + compare（复用 story `compare()`/`read_markdown()` 框架）
- `test_design_golden_diff.py` — 回归单测
- `validate_manifest.py` + `manifest.schema.json` — **新建（补 story 缺口，先服务 design，可回填 story）**
- ⚙ `gate-manifest.json`、`baseline-sanity.md`、`optimized-sanity.md`、`gate-report.md`

**复用不改**：story `golden_diff.py`（Story 语义冻结不动）、story corpus/golden、WS driver 协议、digest 机制、sanity 报告模板。

**修改**：无（红线：不修改 story 语义，不触碰共享生产代码）。

**工作包与工作量**：

| WP | 内容 | 交付物 | 估计 |
|---|---|---|---|
| WP-D1 | 语料冻结：6 形态 + 6 上游 Story fixture + digests + README | corpus/ 全套 | 0.5d |
| WP-D2 | Design golden normalizer + 回归测试 + 首个 golden | design_golden_diff.py、test_design_golden_diff.py、golden/ | 1d |
| WP-D3 | manifest schema + 校验器（含按 `(provider,model,strategy,case_id,run_kind)` 去重、缺边/重复/retry 入分母/零 usage 拒绝） | validate_manifest.py、manifest.schema.json | 0.5d |
| WP-D4 | 基线 sanity：3 组合 × 1 样本真机驱动 + 两阶段（Story 种子→Design 生成）协议固化 | baseline-sanity.md | 0.5d（+ 真机等待） |
| WP-D5 | gate campaign：3×10 + 四口径统计 + token ledger + gate-report | gate-manifest.json、gate-report.md | 1d（+ 真机运行时间） |
| WP-D6 | 边界判例结论 + B6 开放问题收敛 | 结论写入 report | 0.5d |

合计约 4 人日 + 真机运行/等待时间。

---

## B6 开放问题

1. **story 的 manifest 校验器未落盘**：tasks.md 7.2 要求 `validate_manifest.py` + `manifest.schema.json`，实际不存在。design 是否新建并**回填**校验 story 的 `gate-manifest.json`（会暴露 story 缺字段，需裁决）。
2. **story 最终 manifest 缺 `model/version/strategy/token usage` 字段**，与 tasks.md 7.2 去重口径不符。design 若补齐这些字段，与 story manifest 结构不完全一致；是否接受「design 更严」？
3. **上游 Story Spec 种子方式**：store 级直接种入 Confirmed Story Spec fixture（fixture 精确、隔离 Story 变量）vs API 两阶段先跑 story 再 design（引入 story 生成噪声）。推荐前者，需确认 runner 可写 `.aria` store。
4. **design decision 键双形态**：`author-decision-*` 与 `[DEC-*]` 均可（prompt 明示），golden 的锚定规范（统一 author-decision-* + `dec_id` 等价映射）需正式确认。
5. **`dec/cmp/api` 的 forbidden_added 是否过严**：story 对 REQ/AC/NFR 双向禁增减；Design 决策开放度更高，新增 DEC 可能合法。是否降级为 warning-only？
6. **边界指标样本量**：D04/D05 各 3 观测是否足够支撑 P1 判例结论；是否另立 2×4 的边界 mini-campaign。
7. **样本排布最终拍板**：本方案「4 常规 ×2 + 2 边界 ×1 = 10/组合」；是否改为 6×2=12（门槛变 12/12）。

---

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "全程只读：未修改任何文件、未执行任何 git 写操作、未派生子代理；scope 严格限定单仓 Design（aggregate_codebase=None 分支），aggregate 符号（aggregate_design_scope_prompt/write_back_aggregate_output/parse_design_aggregate_output/validate_confirm_aggregate_spec）一律仅标注'超出范围'未深入。输出为 B1-B6 规划文档 + acceptance 报告。"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "find cadence/reports/story-weak-model-campaign -type f",
      "result": "passed",
      "summary": "盘点 story campaign 全部资产，确认 validate_manifest.py/manifest.schema.json 不存在"
    },
    {
      "command": "python3 读取 gate-manifest.json 顶层结构与 sample 字段",
      "result": "passed",
      "summary": "确认 schema=gate-campaign/1、run_kind=revised、samples=30；sample 字段清单与无 token usage/model/version 字段"
    },
    {
      "command": "rg -n validate_manifest|manifest.schema|gate-campaign/1",
      "result": "passed",
      "summary": "全仓检索确认无 manifest 校验器；gate-campaign/1 仅出现在 story gate-manifest.json"
    },
    {
      "command": "ast-grep outline artifact_constraints.rs / parsers/choice.rs / decisions.rs / review.rs",
      "result": "passed",
      "summary": "定位 WorkspaceType::Design 契约、choice 检测、review 三档 severity 与 gate"
    },
    {
      "command": "rg 定位 design-specs:generate 路由与 GenerateDesignSpecsRequest / generate_design_specs / validate_confirmed_story_specs",
      "result": "passed",
      "summary": "确认 Design 生成需 story_spec_ids 非空且 Confirmed；单仓=Legacy routing、aggregate_codebase=None"
    },
    {
      "command": "git status --short && git diff --stat",
      "result": "passed",
      "summary": "工作树干净，无暂存文件（noStagedFiles=true）"
    }
  ],
  "validationOutput": [
    "Design 契约核实：7 heading（设计范围/设计决策/公共组件/API 契约/数据模型/风险/追踪关系）+ [DEC-*]/[CMP-*]/[API-*] 各≥1 + source id + 禁止 [TASK-*]/WI-*/Work Item Plan/任务拆分/开发任务/执行 checklist",
    "reviewer 边界核实：测试计划/文件/命令/职责分配=must_fix；纯抽象 DEC→REQ/AC 追踪不得 must_fix",
    "choice 绑定核实：Design 决策写入 ## 设计决策/## 追踪关系，author-decision-* 或 [DEC-*]，绑定 [REQ-*]/[AC-*]/[DEC-*]（prompts.rs:498）",
    "单仓红线核实：is_aggregate_story_or_design()=false 时 structured_output_contract=None（单仓行为不变）",
    "上游依赖核实：Design linked_context=已确认 Story Spec latest markdown（workspace_context/entity.rs:35-46）"
  ],
  "residualRisks": [
    "本任务为规划文档，未运行任何真实模型样本；Design 弱模型成功率/边界准确率仍是'未测定'",
    "story 遗留缺口（无 manifest 校验器、无 token usage、无 model/version 字段）需在 B6 裁决",
    "边界分类指标样本量薄（D04/D05 各 3 观测），不足以单独设 95% gate",
    "Design decision 键双形态（author-decision-* vs [DEC-*]）的 golden 锚定规范待确认"
  ],
  "noStagedFiles": true,
  "diffSummary": "无 diff（只读分析任务，未修改任何文件）",
  "reviewFindings": [
    "no blockers"
  ],
  "manualNotes": "所有结论均来自对 worktree feat-b-0808-add-monorepo 的定向只读核实（story campaign 资产、artifact_constraints.rs、choice.rs、prompts.rs、lifecycle.rs、types.rs、workspace_context/entity.rs、parsers.rs）。建议 Design campaign 目录独立为 cadence/reports/design-weak-model-campaign/，不触碰 story 语义。"
}
```