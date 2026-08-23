I have the full chain verified. One finding materially corrects my own prior proposal.

## D1 repair 路径验证结论（成立/不成立+证据）

**结论：分情况——我上轮的表述「NonceMismatch 可复活虚构 finding」不成立；真正成立的是 JsonNonceMismatch / MissingJsonNonce 两条码，且当前仓库状态下风险尚未激活，是 WP-1 自己会引入的。**

### 路径 A：整块照抄示例（start tag 与 JSON 双 EXAMPLE_NONCE）→ 不成立

| 步 | 证据 | 结果 |
|---|---|---|
| 1 | `parse_nonce` `src/cross_cutting/structured_output.rs:238-247` 只要求非空，接受 `EXAMPLE_NONCE` | 取到 start_nonce |
| 2 | `parse_block_at` `structured_output.rs:122-132`，start_nonce≠expected → 返回 `NonceMismatch`，第 130 行 `recoverable_value` 传 **None** | 无可恢复载荷 |
| 3 | `is_repairable` `review/structured_output.rs:38-52`，第 42 行前置条件 `recoverable_value.is_some()` 不满足 | **false** |
| 4 | `drive.rs:180-183` / `drive.rs:396+` 兜底 | `fallback_review_verdict` → NeedsHuman + `UserTriageRequired` |

`NonceMismatch` 全仓仅一处构造（`structured_output.rs:126`，grep 确认），且恒传 None。**`is_repairable` 第 47 行的 `NonceMismatch` 分支是恒不成立的死分支**——我上轮提的「observed_nonce==EXAMPLE_NONCE 时判不可修复」改在这条码上是语义空操作。既有测试 `rejects_copied_example_nonce`（`structured_output.rs:537-546`）已锁死这条路。

### 路径 B：混合照抄（start tag 用真 nonce N，JSON body 用 EXAMPLE_NONCE）→ 成立

| 步 | 证据 |
|---|---|
| 1 | `structured_output.rs:122` 通过（start_nonce==N） |
| 2 | `structured_output.rs:191` `let recoverable_value = business_payload_without_nonce(&value)` —— 在校验 nonce **之前**先摘出剥 nonce 的业务体，即照抄来的虚构 finding |
| 3 | `strip_and_validate_json_nonce` `structured_output.rs:283-292` → `JsonNonceMismatch`，`observed_nonce=Some("EXAMPLE_NONCE")` |
| 4 | `structured_output.rs:199-201` `error.recoverable_value = recoverable_value` —— 把虚构载荷挂回错误 |
| 5 | `is_repairable` `review/structured_output.rs:49` 命中 `JsonNonceMismatch` + 载荷非空 → **true** |
| 6 | `drive.rs:62`（直连）/ `drive.rs:270`（gateway）进入 repair |
| 7 | `build_review_repair_input` `prompts/review_repair.rs:16-20,31,37` 把虚构载荷序列化进 repair prompt，并标注「**必须逐字段保持语义一致**」；`review_repair.rs:15` 生成新 nonce |
| 8 | repair 轮回填新 nonce → `repair_payload_is_compatible` `review/structured_output.rs:55-66` 比较剥 nonce 后的值，相等 → `success_diagnostic(repair_succeeded=true)` `drive.rs:126-146` |

**虚构 finding 以合法新 nonce 通过，诊断标记为「修复成功」。路径成立。**

### 但当前尚未可利用（关键限定）

今天 prompt 里的 EXAMPLE_NONCE 块载荷是 schema 模板占位符（`prompts.rs:178-187` + `schema_with_nonce`），`verdict` 字面量是 `"pass|revise|needs_human"`。repair 轮通过 envelope 后仍会撞 `parse_review_value` `parsers.rs:139-143` → `InvalidVerdict`；findings 的 `severity` 占位符撞 `parsers.rs:541-548` → `MalformedFindings`。二者都是 `Schema` 错误，`recoverable_value()` 返回 None（`review/structured_output.rs:34`），走 `drive.rs:161` second_error → fallback NeedsHuman。

**即：路径 B 的闸门今天由「示例载荷不是 schema-valid」挡着，而不是由 nonce 校验挡着。WP-1 一旦引入带具体 verdict/severity/evidence 的判例并包在 sentinel 里，这道闸门立刻失效。风险是 WP-1 自造的，不是既有缺陷。** 这反转了我上轮把它写成「共享层既有 bug」的定性。

## D2 repair 轮绕过风险与配套修复

**判例文本不会在 repair 轮重发。** `build_review_repair_input`（`review_repair.rs:26-40`）从零拼 prompt，不调 `reviewer_output_contract`、不带 `reviewer_boundary_rules_for`、不复用 `base_input.prompt`；既有测试 `part_10.rs:758-751` 断言 repair prompt 不含 `当前阶段：` / `using-superpowers` / `[cadence_project_rules]`。WP-1 的判例天然不进 repair 轮。

**但 repair prompt 有两处主动「洗白」照抄内容：**
1. `review_repair.rs:32` 原样回灌 `completion.full_output` —— 首轮若照抄了示例，`EXAMPLE_NONCE` 字面量与虚构 finding 原文随之进入 repair 轮上下文。
2. `review_repair.rs:31` 把虚构载荷标注为「已恢复的原业务 JSON（必须逐字段保持语义一致）」—— 指令层面要求模型保真复述虚构内容。

**repair 轮重复输出 EXAMPLE_NONCE 是安全的**：只有一次 repair 机会，`drive.rs:159-179` 的 `Err(second_error)` 直接 fallback，无循环。start tag 再错 → NonceMismatch(None) → fallback；JSON 再错 → 同样 fallback。

**收紧后仍存在的三条绕过（按危险度排序）：**

- **B-1（最危险，与 is_repairable 无关）**：模型照抄判例载荷但两处 nonce 都写对 → envelope 直接 `Parsed`，**根本不进 repair 通道**，虚构 finding 由 `parse_review_value` 直接采纳。任何 `is_repairable` 收紧对此零作用。唯一有效对策：判例本身不可照抄化（见 D3/D4-2）。
- **B-2（我原方案漏掉）**：照抄载荷但**删掉 nonce 字段** → `MissingJsonNonce`（`structured_output.rs:274-282`），第 201 行同样挂上载荷 → `is_repairable` 第 48 行命中 → 可修复；而此码 `observed_nonce` 为 **None**，我提的「observed_nonce==EXAMPLE_NONCE」判定**完全拦不住**。
- **B-3**：`MissingEndTag` / `InvalidEndTag` 路径安全，无需处理——其载荷经 `recoverable_value()` `structured_output.rs:248-253`，内部 `strip_and_validate_json_nonce` 必须成功，照抄的 EXAMPLE_NONCE 在此返回 None。

**配套修复（替换我原方案）：**
1. `is_repairable` 增加基于**载荷内容**而非 nonce 值的判据：`recoverable_value` 命中判例指纹（见 D3 的 `evidence` 哨兵串）时判不可修复 → fallback UserTriage。同时覆盖 `JsonNonceMismatch` 与 `MissingJsonNonce`。
2. 删除 `review/structured_output.rs:47` 的 `NonceMismatch` 死分支（纯清理，附 1 条 contract 测试锁定其恒不可修复）。
3. repair prompt 增加一行显式约束：`nonce 必须是 {nonce}，禁止使用 EXAMPLE_NONCE 或原始输出中出现的任何其他 nonce`，并把「原始输出」段改为**剥离 sentinel 块后**的 readable 文本，避免回灌照抄块。
4. 最强的一道：判例不带 sentinel 封装（D3），从源头消灭照抄向量。

## D3 判例精简版文案

原 ~3.9KB 的成本主要来自「每个判例包一个完整 sentinel + 完整 JSON」。**去掉封装与 JSON、只保留判定语义**后三条合计 1279 B（含表头），远低于 2.5KB，因此**判例 3 不必砍**；若仍要压到两条，砍判例 3（它与 `prompts/review.rs:87-89` 既有契约文字重复度最高）。

表头（152 B），承担「不可照抄」的结构性保证：

```text
[reviewer_boundary_examples]
以下判例只示范 severity 边界，不含 nonce 与 sentinel 封装，禁止照抄其文字或 ID 到真实输出。
```

判例 1（414 B）：

```text
[边界判例1｜抽象追踪]
产物状态：## 决策记录 与 DEC-001 在位，dec_req_links 指向 REQ-003，但决策正文仅写"采用分层架构"，未展开层边界。
正确判定：severity=suggestion。必需 heading / 三族 ID / source 全部在位，下一阶段可用；"描述不够具体"是深度建议。
错误判定：must_fix 或 blocking —— 把已可用产物打回返修。
```

判例 2（431 B）：

```text
[边界判例2｜测试越界]
产物状态：## 组件契约 缺 CMP-002 的 API-002 引用（断链），同时未写单元测试用例。
正确判定：只对断链出 severity=must_fix（parser gate 命中，下一阶段不可用）；"补单元测试用例/覆盖率"不得成为 finding —— 测试属实现阶段，超出 Design 边界。
错误判定：为缺测试出 must_fix，或把断链降级为 suggestion。
```

判例 3（282 B，可选）：

```text
[边界判例3｜风险提及即可]
产物状态：## 风险与权衡 列出"跨仓迁移可能超期"，未给量化验证方案。
正确判定：verdict=pass。风险已显式记录，Design 段不要求验证方案。
错误判定：以"风险未验证"出 must_fix。
```

三条均无 `verdict` 合法字面量以外的可直接复用 JSON、无 `evidence` 字段、无 sentinel，故照抄进结构化输出后必然撞 schema 或根本不成块。判例内 ID 统一用 `DEC-001/CMP-002/API-002/REQ-003` 这组固定值，可直接作为 B-1 的照抄指纹（真实产物几乎不会同时出现这四个 ID 且 evidence 为空）。

## D4 方案自我批判与修正

**1. 改错了错误码（最严重）。** 我上轮把修复点定在 `NonceMismatch`，而该码恒不携带 `recoverable_value`（`structured_output.rs:130`），`is_repairable` 第 47 行本就是死分支——照我原文实施等于什么都没修，却会留下「已修复」的假象，并可能让 WP-1 的判例带着 sentinel 上线。修正：改判据落到 `JsonNonceMismatch` + `MissingJsonNonce`，且用载荷内容指纹而非 `observed_nonce`（后者对 `MissingJsonNonce` 恒为 None）。

**2. 我自己是风险的引入者，定性写反了。** 我把它描述成共享层既有缺陷，但今天示例载荷不是 schema-valid，路径 B 被 `parse_review_value` 挡死；是「带具体判例 + 包 sentinel」这个设计把它变成可利用。修正：判例一律不带 nonce/sentinel/完整 JSON（D3 已按此重写），从源头消除。相应地，`is_repairable` 收紧从「阻塞性前置修复」降为**纵深防御**，但仍建议随 WP-1 同批落地，因为它同时封住 B-2。另外必须承认：**B-1（两处 nonce 都写对的照抄）绕过整个 repair 通道，任何 `is_repairable` 改动都无效**——这是我原方案完全没覆盖的主路径，只有 D3 的不可照抄化能挡。

**3. 漏了 repair prompt 的洗白效应。** 我只看了注入侧，没看 `review_repair.rs:31-32` 把虚构载荷标为「必须逐字段保持语义一致」并回灌 `full_output`。修正：加 nonce 显式排除提示 + 用剥离 sentinel 后的 readable 文本替代 `full_output` 回灌（D2 第 3 条）。

**4. 注入点算多了，WP-1 范围可砍 5/6。** Design 只经 `build_review_input`（`prompts/review.rs:15`），其第 16-18 行把 WorkItemPlan 路由走；另外 5 处 `reviewer_output_contract` 调用（`review.rs:285/413/598/668/811`）全属 WorkItemPlan 家族（plan/projection/outline/batch/draft，按 outline 行区间核对）。因此「给 `reviewer_output_contract` 增参 + 改 6 处调用点 + 非 Design 返空串」是无谓的共享签名变更。修正：只在 `review.rs:82` 单一调用点前 `push_str` 判例串，`reviewer_output_contract` 签名不动——少动 5 个 WorkItemPlan 调用点，也不需要「非 Design 返空串」这条测试。

**5. 判例插入位置需要一条锚定测试。** 原方案说「EXAMPLE_NONCE 示例之后、真实 nonce 模板之前」，但若按修正 4 在调用点外拼接，判例会落在整个 contract **之前**。`prompts.rs:809-812` 已有「EXAMPLE_NONCE 先于真实 nonce」的顺序断言，判例串不含 nonce 故不受影响。建议新增一条断言：判例串出现在 `实际输出模板` 之前，且判例串不含 `ARIA_STRUCTURED_OUTPUT` 与 `nonce=`——后者正是防回归到「带封装判例」的锁。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "只读评审任务：未修改任何文件、未执行 git 写操作、未派生子代理。git status --porcelain 输出为空。产出为 D1-D4 四段中文 markdown 评审报告，范围严格限定在受托的「判例×repair 交互深挖 + 方案作者自查」，未扩展到 WP-2/3/4 或 P2 待拍板项的实现。"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "cargo test --locked --lib cross_cutting::structured_output",
      "result": "passed",
      "summary": "11 passed / 0 failed。其中 rejects_copied_example_nonce 与 classifies_missing_and_mismatched_json_nonce_with_distinct_stable_codes 是 D1 路径 A 不成立、路径 B 成立的直接证据"
    },
    {
      "command": "git status --porcelain",
      "result": "passed",
      "summary": "输出为空，工作区干净，无暂存文件"
    },
    {
      "command": "ast-grep outline (structured_output.rs / drive.rs / prompts/review.rs)",
      "result": "passed",
      "summary": "按 code-reading.md 先取结构大纲再定向精读；用 review.rs 方法行区间核对 6 处 reviewer_output_contract 调用点归属"
    }
  ],
  "validationOutput": [
    "NonceMismatch 全仓唯一构造点 structured_output.rs:126，第 130 行 recoverable_value 恒为 None → is_repairable(review/structured_output.rs:47) 该分支恒 false，为死分支",
    "structured_output.rs:191 在 nonce 校验前摘出 business_payload_without_nonce，第 201 行回挂到错误上 → JsonNonceMismatch(283-292) 与 MissingJsonNonce(274-282) 两码均携带可恢复载荷",
    "MissingJsonNonce 的 observed_nonce 恒为 None，原方案的 observed_nonce==EXAMPLE_NONCE 判据对其无效",
    "review_repair.rs 从零构造 prompt，不含判例文本（part_10.rs:748-751 既有断言佐证）；但第 32 行回灌 full_output、第 31 行标注必须逐字段保持语义一致",
    "Design 仅经 prompts/review.rs:15 build_review_input（16-18 行路由走 WorkItemPlan），其余 5 处 reviewer_output_contract 调用点均属 WorkItemPlan 家族",
    "判例精简实测：表头 152B + 三条 414/431/282B = 1279B，两条方案 997B，均低于 2.5KB 上限"
  ],
  "residualRisks": [
    "B-1 未被任何 is_repairable 改动覆盖：模型照抄判例载荷且两处 nonce 均正确时 envelope 直接 Parsed，绕过整个 repair 通道，仅能靠 D3 的判例不可照抄化缓解；建议 WP-1 验收里为此单列一条负例",
    "载荷指纹判据（按固定 ID 组 + evidence 缺失识别照抄）属启发式，存在误判真实 finding 的理论可能；建议指纹只用于降级为 UserTriage 而非静默丢弃，保留人工可见性",
    "本次未实际运行 review 全链路集成测试（part_14 系列 repair 用例），D1 路径 B 的第 7-8 步是基于代码精读推演而非端到端执行验证；若要坐实建议补一条 JsonNonceMismatch + schema-valid 载荷的集成负例",
    "判例 token 成本 1279B 系字节数实测，未换算为各 provider 的实际 token 数"
  ],
  "noStagedFiles": true,
  "diffSummary": "无代码改动（只读评审任务）",
  "reviewFindings": [
    "blocker: 提案 WP-1 附带修复段 - 修复点定在 NonceMismatch 是空操作（review/structured_output.rs:47 为死分支，structured_output.rs:130 恒传 None），须改为 JsonNonceMismatch + MissingJsonNonce 且用载荷内容指纹而非 observed_nonce",
    "blocker: 提案 WP-1 判例设计 - 判例若包 sentinel/EXAMPLE_NONCE 封装，将由 WP-1 自身引入今天并不存在的可利用路径（当前被示例载荷非 schema-valid 挡住）；判例必须去封装、去完整 JSON",
    "major: 提案定性错误 - 该风险不是共享协议层既有缺陷，而是 WP-1 引入项，立项叙述需修正以免误导优先级",
    "major: 遗漏 B-1 主路径 - 两处 nonce 均正确的照抄绕过 repair 通道，原方案完全未覆盖",
    "major: 遗漏 B-2 - MissingJsonNonce 同样携带照抄载荷且 observed_nonce 为 None，原判据拦不住",
    "minor: 提案 WP-1 范围虚高 - 6 处调用点可砍至 1 处（prompts/review.rs:82），reviewer_output_contract 签名无需变更，配套的「非 Design 返空串」测试也随之不需要",
    "minor: repair prompt 缺 nonce 排除提示，且回灌 full_output 会把照抄块带入 repair 轮上下文"
  ],
  "manualNotes": "两点需要 parent 拍板。第一，WP-1 的「附带共享层修复」在现有文字下等于空操作，必须按 D2 重写为 JsonNonceMismatch + MissingJsonNonce 的内容指纹判据，否则该工作包会带着虚假的安全结论上线。第二，判例是否包 sentinel 封装是本次评审最关键的单点决策：不包封装则 3.9KB 降到 1.3KB、B-1/B-2 两条绕过同时消失、且 WP-1 的注入点从 6 处降到 1 处；包封装则需要额外补一整套 repair 侧防护才能达到同等安全水位。我的建议是不包封装。另外 WP-1 范围可显著收窄（reviewer_output_contract 签名不动），对应的验收断言也应从「非 Design 返空串」改为「判例串不含 ARIA_STRUCTURED_OUTPUT 与 nonce=」这条防回归锁。"
}
```