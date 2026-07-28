# 移除 WorkItemHandoff 交接摘要实施计划

## 元信息

- **OpenSpec Change**：`remove-work-item-handoff`
- **Capability**：`work-item-handoff-removal`
- **计划类型**：基线修复
- **版本**：v1.0
- **日期**：2026-07-28
- **版本**：v1.1（评审后修订：新增组完成门禁数据源迁移、reviewer 指令改写、`completion_commit` 保留三项）
- **前置**：本 change 必须先于 `remove-testing-stage` 实施（硬依赖见「与 `remove-testing-stage` 的归属划分」）

## 背景

`WorkItemHandoff`（`src/product/coding_models/plan.rs:49`）是 provider 生成的自然语言交接摘要，与承担运行时契约职责的 `HandoffRevision` 并存。其 provider 生成路径结构性必然失败（prompt 要求汇报 diff 信息却不提供，且 `--tools ""` 禁用工具），失败后静默降级为占位且不留任何执行痕迹；字段绝大部分与 diff / `HandoffRevision` 冗余；却被用作验收依据造成 group final review 假阳性。

## 勘察结论

### 移除范围：88 个文件

统计口径：`WorkItemHandoff|work_item_handoff|handoff_summary|workItemHandoff|handoffSummary|required_handoff_from|planned_handoff_summary|max_handoff_chars|max_dependency_handoffs|work-item-handoff` 在 `src/`、`tests/`、`web/src` 下命中的文件数。

### `WorkItemHandoff` 的 store API（5 个）

`src/product/coding_attempt_store/attempt.rs:210-295`：`save_work_item_handoff`、`save_coding_unit_handoff`、`get_work_item_handoff`、`get_coding_unit_handoff`、`get_visible_work_item_handoff`。路径解析在 `paths.rs:38`（`work_item_handoff_path`）与 `paths.rs:197`（`coding_unit_handoff_path`）。

### 关键消费点（决定移除后 reviewer 的视野）

**一、group final review prompt 的 `Completed Units` 段落**

`reports.rs:30` `collect_completed_group_unit_handoffs` 收集 `(unit, WorkItemHandoff)` 对，`reports.rs:61` `format_group_unit_handoff_section` 渲染为：

```
- Unit / Work Item / Status / Completion Commit
  Handoff Summary: <handoff.summary>
  Tests Run: <handoff.tests_run>
  Risk Notes: <handoff.open_risks>
```

经 `internal_pr_review.rs:63` 注入 prompt 的 `units_section`。这正是 reviewer 看到「Handoff Summary: 占位文本、Tests Run: 无」的确切来源。

移除后该段落必须重建。可用的真实数据源：

| 字段 | 来源 | 实测值 |
|---|---|---|
| `unit.id`、`logical_work_item_id`、`status` | `CodingExecutionUnit` | 真实 |
| `unit.completion_commit` | `CodingExecutionUnit` | 真实（`21cad4ad`、`3b302d3a`） |
| `unit.latest_handoff_revision_id` | `CodingExecutionUnit` | 真实 |
| `unit.summary` | `CodingExecutionUnit` | **通用占位**（「当前 Work Item 已完成」），不可用 |
| `provided_contracts` / `provided_capabilities` | `HandoffRevision` | 真实且详尽 |

因此重建方案：段落保留 unit 标识、状态、completion commit 与 handoff revision id，摘要与测试行替换为该 unit 的 `provided_contracts` 与 `provided_capabilities`。reviewer 手上另有完整 git diff，不损失判断依据。

**二、evaluation context 的交接字段**

`coding_evaluation_context/builder.rs`：
- `:346` 读 `get_visible_work_item_handoff`
- `:352-353` internal reviewer 角色下缺失时压入 `work_item_handoff_missing` 证据告警
- `:383-389` 输出 `handoff_tests_run`、`handoff_test_result_summary`

三处全部移除。`:267` 的 `dependency_handoff_refs`（来自 `resolved_handoff_revision_ids`）属 `HandoffRevision` 体系，保留。

**三、`HandoffRevision` 的 `tests` / `artifacts` 来源**

`group_completion.rs:576-602` `build_group_handoff_revision`：`tests` 来自 `legacy_handoff.tests_run`（`:583`），`artifacts` 来自 `legacy_handoff.files_changed`（`:586`）。同函数的 `provided_contracts` / `provided_capabilities` 来自 `facts`（契约编译），不经摘要。

`group_completion.rs:92-101` 与 `:417-429` 读取摘要并在缺失时报 `WorkItemHandoffMissing`，两处均需移除。

**四、🔴 `artifacts` 的生产消费者：组完成写入范围门禁**

`artifacts` 不是只有生产者。`gates.rs:281-287`：

```rust
if self.schema_v2_group_plan_lineage(attempt)?.is_some() {
    for facts in self.schema_v2_group_completion_gate_facts(attempt)? {
        self.validate_changed_files_for_runtime(
            &facts.runtime,
            &facts.handoff.artifacts,
            worktree_path.as_ref(),
        )?;
    }
}
```

`validate_changed_files_for_runtime`（`gates/schema_v2.rs:110-140`）按 `write_policy.forbidden_scopes` / `exclusive_scopes` 判 `WorkItemDiffScopeViolation`。legacy 路径（`gates.rs:289-305`）用 `handoff.files_changed`，同样在移除范围内。

**两条路径的写入范围校验数据源都会随本变更消失。** 函数体是 `for relative_path in changed_files`，清单为空时零次迭代、恒返回 `Ok(())`——任何越界写入静默放行。这是安全相关的行为退化，必须迁移数据源，不接受门禁失效。

可用来源：
- `changed_files_for_attempt`（`gates.rs:351-369`，走 `git_status --porcelain`）。单 WorkItem 完成门禁（`gates.rs:230-236`）正是这么取的，但它是 attempt 级、无 per-unit 归属。
- per-unit 粒度：`HandoffRevision.commit_sha`（`work_item_revision.rs:195`）与 unit run 的 `completion_commit` 已由 `runtime_handoff_authority.rs:38` 校验一致，因此可按 unit 的 completion commit 取该 unit 的 changed files。`git_workspace_service.rs` 现有 `git_status`（`:195`）、`git_diff_stat`（`:322`）、`git_diff`（`:347`），无按 commit 取文件名清单的现成方法，需新增或复用 `git_diff_stat` 的 `DiffStat.files`。

**首选 per-unit**，保持现有校验语义不变。若实施中确认无法保住 per-unit 粒度，退为 attempt 级对每个 unit runtime 各校验一次——这会放宽归属判定（A unit 的越界写入可能记到 B unit 头上），必须在提交说明中显式记录该退化，不得静默采用。

**五、reviewer 提示词中以摘要为审查对象的指令**

`prompts.rs:346-353` `group_final_review_material_protocol` 多条硬性指令以 unit handoff 为审查对象：
- `:347`「必须确认每个 completed unit 的 handoff 承诺是否体现在最终 diff 或最终报告中」
- `:350`「如果某个 unit 的验证证据缺失、handoff 未闭环、或最终 PR 描述遗漏关键影响，必须 request_changes 或 blocked」
- `:353`「不得用平台默认技术栈假设替代 unit handoff 或 Work Item 内容」

`prompts.rs:325` `code_review_material_protocol` 直接点名「handoff tests_run/test_result_summary」。

**只删数据结构不改这些指令，本 change 的核心动机就落空**：reviewer 会拿到「必须确认 handoff 承诺闭环」的要求配一个没有承诺字段的上下文，假阳性换形式保留。必须同批改写。

**六、`update_work_item_handoff_summary` 不能整体删除**

`lifecycle_store/work_item.rs:177-192` 同时写两个字段：

```rust
record.handoff_summary_ref = handoff_summary_ref;   // 本变更移除
record.completion_commit = completion_commit;        // 必须保留
```

`:188` 是 `completion_commit` 在全仓的唯一写入点，而该字段有独立消费者：`web/handlers/coding.rs:318` 填 `WorkItemDependencyHandoffRef.commit_sha`，前端 `IssueLifecycleWorkbenchParts.tsx:536`、`LifecycleCardDrawer.tsx:631` 渲染。删函数会让 `completion_commit` 永不写入。

**七、`WorkItemHandoffMissing` 的九个触发点分属两个体系**

| 触发点 | 归属 | 处置 |
|---|---|---|
| `group_completion.rs:101`、`gates.rs:241`、`reports.rs:51`、`handoffs.rs:244` | 交接摘要 | 随本变更移除 |
| `runtime_impact.rs:479`、`plan_defect.rs:474`、`plan_defect.rs:487` | `HandoffRevision` 体系 | **保留语义** |
| `group.rs:20` | 既有语义错用（`get_active_coding_unit` 返回 None 时报此错，与交接无关） | 换用语义正确的变体 |

因此**变体本身不能移除**。另注意 `web/error.rs:97` 的 `"work_item_handoff_missing"` 是 API 错误码字符串（对应 `web/handlers/coding.rs:130` 的 `ApiError`），与引擎侧枚举是两套东西。

### 必须保留的 schema v2 标识

`work_item_split_engine` 同时含两类 handoff 命名：

| 标识 | 位置 | 处置 |
|---|---|---|
| `required_handoff_from` | `schema.rs:60`、`types.rs:247` | 移除 |
| `max_handoff_chars`、`max_dependency_handoffs` | 见下方完整落点 | 移除 |
| `handoff_contract` | `schema.rs:282,358` | **保留** |
| `handoff_field` | `schema.rs:243`（`EvidenceKind` 成员） | **保留** |
| `handoff_notes` | `schema.rs:496,515`、`models/outline.rs:97` | **保留**（outline 层，schema required 项） |
| `handoff_strategy` | `schema.rs:524,536`、`outline.rs:51`、`workspace_engine/plan_projection.rs:222`（用作 `split_reason`） | **保留**（同上） |

后两项名字带 handoff 但与交接摘要无关，且都是 splitter schema 的 `required` 项，**误删会让 splitter 输出全部校验失败**。

`max_handoff_chars` / `max_dependency_handoffs` 是 `WorkItemContextBudget` 成员，落点远超 `schema.rs:53,57`：

- `models/lifecycle.rs:79,83`（定义）与 `:91,95`（Default）
- `work_item_split_validator/plan.rs:214,218,234,238`（上限常量与校验分支）
- `web/types.rs:470,474,482,486`
- `web/handlers/dto.rs:370,374`
- `cross_cutting/streaming_provider/fake.rs:276,280`（JSON 字面量，**编译器不报错**）
- `web/src/api/types/common.ts:222,226`
- `web/src/components/lifecycle/LifecycleCardDrawer.tsx:620,624,726`（`:726` 有可见文案「依赖交接上限」）

### 历史持久化数据不做兼容

`HandoffRevision` 持久化于 issue 级 lineage（`work-item-revisions/<plan>/logical-work-items/<wi>/handoff-revisions/`），既有记录含 `tests` 与 `artifacts`。

影响面不止 lineage。`LifecycleWorkItemRecord` 的三个待移除字段 `planned_handoff_summary`、`required_handoff_from`、`handoff_summary_ref`（`models/lifecycle.rs:152,166,174`）均**无** `#[serde(default)]` 保护，移除后既有 work item 记录同样面临反序列化失败，且这类记录数量远大于 handoff revision。

按用户决定，本次以全新系统处置历史数据：**不写迁移、不加 `#[serde(alias)]` 或忽略未知字段的兼容层、不为历史记录写兼容测试**。移除字段后本变更前写入的 lineage 记录与 work item 记录都可能不可读，这是已接受的取舍。

同样地，实施中遇到任何"旧记录含 `testing` / 摘要字段"的场景，一律不加兼容分支。

### 与 `remove-testing-stage` 的归属划分

`handoffs.rs:357-368` 的 `generate_placeholder_work_item_handoff` 在 `:360` 调用 `list_testing_reports`。本 change 整体删除该函数，这个消费者随之消失——这是 A 必须先于 B 的硬依赖（B 若先做，会在移除 `TestingReport` 时撞上一个 Impact 清单外的调用点）。

- 本 change 只消除 `handoffs.rs:360` 一个 `list_testing_reports` 消费者。
- `reports.rs:11`、`reviewer_context.rs:18`、`plan_defect.rs:427`、`web/coding_ws_handler/state.rs:30`、`web/handlers/coding.rs:552` 五处归 `remove-testing-stage`，**本 change 不触碰**。
- `coding_evaluation_context` 的 `handoff_tests_run` / `handoff_test_result_summary`（`builder.rs:383-389`、`mod.rs:59-60`）归本 change；B 不重复处理。

## 实施步骤

### 阶段一：失败测试（工作包 1.1–1.7）

先写测试，此时应全部失败或无法编译。

**1.5 交接发布字段收敛**：在 `work_item_revision_store` 或 group completion 测试中断言发布出的 `HandoffRevision` 只承载 `provided_contracts` / `provided_capabilities` / `contract_hash` / `commit_sha`，不再有测试与产物清单。不写历史记录兼容测试。

**1.3、1.4 契约与权威校验回归**：在 `coding_workspace_engine/tests/` 下断言下游仍获得上游契约与能力、且 commit / revision / status 不一致时仍失败关闭。参照既有 `runtime_handoff_*.rs` 系列测试的夹具。

**1.1、1.2 unit 完成行为**：断言完成后 attempt 目录下不出现 `work-item-handoff.json`、不发生用于生成摘要的 provider 调用、不因摘要缺失失败或降级。

**1.6 启动 coding**：断言上游无摘要引用时不再被拒绝（对应移除 `work_item_handoff_missing`）。

**1.7 schema v2 编译回归**：断言 `handoff_contract` 与 `handoff_field` 仍可用、编译产出契约与能力不变。

**🔴 1.8、1.9 组完成写入范围门禁（最高优先级）**：这两条是本 change 唯一防止安全退化的测试，必须先写、先失败。

- 1.8 越界拒绝：构造某已完成 unit 的实际变更命中 `forbidden_scopes`，断言组完成门禁拒绝并给出 `WorkItemDiffScopeViolation`。**输入必须来自真实 git 状态，不得覆写任何交接摘要字段构造**。
- 1.9 合规放行：各 unit 变更均在 `exclusive_scopes` 内时门禁放行，防止改数据源后误拒。

此二者**替代** `tests/it_product/product_coding_workspace_engine/part_13.rs:569-605` 的 `group_final_confirm_rejects_unit_handoff_outside_exclusive_scope`——原测试靠覆写 `WorkItemHandoff.files_changed` 为 `web/src/app.tsx` 触发违规，模型移除后无法这样构造输入。**不得直接删掉原测试了事**：它是该门禁的唯一回归覆盖，删除前必须先有 1.8 / 1.9 转绿。

**1.10、1.11 reviewer 提示词**：在 `coding_workspace_engine/tests/parser_prompt.rs` 断言两个协议均不含「确认 handoff 承诺闭环」类要求、不点名已移除字段、跨 unit 审查对象为 `HandoffRevision` 契约与能力语义（1.10）；并断言 verdict 取值口径与其他否决依据未被削弱（1.11，防止改写时过度删除）。

**1.12 完成 commit 回归**：断言 `completion_commit` 仍被写入且可经接口读取，对应决策七。

### 阶段二：数据结构（工作包 2.3）

`src/product/models/work_item_revision.rs:187` 移除 `HandoffRevision` 的 `tests` 与 `artifacts`。

此步会引发大量编译错误，用 `cargo check --locked --all-targets` 驱动定位。

验证：1.5 转绿。

### 阶段三：group completion 解耦（工作包 2.4）

`group_completion.rs`：
- `build_group_handoff_revision`（`:576`）移除 `legacy_handoff` 参数与 `tests` / `artifacts` 组装
- `:92-101`、`:417-429` 移除摘要读取与 `WorkItemHandoffMissing` 报错
- 移除 `generate_and_save_work_item_handoff_if_missing` 调用（`:84`）

验证：1.3、1.4 转绿。

### 🔴 阶段三之二：组完成门禁数据源迁移（工作包 2.5）

**必须与阶段二同一提交完成**：`artifacts` 字段一移除，`gates.rs:284` 就没有数据源，中间状态是门禁空转。

`gates.rs:281-305` 两条路径都改：

1. schema v2 路径（`:281-287`）：`facts.handoff.artifacts` 换为按该 unit 的 completion commit 取得的 changed files。`SchemaV2GroupCompletionGateFacts`（`gates/schema_v2.rs:7-10`）已含 `runtime` 与 `handoff`，`handoff.commit_sha` 可直接用；需在 `git_workspace_service.rs` 新增按 commit 取文件名清单的方法，或复用 `git_diff_stat`（`:322`）的 `DiffStat.files`。
2. legacy 路径（`:289-305`）：`handoff.files_changed` 同样换为 git 事实。

**退化预案**：若确认无法保住 per-unit 粒度，退为 `changed_files_for_attempt`（`gates.rs:351`）对每个 unit runtime 各校验一次，并在提交说明中显式写明「per-unit 归属判定放宽」。不得静默采用，也不得因为难做就留空清单。

验证：1.8、1.9 转绿。确认 `part_13.rs:569-605` 的原测试可以安全移除（新测试已覆盖同一门禁）。

### 阶段四：生成路径与 store API（工作包 2.1、2.2）

`handoffs.rs`：移除 `generate_and_save_work_item_handoff_if_missing`、`generate_work_item_handoff`、`generate_work_item_handoff_from_provider`、`generate_placeholder_work_item_handoff` 及 `:533`、`:568` 的调用点。

`handoffs.rs:269-285` 那段写 `handoff_summary_ref` 的 legacy 分支一并移除。

`coding_attempt_store/attempt.rs` 移除 5 个 store API，`paths.rs` 移除 2 个路径解析。

`coding_models/plan.rs:49` 移除 `WorkItemHandoff` 模型。

注意实际调用点是 `handoffs.rs:42`、`:537`、`:572` 三处（不是原先记的 `:533`、`:568` 两处）。

`CodingWorkspaceEngine.provider` 字段**确定要移除**，不需要实施中再确认：该字段在全仓唯一读取点是 `handoffs.rs:328`，移除摘要生成后必为死字段。`with_provider` 构造器（`lifecycle.rs:20`）有 5 个调用点，其中生产代码 1 个（`web/coding_ws_handler/runner/task.rs`），一并调整。若 `CliProviderAdapter` 相关 import 随之未使用，clippy 会报 `unused_imports`。

验证：1.1、1.2 转绿。

### 阶段五：prompt 段落重建（工作包 2.4 延伸）

`reports.rs`：
- `collect_completed_group_unit_handoffs`（`:30`）改为收集 `(unit, HandoffRevision)`
- `format_group_unit_handoff_section`（`:61`）改为渲染 unit 标识、状态、completion commit、handoff revision id、`provided_contracts`、`provided_capabilities`；移除 `Handoff Summary` / `Tests Run` / `Risk Notes` 三行

`internal_pr_review.rs:63` 调用点随签名调整。

此步是本 change 中唯一新增行为的地方，需要为重建后的段落补一个断言测试：段落含契约与能力、不含摘要占位文本。

### 🔴 阶段五之二：reviewer 提示词改写（工作包 2.6）

段落重建（阶段五）只换了数据，指令没换——不做这一步，reviewer 仍被要求「确认 handoff 承诺闭环」，本 change 的核心动机落空。

改写对象：
- `prompts.rs:346-353` `group_final_review_material_protocol` 的 `:347`、`:350`、`:353` 三条以 unit handoff 为审查对象的指令
- `prompts.rs:325` `code_review_material_protocol` 中点名 `handoff tests_run/test_result_summary` 的表述

改写方向：审查对象从「自然语言承诺是否闭环」切换为「`HandoffRevision` 的契约与能力语义是否被下游正确消费」。

**改写限于切换审查对象**。不动 verdict 取值口径，不动除交接摘要外的其他否决依据（`:350` 是三项复合条件——验证证据缺失 / handoff 未闭环 / PR 描述遗漏，只改中间一项，另两项保留）。

验证：1.10、1.11 转绿。1.11 是防止过度删除的护栏。

### 阶段六：evaluation context（工作包 2.12）

`coding_evaluation_context/builder.rs` 移除 `:346`、`:352-353`、`:383-389`（含 `handoff_tests_run` / `handoff_test_result_summary`）与 `mod.rs:59-60` 的 `CoderEvidencePack` 字段定义；保留 `:267` 的 `dependency_handoff_refs`。

此项归本 change，`remove-testing-stage` 不重复处理。

### 阶段七：lifecycle 与 legacy 校验（工作包 2.7、2.8、2.9）

- `src/product/models/lifecycle.rs`、`lifecycle_store/work_item.rs`：移除 `handoff_summary_ref`、`planned_handoff_summary`、`required_handoff_from`。**`update_work_item_handoff_summary` 不整体删除**——摘掉 `handoff_summary_ref` 参数与 `:188` 前一行的赋值，保留 `completion_commit` 写入，按新职责重命名（如 `update_work_item_completion_commit`），并同步其调用点。
- `lifecycle_store/inputs.rs`、`plan.rs`：移除相关输入字段
- `src/web/handlers/coding.rs:117-135`：移除 `work_item_handoff_missing` 前置校验
- `work_item_split_engine/` 及 context budget 全部落点：移除 `required_handoff_from`（`schema.rs:60`、`types.rs:247`）、`max_handoff_chars`、`max_dependency_handoffs`（见「必须保留的 schema v2 标识」一节列出的 7 个额外文件，其中 `fake.rs:276,280` 是 JSON 字面量、`common.ts` 与 `LifecycleCardDrawer.tsx` 是前端，**编译器都不报错**）
- **逐项确认 `handoff_contract`、`handoff_field`、`handoff_notes`、`handoff_strategy` 未被触动**

验证：1.6、1.7、1.12 转绿。

### 阶段八：协议、错误与前端（工作包 2.10、2.11、2.14）

- `src/web/coding_ws_handler/protocol.rs:58`、`src/web/types.rs:559`、`state.rs`：移除协议字段
- `src/web/handlers/dto.rs`、`dto/work_item_runtime.rs`：移除 DTO 字段
- `WorkItemHandoffMissing`（工作包 2.11）：**变体本身保留**。只移除 `group_completion.rs:101`、`gates.rs:241`、`reports.rs:51`、`handoffs.rs:244` 四个摘要触发点；`runtime_impact.rs:479`、`plan_defect.rs:474`、`plan_defect.rs:487` 三处保留；`group.rs:20` 换用语义正确的变体（既有语义错用，属顺带修正）。`web/error.rs:97` 的 API 错误码字符串单独判断。
- `web/src/api/types/`（`coding.ts`、`common.ts`、`lifecycle.ts`、`work-item-plan.ts`）：移除类型。注意前端 `WorkItemHandoff`（`common.ts:185-194`）与后端不同构（多 `handoff_id`、`handoff_summary_ref`、`dependency_handoffs`、`verification_summary`，缺 `files_changed`、`tests_run`），且**无组件消费**——`workItemHandoff` 全仓仅 `coding-workspace-store.ts:111,213,276` 三处加一个断言（`useCodingWorkspaceWs.actions.test.tsx:374`）。因此前端这块比预期简单，仅需清理 store 引用。
- `web/src/state/`（`coding-workspace-store.ts`、`lifecycle-workbench-store.ts`）：移除状态字段与消费
- `web/src/components/`（`IssueLifecycleWorkbenchParts.tsx`、`LifecycleCardDrawer.tsx`、`WorkItemPlanArtifactContent.tsx`）：移除交接摘要渲染。**`IssueLifecycleWorkbenchParts.tsx:536` 与 `LifecycleCardDrawer.tsx:631` 渲染的是 `completion_commit`，保留**。
- 失效测试与夹具（工作包 2.14）：`tests/runtime_handoff_delta.rs:35-36` 的 `coding_runtime_handoff_ignores_commit_tests_and_artifacts_for_unchanged_contract` 在字段移除后失去意义（`compare_handoff_revisions` 本身只读 `contract_hash` 与 `provided_*`，见 `runtime_impact.rs:85-96`，判定口径不变），需删除或改写；`web/test_controls/plan_repair/seed.rs:406-407`、`recovery.rs:542-543` 会编译报错——注意 `src/web/test_controls` 不在 `#[cfg(test)]` 下（`web/mod.rs:14` 无条件 `pub mod test_controls`），属正常编译目标。
- 同步更新前端测试与 test-data / test-utils

注意 `WorkItemPlanArtifactContent.tsx` 当前 814 行，已在 `large_file_guard` 超限清单中；本次为删除，不会加剧。

### 阶段九：边界确认（工作包 2.13、2.15）

逐项确认未改动：

- `HandoffRevision` 的 `provided_contracts`、`provided_capabilities`、`contract_hash`、`commit_sha` 语义
- `runtime_handoff_authority.rs` 的权威校验判定口径
- group completion 的 unit 完成判定、commit 绑定、`HandoffRevision` 发布路径与 ID 派生规则
- schema v2 的 `handoff_contract`、`handoff_field`、`handoff_notes`、`handoff_strategy`
- 组完成门禁的写入范围**判定口径**（`forbidden_scopes` / `exclusive_scopes` 语义），只允许数据源变化
- reviewer 的 verdict 取值口径与除交接摘要外的否决依据
- `completion_commit` 的写入路径完好、消费方可读
- testing 阶段本身（其移除由 `remove-testing-stage` 负责）
- 只消除 `handoffs.rs:360` 一个 `list_testing_reports` 消费者，另五处未触碰
- 未新增历史 `work-item-handoff.json` 的自动清理逻辑

### 阶段十：验证（工作包 3.1、3.2、3.3）

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked --lib
cargo test --locked --test it_core
cargo test --locked --test it_web
cargo test --locked --test it_provider
cargo test --locked --test it_product
cargo test --locked --test it_task_run
cd web && pnpm tsc -b
cd web && pnpm test
```

命令规范见 `cadence/project-rules/build-test-commands.md`（🔴 禁止 `-j 1`）。

**既有失败基线**：`it_core` 的 `large_file_guard` 在本 change 前即失败（9 个文件超限）。该失败会使 `cargo test --locked` 提前终止，因此各集成测试目标需单独运行。验证时须将超限清单与基线逐行比对，确认无新增。

### 阶段十一：审查与交付（工作包 3.4、3.5）

- `openspec validate remove-work-item-handoff --strict`
- 代码审查
- 勾选 tasks 工作包
- 经用户确认后重启后端，由用户验证 group final review 不再因交接摘要缺失判要求修改

## 追溯

| 工作包 | Requirement | 实施阶段 |
|---|---|---|
| 1.1、1.2、2.1、2.2 | 交接摘要不再存在 | 阶段一、四 |
| 1.3、1.4、2.4、2.15 | 交接契约由 HandoffRevision 单一承担 | 阶段一、三、五、九 |
| 1.5、2.3 | HandoffRevision 不再携带测试与产物清单 | 阶段一、二 |
| **1.8、1.9、2.5** | **组完成的写入范围门禁必须保持有效** | **阶段一、三之二** |
| **1.10、1.11、2.6** | **评审不再以交接摘要承诺为审查对象** | **阶段一、五之二** |
| **1.12、2.7** | **work item 完成 commit 记录不受影响** | **阶段一、七** |
| 1.6、2.7、2.8、2.10、2.11 | 交接摘要不再作为验收或前置依据 | 阶段一、六、七、八 |
| 1.7、2.9 | schema v2 契约体系不受影响 | 阶段一、七、九 |
| 2.12、2.13、2.14 | 移除后无死代码与失效夹具 | 阶段四、六、八 |
| 3.1–3.5 | 全部 | 阶段十、十一 |

## 提交建议

按阶段切分，每步保持可编译：

1. `test: cover group completion write scope gate`（阶段一的 1.8、1.9——先落地安全护栏）
2. `test: cover handoff revision field convergence`（阶段一的 1.5、1.12）
3. `refactor!: drop tests and artifacts from handoff revision`（阶段二 + 三 + **三之二**）
4. `refactor!: remove work item handoff summary generation`（阶段四）
5. `refactor: render group review units from handoff revisions`（阶段五 + 五之二 + 六）
6. `refactor!: remove legacy handoff summary references`（阶段七）
7. `refactor!: remove handoff summary from protocol and web`（阶段八）
8. `test: cover work item handoff removal`（阶段一余下测试）

**阶段二、三、三之二必须在同一提交内完成**：`artifacts` 一移除，`gates.rs:284` 立刻失去数据源，中间状态是写入范围门禁空转。不允许分两次提交。

## 风险

1. 🔴 **写入范围门禁静默失效（最高风险）**：`artifacts` 与 `files_changed` 是组完成写入范围校验两条路径的唯一数据源，移除后 `validate_changed_files_for_runtime` 的循环零次迭代、恒返回 `Ok(())`，任何越界写入放行。这是安全退化且不产生任何编译错误或测试失败（原唯一覆盖测试也会因夹具消失而失效）。控制手段：1.8 / 1.9 必须先写先失败；阶段二、三、三之二同一提交；阶段九专项确认判定口径。
2. **改动面大（88 文件）**：编译器可定位大部分 Rust 侧，但下列不报错——`fake.rs:276,280` 的 JSON 字面量、前端 TS 结构类型与字符串联合、`web/error.rs:97` 的错误码字符串。这些必须靠清单逐项核对，不能依赖 `cargo check` 与 `pnpm tsc -b`。
3. **reviewer 指令未同步改写会让本 change 白做**：只删数据结构、保留「必须确认 handoff 承诺闭环」指令，假阳性换形式保留。阶段五之二不可省，且 1.11 是防止改写时过度删除的护栏。
4. **prompt 段落重建是唯一新增行为**：其余均为纯删除。若重建后的段落让 reviewer 判断依据不足，可能改变 group final review 的结论倾向。需要用户实际验证（工作包 3.5）。
5. **历史记录不可读**：`HandoffRevision` 的 lineage 记录与 `LifecycleWorkItemRecord`（三个字段均无 `#[serde(default)]`）都可能反序列化失败，后者数量远大于前者。按用户决定不做兼容，本地遗留数据需重建而非迁移。实施中不得因此临时加入兼容分支。
6. **`update_work_item_handoff_summary` 误删会让 `completion_commit` 永不写入**：该函数是 `completion_commit` 全仓唯一写入点，且有前端消费者。1.12 是这条的回归覆盖。
7. **`WorkItemHandoffMissing` 误整体移除会编译不过**：九个触发点中四个属 `HandoffRevision` 体系，语义必须保留。
8. **legacy 与 schema v2 的 handoff 命名易混淆**：除 `handoff_contract`、`handoff_field` 外，`handoff_notes` 与 `handoff_strategy` 也是 outline 层的 schema required 项，误删会让 splitter 输出全部校验失败。阶段七与阶段九各有一道确认。
