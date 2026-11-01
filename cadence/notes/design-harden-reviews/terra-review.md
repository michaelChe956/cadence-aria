## R1 各项评审结论

### 1. `is_repairable` 收紧：**建议改，但修正问题定位与实现条件**

**结论**

建议在 `WorkspaceEngine` reviewer repair 范围内，将 `observed_nonce == "EXAMPLE_NONCE"` 统一判为**不可修复**，直接降级 `UserTriage`；同时从可修复枚举中移除当前没有实际恢复载荷的 `NonceMismatch`，或至少以测试锁定其不可达性。

但提案对现状的描述不准确：**完整照抄 `EXAMPLE_NONCE` 示例当前并不会进入 envelope repair**，因而不会如提案所称“以新 nonce 复活虚构 finding”。

**证据**

- `src/product/workspace_engine/review/structured_output.rs:38-52`
  - `is_repairable` 除错误码外，先要求 `error.recoverable_value.is_some()`。
  - 它当前列出了 `NonceMismatch`、`JsonNonceMismatch` 等五类。
- `src/cross_cutting/structured_output.rs:122-131`
  - start-tag nonce 不匹配时直接返回 `NonceMismatch`，且明确传入 `recoverable_value=None`。
  - 完整复制：
    ```text
    <ARIA_STRUCTURED_OUTPUT nonce="EXAMPLE_NONCE">
    {"nonce":"EXAMPLE_NONCE", ...}
    </ARIA_STRUCTURED_OUTPUT>
    ```
    会走这一路径。因此 `is_repairable()` 已为 `false`，直接 fallback 到 `UserTriage`。
- 真正可被 repair 的 anti-copy 边缘是：
  ```text
  <ARIA_STRUCTURED_OUTPUT nonce="<真实 nonce>">
  {"nonce":"EXAMPLE_NONCE", "verdict":"revise", ...}
  </ARIA_STRUCTURED_OUTPUT>
  ```
  此时进入 `JsonNonceMismatch`；`src/cross_cutting/structured_output.rs:191-204` 会保留去除 nonce 的业务 JSON，当前可进入 repair。
- `is_repairable()` 的全部消费方只有两处：
  - `src/product/workspace_engine/review/drive.rs:62`：普通 provider 驱动；
  - `src/product/workspace_engine/review/drive.rs:270`：logical gateway 驱动。
- 因而改动影响的是所有 Workspace reviewer 类型：Story、Design、WorkItem、WorkItemPlan；而不是全局 sentinel 行为。
- Coding 不调用此方法。`src/product/coding_workspace_engine/review_parser.rs:218-224` 反而会直接尝试消费失败 sentinel 的 `recoverable_value`；Image 在失败时返回 `None`，不做 repair。故本改动**不会改变** coding/image 行为，也不能宣称其已消除全局 `EXAMPLE_NONCE` 风险。

**建议**

推荐实现形态：

```rust
const EXAMPLE_NONCE: &str = "EXAMPLE_NONCE";

error.recoverable_value.is_some()
    && error.observed_nonce.as_deref() != Some(EXAMPLE_NONCE)
    && matches!(
        error.code,
        MissingEndTag
            | InvalidEndTag
            | MissingJsonNonce
            | JsonNonceMismatch
    )
```

说明：

- 不能只对 `NonceMismatch` 做 `EXAMPLE_NONCE` 判断；当前完整示例照抄在该分支本来就不可修复，真正需拦截的是 `JsonNonceMismatch`。
- 保留其他随机/手误 nonce 的 `JsonNonceMismatch` repair：repair prompt 固定业务 payload，随后 `repair_payload_is_compatible` 等值校验，仍是合理的“只修封装”恢复。
- 输出了 `EXAMPLE_NONCE` 而被 triage 的合法场景理论存在，但这是固定的、从不派发的示例 nonce。对于带着示例业务 finding 的输出，牺牲一次自动封装修复、转人工确认是可接受的 fail-closed 取舍。
- 若产品目标是“任何 sentinel 消费方都不得信任示例 nonce”，应另立共享协议工作项；不能将本 WP 的 Workspace-only 变更误包装为 coding/image 修复。

**测试清单**

1. 保留并加强 `cross_cutting::structured_output::tests::rejects_copied_example_nonce`：断言完整复制时 `NonceMismatch`、`observed_nonce=EXAMPLE_NONCE`、`recoverable_value=None`。
2. 为 `ReviewCompletionError::is_repairable` 加直接单测：
   - `JsonNonceMismatch + observed=EXAMPLE_NONCE + recoverable_value` → `false`；
   - `JsonNonceMismatch + 普通错误 nonce + recoverable_value` → `true`；
   - `MissingEndTag + recoverable_value` → `true`；
   - `NonceMismatch` → `false`。
3. 增加 Design reviewer 的异步驱动回归：真实 start nonce、JSON nonce 为 `EXAMPLE_NONCE`、带 `must_fix` finding；断言 provider 仅启动一次、`ReviewGate::UserTriageRequired`、`repair_attempted=false`。
4. 至少覆盖两个 driver：普通 `drive_review_session` 与 logical `drive_review_session_via_gateway`，避免两套条件分支日后漂移。
5. 回归现有 `review_structured_output_repair_succeeds_for_all_general_workspace_types`，确保 `MissingJsonNonce` 修复仍覆盖 Story/Design/WorkItem。

---

### 2. WP-2 用户反馈修订入口：**有条件通过；不建议无条件注入 `compact_history`**

**结论**

Design 分支应补齐：

1. `append_missing_context_notes_to_prompt`；
2. `append_author_artifact_output_contract(..., true)`；
3. Design skeleton；
4. 将 skeleton 防照抄文案从泛化的 `REQ/AC` 改成 Design 实际契约：`DEC/CMP/API + source id`。

但不建议在 `build_author_revision_prompt(Design)` 中**无条件**塞入 `compact_history`。推荐仅在 fresh author turn 注入，resume turn 不注入；或者本轮先不加 compact history，只加 schema/fence/notes/skeleton。

**证据**

- 当前 `src/product/workspace_engine/prompts/author_revision.rs:7-27` 仅包含当前 artifact、反馈、增量修订规则和改动摘要：
  - 没有 schema；
  - 没有 artifact fence 规则；
  - 没有 Design skeleton；
  - 没有 missing context notes；
  - 没有 compact history。
- `src/product/workspace_engine/prompts/revision.rs:125-153` 的 full revision 已经证明 schema、history、notes、artifact contract、skeleton 可以组合使用。
- 但 author feedback 路径在 `build_revision_input_with_resume` 中仍计算并传递 `resume_provider_session_id`（`revision.rs:34-47, 58-69`）。
  - 对 Kimi/Pi/其他 resume provider，服务端历史可能已存在；
  - `compact_history` 仅是客户端 prompt 瘦身，且没有绝对字符/token 上限：最近两轮保留原文，异常时 fail-closed 回放全历史；
  - 将 compact history 再塞进 resume prompt，可能与服务端上下文和当前 artifact 形成重复。
- 当前 Story campaign 的 manifest 对全部 30 个样本仍报告 usage 缺失；现有证据无法量化 Kimi/Pi 的真实 input-token 余量。
- 当前入口用三反引号包裹“当前产物”。Design artifact 内含 ` ```json ` 等代码块时，输入 fence 本身可能提前闭合；`part_04.rs:659-712` 已表明 Design artifact 有内嵌代码块是现实场景。

**建议**

最终注入清单：

|内容|建议|原因|
|---|---|---|
|Design parser schema|并入|直接降低 heading/ID/source 漏失|
|artifact 输出 fence 契约|并入|当前入口缺失，且能处理内嵌代码块外层四反引号规则|
|Design skeleton|并入|低静态成本，保留 anti-copy 约束|
|missing context notes|并入|上下文缺口会直接影响 Design 决策|
|compact history|仅 fresh；resume 不注入|避免与 provider 原生历史、当前 artifact 双重重放|
|完整 routing reference|不并入|超出 WP-2，且容易将反馈修订重写成另一套完整 author 入口|

实现上可令 `build_revision_input_with_resume` 把 `resume_provider_session_id.is_none()` 作为参数传入 Design feedback prompt，而不是让 `build_author_revision_prompt` 自行猜测会话模式。

同时建议把“当前产物”输入 fence 改为对 Design 使用四反引号，或改为无 fence 的明确 markdown 区段；否则补了输出 fence 后，输入侧仍有歧义。

**测试建议**

- Design feedback prompt：含 schema、`DEC/CMP/API`、source、outer artifact fence、四反引号规则、Design skeleton、missing notes。
- Story 与 WorkItem：断言保持当前 prompt 内容，不意外拥有 Design schema/skeleton。
- Design artifact 含三反引号代码块：输入边界仍完整，用户反馈未被纳入 artifact 区。
- resume Design feedback：不重复 compact history；fresh Design feedback：若最终决定注入，则有 compact history。
- 提示词长度只记录为观测指标，不建立凭字符数“达标”的虚假 gate；真实 campaign 应记录 provider usage。

---

### 3. WP-1 `reviewer_output_contract` 增参与在 `build_review_input` 末尾追加：**选增参方案**

**结论**

选择 `reviewer_output_contract` 增参方案。其“6 个生产调用点 + 3 个已有单测调用点”的盘点完整，没有遗漏。

**证据**

`rg -n 'reviewer_output_contract\(' --glob '*.rs' .` 得到：

- 1 个函数定义；
- 6 个生产调用：
  1. `build_review_input`，`review.rs:82`；
  2. `build_work_item_plan_review_input`，`:285`；
  3. `build_projection_plan_review_input`，`:413`；
  4. `build_work_item_plan_outline_review_input`，`:598`；
  5. `build_work_item_batch_review_input`，`:668`；
  6. `build_work_item_draft_review_input`，`:811`。
- 3 个已有单测调用，均在 `src/product/workspace_engine/prompts.rs`：
  1. `reviewer_output_contract_legacy_uses_on_demand_generation_reference`；
  2. `author_skeletons_are_gate_incomplete_and_reviewer_example_uses_an_unissued_nonce`；
  3. `reviewer_output_contract_logical_declares_policy_envelope`。

总引用为 10，和“定义 1 + 生产 6 + 测试 3”一致。

**建议**

- 新增 `prompts/reviewer_boundary_examples.rs`：
  ```rust
  pub(crate) fn reviewer_boundary_examples_for(
      workspace_type: &WorkspaceType,
  ) -> &'static str
  ```
  仅 `WorkspaceType::Design` 返回三个判例，其余返回空串。
- 将参数插入 `reviewer_output_contract` 的完整示例和真实 nonce 模板之间，保持顺序：
  1. 通用 `EXAMPLE_NONCE` 输出结构；
  2. Design 边界判例；
  3. 真实 nonce 输出模板。
- 在全部 6 个调用点传该参数。WorkItemPlan 的五个调用点虽然必为非 Design，传空串仍有价值：共享函数签名完整、未来新增 Design 子入口不会静默遗漏。

不建议“在 `build_review_input` 最末尾追加”：

- 该替代虽能覆盖当前普通 Design 路径，却落在真实 nonce 输出模板之后，破坏提案要求的定位；
- 只覆盖 general review，不自然表达共享 contract 的扩展点；
- 未来若 reviewer 入口变化，易出现 Design 判例位置漂移；
- 测试只能验证某个 builder 的尾部字符串，不能验证 contract 层的顺序不变量。

**测试建议**

新的 `design_reviewer_boundary.rs` 可做 5 组 contract-driven 断言：

1. Design prompt 中判例位于 `EXAMPLE_NONCE` block 后、真实 nonce block 前；
2. Story、WorkItem 与全部 WorkItemPlan reviewer prompt 不含 Design 判例；
3. 抽象 `[DEC-*] -> [REQ-*]/[AC-*]` 判例明确是 `suggestion`/允许 pass，而非 `must_fix`；
4. 测试计划、命令、测试文件或组件测试职责分派判例明确是 `must_fix`/`revise`；
5. 风险章节仅描述验证风险、未落入可执行测试计划时允许 `pass`。

注意：这些单测只能证明**提示词契约及 parser/gate 对给定输出的解释**，不能证明弱模型必然完成分类；真实分类准确率必须由 WP-4 campaign 统计。

---

### 4. WP-3 结构契约回归矩阵：**值得做，但“纯测试”边界需澄清**

**结论**

7 heading、三族 ID、source、禁止项的表驱动负例与 Design 多轮滑窗 fixture 应并入，且可优先执行。D-07 不应随 WP-3 一并落地。

**证据**

- `artifact_constraint_spec_for(Design)` 已有：
  - 7 个 required headings；
  - `[DEC-*]`、`[CMP-*]`、`[API-*]`；
  - source traceability；
  - Work Item Plan / 任务拆分 / 开发任务 / checklist 和 `[TASK-*]`、`WI-*` 禁止项。
- 现有 `part_12.rs` 覆盖了编号 heading 与 legacy heading；`part_31.rs` 覆盖 prompt schema gate；但没有一张按所有 Design 条目逐项缺失/命中的回归矩阵。
- `part_32.rs` 的滑窗 fixture 主要是 Story，Design 尚未验证其多轮 decision、source、强 finding 与相邻版本 diff 的保留。
- skeleton 当前泛称“缺稳定 ID、REQ/AC 与追踪 token”，但 Design 实际 gate 并不要求 `REQ/AC`，应改为 `DEC/CMP/API/source id`。

**建议**

- WP-3 的表驱动用例应采用“单个失败原因”的基准 Design artifact：每次只删除一个 heading/ID/source，或只加入一个禁止项，确保失败归因稳定。
- 多轮 fixture 至少断言：
  - 早轮 raw history 不重放；
  - 最近两轮、用户 choice audit、最新 Design artifact、未关闭 `must_fix` 完整保留；
  - `DEC/CMP/API` 和 source 追踪未被摘要误删；
  - reviewer 的强 finding 不重复出现两次。
- skeleton 文案修正不完全是“纯测试”：现有 `prompts.rs` 测试按完整固定 prefix `strip_prefix`，改文案时应同步重构该测试，避免将文本偶然格式当作接口。

---

### 5. WP-4 corpus / golden / campaign：**方向正确，工作量和 gate 严格度被低估**

**结论**

应做，但不能直接复制现有 Story campaign 的 validator 后宣称可信 release gate。建议拆成“可机读基础设施 + baseline”与“改造后 revised campaign”两阶段。

**证据**

现有 `cadence/reports/story-weak-model-campaign/validate_manifest.py` 不能直接作为严格模板：

- `SCHEMA_PATH` 已声明但实际未使用；
- 文档称支持 `--paired`，argparse 中并没有该参数；
- `model`、`model_version`、`strategy` 都是可选项；
- usage 缺失仅 warning；
- 去重键不包含 model/version/strategy；
- 不校验 corpus/golden digest；
- 当前 Story manifest 的 30 个样本全部缺 model、model_version、strategy、usage，validator 仍以 exit 0 通过。

因此“本次补齐字段”不足够；应把字段改为必填，完善 pair、digest、usage 规则并补 Python 单测。

**建议**

1. 六种 Design corpus 先冻结 upstream Story Spec fixture、source ID、digest、单仓范围标记；不要让 runner 运行时生成上游 Story。
2. manifest 至少区分：
   - author/reviewer provider；
   - author/reviewer model 与版本；
   - provider/CLI 版本；
   - fresh/resume；
   - author/reviewer 分别的 usage；
   - retry、超时、用户 choice、最终 reviewer verdict；
   - 单仓断言结果。
3. normalizer 必须覆盖：
   - heading；
   - `DEC/CMP/API`；
   - `DEC -> REQ/AC` link；
   - source coverage；
   - `设计决策` 与 `追踪关系` 双章节 decision 的合并；
   - fenced code 不计入 ID/heading；
   - 重复或冲突 ID fail closed。
4. 不要把“与某一个 canonical Design 的完全相同 DEC/CMP/API 集合”作为弱模型 gate；设计方案可合法多样。更稳妥的是比较上游 REQ/AC coverage、必需 source、禁止项、稳定 ID 与明确决定约束。
5. 边界分类若仅为“辅助信号”，它不能证明 WP-1 达标。建议定义为独立的最低门槛：抽象 traceability 不得出强返修；可执行测试越界必须出 `must_fix`。
6. baseline 必须在 prompt 改造前采集；若无 baseline，报告只能说明 revised 表现，不能说明改造收益。
7. 真实 provider campaign 成本高，建议运行在隔离项目/数据目录，保留脱敏原始证据与超时分类，不把临时 Issue/session 混入仓库 fixture。

---

## R2 发现的提案漏洞或遗漏

1. **`EXAMPLE_NONCE` repair 风险的错误码归因错误。**  
   完整复制示例是 `NonceMismatch + recoverable_value=None`，当前已不会 repair；应防的是 `JsonNonceMismatch` 中 JSON nonce 复制示例、start tag 却是真实 nonce 的情况。

2. **“单仓 Design”与“明确不做 aggregate 分支”必须写成硬范围。**  
   既有分析已确认 aggregate Design 存在更高优先级问题：request-bound nonce 没有贯穿返修、metadata 从 artifact markdown 重新发现、raw/fenced fallback、`change_order` exact-set 未闭环。此次不做 aggregate 可以成立，但 proposal、campaign 报告和验收标题不得声称“Design 链路整体可用”。

3. **WP-2 当前 artifact 的三反引号输入 fence 是独立风险。**  
   Design 中很常见 JSON、代码片段、命令；当前 prompt 内层三反引号会造成输入边界歧义。只补输出 fence 不能消除该问题。

4. **`compact_history` 并不等价于硬 token budget。**  
   它保留最近两轮原文、异常时全量回放，且 author feedback 可能 resume 原生会话；不能用“已压缩”推导 Kimi/Pi 安全预算。

5. **D-07 不是低风险 parser 收紧。**  
   当前 `normalize_workspace_heading_line` 接受 1 至 6 级 heading；required ID/source 检查对 fenced 内容也非完全隔离。又有 `work_item_split_engine/context.rs` 复用 heading 规则。若要求“仅新生成 artifact 生效”，需引入明确的生成来源/版本边界，不能直接改通用 validator。

6. **现有 reviewer prompt 对未关闭强 finding 可能双重重放。**  
   `compact_history` 在多轮压缩成功时已追加一次，`build_review_input` 又无条件追加一次。不能简单删第二处：轮数不足或 compaction fallback 时第一处未必存在；需要显式返回“已包含”信息后再去重。

7. **campaign 的“3 组合 × 10 样本”不清楚 strategy 维度。**  
   若 fresh/resume 都是比较维度，10 个样本无法同时做到六类语料均衡、覆盖多轮、又支持 strategy 对比。应把 primary fresh one-shot gate 与 resume usage 观察样本分开统计。

8. **单一 `provider/model/version` 字段不足以表达 author/reviewer 组合。**  
   真实 Design campaign 需要分别记录 author 与 reviewer 的 provider/model/version；否则 reviewer 边界分类失败无法归因。

9. **5 个 few-shot 单测不等于模型语义验收。**  
   应在报告中明确：静态单测锁位置、文本和 parser 行为；实机 campaign 才统计 false positive/false negative。

---

## R3 实施顺序与里程碑建议

### M0：范围和验收口径冻结

- 明确本期只覆盖 legacy 单仓 Design，不处理 aggregate metadata/change order。
- 接受 D-05，拒绝本期 D-07 和 finding 双重重放去重。
- 定义“full-chain”对预期 `revise` 样本的口径，避免把正确识别测试越界误记成失败。

### M1：先建可比较基线（WP-4A）

- 冻结六类 corpus、上游 Story Spec fixture、digest。
- 实现 Design normalizer、严格 manifest validator 及 Python 单测。
- 运行 baseline；此阶段只记录结果，不设置 release gate。

### M2：结构性回归先行（WP-3）

- 建立 Design 7 heading / 三族 ID / source / 禁止项表驱动矩阵。
- 补 Design 多轮滑窗 fixture。
- 修正 Design skeleton anti-copy 描述。
- 加入 D-05 单仓确认红线。

### M3：reviewer 可信边界（WP-1）

- 先加 `EXAMPLE_NONCE` 的 repair fail-closed 测试；
- 再收紧 `is_repairable`；
- 加 Design boundary examples module；
- 调整 6 个 contract 调用和 3 个既有测试调用；
- 加 5 组 Design contract-driven test。

### M4：用户反馈修订入口（WP-2）

- 仅给 Design 分支加 notes、schema/fence contract、skeleton；
- compact history 仅 fresh 注入或暂缓；
- 修正 artifact 输入 fence；
- 锁 Story/WorkItem 不变。

### M5：revised campaign 与 gate（WP-4B）

- 运行 revised cohort；
- 输出 author/reviewer/full-chain、边界分类、retry、usage、超时与工具可用性；
- 只有 baseline/revised、digest、manifest、golden 都完整时才给出可用性结论。

---

## R4 我对决策项的立场

|决策项|立场|理由|
|---|---|---|
|D-05 单仓确认红线测试|**并入**|低成本纯回归，且当前直接覆盖的是单仓 Design prompt contract、单仓 Story provider drive；单仓 Design 最终确认红线值得补齐。|
|D-07 二级 heading / 非空 ID / fenced code 不计入|**不并入**|通用 validator 和下游 Design context 有共享影响；“仅新生成 artifact”需要版本/来源边界，不是 P2 小改。|
|finding 双重重放去重|**默认不并入**|确有 token 冗余，但简单删除会在非压缩路径丢失强 finding；应在 campaign usage 证明问题后另立共享修复。|
|campaign 10 vs 12|**选 12**|六种语料可每类 2 次；在 95% 门槛下 10 和 12 都要求零失败，12 的覆盖更均衡。建议为每个 provider 组合 12 个 fresh 主 gate 样本，resume 单独作为多轮 usage cohort。若对外宣称与 Story 同级 release gate，则应采用既有 Story 规格的 20/组合，而非 10 或 12。|

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "按只读约束完成实现可行性与影响面评审；未修改文件、未执行 git 写操作，结论将范围限定为单仓 Design。"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "codegraph status；ast-grep outline 目标 Rust 文件",
      "result": "passed",
      "summary": "CodeGraph 索引为最新状态（1,488 files）；已按仓库规则进行结构化阅读。"
    },
    {
      "command": "rg -n 'reviewer_output_contract\\(' --glob '*.rs' .",
      "result": "passed",
      "summary": "确认 1 个定义、6 个生产调用、3 个既有单测调用，共 10 个引用。"
    },
    {
      "command": "rg -n '\\.is_repairable\\(' src tests；阅读 structured_output.rs、review/drive.rs、review_repair.rs",
      "result": "passed",
      "summary": "确认 is_repairable 仅由 Workspace reviewer 的普通与 logical gateway 双驱动消费。"
    },
    {
      "command": "cargo test --locked --lib review_structured_output_repair_succeeds_for_all_general_workspace_types",
      "result": "passed",
      "summary": "1 passed；现有 Story/Design/WorkItem 正常结构化 repair 回归通过。"
    },
    {
      "command": "cargo test --locked --lib rejects_copied_example_nonce",
      "result": "passed",
      "summary": "1 passed；完整 EXAMPLE_NONCE 复制被 parser 拒绝。"
    },
    {
      "command": "cargo test --locked --lib author_revision_prompt_includes_feedback_and_changelog_section",
      "result": "passed",
      "summary": "1 passed；当前用户反馈修订 prompt 基线行为通过。"
    },
    {
      "command": "cargo test --locked --lib reviewer_prompts_render_parser_derived_schema_gate",
      "result": "passed",
      "summary": "1 passed；现有 Story/Design/WorkItem reviewer schema gate 注入通过。"
    },
    {
      "command": "python3 cadence/reports/story-weak-model-campaign/validate_manifest.py cadence/reports/story-weak-model-campaign/gate-manifest.json",
      "result": "passed",
      "summary": "exit 0，但输出 30/30 缺 model/model_version/strategy/usage 的 warning，作为 WP-4 validator 缺口证据。"
    },
    {
      "command": "python3 -m unittest cadence/reports/story-weak-model-campaign/test_golden_diff.py",
      "result": "passed",
      "summary": "2 passed。"
    },
    {
      "command": "git status --porcelain=v1；git diff --cached --name-only；git diff --name-only",
      "result": "passed",
      "summary": "均无输出；工作树无未暂存或暂存文件。"
    }
  ],
  "validationOutput": [
    "未改动工作树。",
    "已确认 EXAMPLE_NONCE 完整复制当前不会触发 repair；实际需防护的是带真实 start nonce 的 JsonNonceMismatch。",
    "已确认 Story campaign validator 当前为 warning-only，不能直接充当严格 Design gate。"
  ],
  "residualRisks": [
    "aggregate Design nonce/metadata/change_order exact-set 缺口仍存在，且本提案明确不覆盖。",
    "Kimi/Pi 的真实 usage 未被现有 campaign 记录，WP-2 不能以字符数证明预算安全。",
    "静态 few-shot 测试不能证明弱模型边界分类准确率，仍需实机 campaign。"
  ],
  "noStagedFiles": true,
  "diffSummary": "只读评审任务；无代码、测试或文档差异。",
  "reviewFindings": [
    "blocker: 提案对 NonceMismatch 的自动 repair 路径判断不准确；应测试并拦截 JsonNonceMismatch 的 EXAMPLE_NONCE 情形。",
    "blocker: 若不明确排除 aggregate Design，当前 proposal 会对 Design 全链路可用性作出过度承诺。",
    "warning: WP-4 不应直接复制现有 Story validator；其配对、schema、usage 和必填字段校验不完整。"
  ],
  "manualNotes": "建议以 M0-M5 里程碑执行；D-05 并入，D-07 与 finding 双重重放去重后置。"
}
```