# Provider Prompt 项目规则引用 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** 让所有流程型 Provider prompt 强制直接读取当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md，不再依赖外部 Cadence-skills 绝对路径或知识库禁令。

**Architecture:** 保留 direct_cadence_routing_rules_reference() 作为唯一共享入口，只替换其静态文本；Workspace、Coding、Tester、Work Item Draft、Web Workspace Context 与运行时模板将自动继承新要求。先把全套 prompt 契约断言切换为新语义取得红灯，再仅修改集中入口使它们转绿。

**Tech Stack:** Rust 2024、Cargo --locked、既有 Rust 单元/集成测试、OpenSpec CLI、ripgrep。

## Global Constraints

- Change 为 use-project-rules-in-prompts；工作包对应 project-rule-aware-prompts 的两个 requirement。
- 只把规则依据改为当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md；不得新增外部绝对路径、规则正文复制、Provider turn、API、存储或依赖。
- 新共享文本固定使用 [cadence_project_rules] 和既有 [cadence_rule_read_gate]；必须要求 Provider 用原生文件读取能力读取两份项目文件，任一文件或工具不可用时只报告阻塞。
- 新共享文本不得包含 Cadence-skills/、KnowledgeBase、唯一流程权威 或旧的 [cadence_original_routing_rules] 标签。
- 保留各类 prompt 既有业务阶段、Aria gate、输出 schema 与 JSON 输出约束；只更新其共享规则断言。
- 当前 worktree 有用户原有未提交改动，特别是 src/web/workspace_context/tests.rs；只叠加本计划列出的断言变更，禁止 reset、checkout、stash 或暂存无关文件。
- Rust 命令必须使用宿主机 Cargo 与 --locked，任何 Cargo 命令禁止 -j 1。
- 只有所有验证通过后才能勾选 Change tasks；精确实施 Plan 不得扩大 OpenSpec 已确认范围。

---

### Task 1: 先将所有生命周期 prompt 契约切换为项目规则语义（RED）

**对应 OpenSpec 工作包：** 1.2、2.1、2.2。

**Files:**

- Modify: src/product/cadence_skills/routing_reference.rs:1-81
- Modify: src/product/workspace_engine/tests/part_10.rs:190-430
- Modify: src/product/workspace_engine/tests/part_16.rs:1-50
- Modify: src/product/workspace_engine/tests/part_31.rs:2-347
- Modify: src/web/workspace_context/tests.rs:157-190
- Modify: src/product/work_item_split_engine/tests/prompt_contract.rs:348-375
- Modify: src/product/work_item_split_engine/tests/part_01.rs:119-145,758-822
- Modify: src/product/coding_workspace_engine/tests/parser_prompt.rs:159-180,222-238,340-354,419-428
- Modify: src/product/coding_workspace_engine/tests/parser_prompt/plan_defect_prompt.rs:4-49
- Modify: src/product/coding_workspace_engine/tests/provider_execution_context.rs:400-416
- Modify: src/product/tester_agent_loop/tests.rs:92-137
- Modify: tests/it_core/context_builder.rs:82-100,360-380
- Modify: tests/it_core/runtime_prompt_routing.rs:6-88

**Interfaces:**

- Consumes: 不变的 direct_cadence_routing_rules_reference() -> &'static str，以及各 prompt 构造器已有的共享插入点。
- Produces: 断言统一要求 [cadence_project_rules]、AGENTS.md、CLAUDE.md 和阻塞语义；断言不再依赖外部规则文件名或路径。

- [ ] **Step 1: 将共享引用的单元测试先改为新契约**

删除外部路径常量及三个旧测试，改写为下列两个测试；此时 production 常量仍是旧文本，因此首个测试必然失败。

~~~rust
#[test]
fn direct_reference_requires_current_project_rule_files_and_fails_closed() {
    let prompt = direct_cadence_routing_rules_reference();

    assert_eq!(
        prompt,
        concat!(
            "[cadence_project_rules]\n",
            "当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据，必须直接读取并遵守其中适用规则。\n",
            "[cadence_rule_read_gate]\n",
            "开始任务前必须使用当前 Provider 可用的原生文件读取工具完整读取 AGENTS.md 和 CLAUDE.md；不得以本 prompt 摘要、内部状态机、外部规则或声称已读替代工具读取。\n",
            "任一文件或工具不可用时，只报告阻塞；不得继续输出候选 artifact、代码、审查结论或 JSON。\n",
        )
    );
}

#[test]
fn direct_reference_excludes_external_and_knowledge_base_constraints() {
    let prompt = direct_cadence_routing_rules_reference();

    for forbidden in ["Cadence-skills/", "KnowledgeBase", "唯一流程权威"] {
        assert!(!prompt.contains(forbidden), "unexpected {forbidden}: {prompt}");
    }
}
~~~

- [ ] **Step 2: 将 Workspace 和 Web Workspace Context 断言切换为项目文件**

在 part_31.rs 的 initial author、system context reuse 与 full revision 三组断言中，把唯一出现次数替换为 [cadence_project_rules]，并以如下组合作为规则契约。对 part_10.rs、part_16.rs 和 src/web/workspace_context/tests.rs 的 author、reviewer、revision workflow 断言应用同一替换；repair 断言改为 !prompt.contains("[cadence_project_rules]")，以保持“不得重启路由生命周期”的原语义。

~~~rust
assert!(prompt.contains("[cadence_project_rules]"), "{prompt}");
assert_eq!(prompt.matches("[cadence_project_rules]").count(), 1, "{prompt}");
assert!(prompt.contains("AGENTS.md") && prompt.contains("CLAUDE.md"), "{prompt}");
assert!(!prompt.contains("Cadence-skills/"), "{prompt}");
~~~

- [ ] **Step 3: 将 Coding、Tester 与 Work Item Draft 断言切换为项目文件**

在 parser_prompt.rs 的 coder、delta、lifecycle 与 compiled context 测试，parser_prompt/plan_defect_prompt.rs，provider_execution_context.rs，tester_agent_loop/tests.rs，以及 work_item_split_engine/tests/prompt_contract.rs 和 part_01.rs 中，删除成对的外部路径检查，增加以下断言。保留现有业务阶段、schema、测试计划、gate 和 Work Item Draft 预算断言；质量预算测试仍须断言 canonical_field_contract、verification_plan、operational_gate、直接依赖合同和 ARIA_STRUCTURED_OUTPUT。

~~~rust
assert!(prompt.contains("[cadence_project_rules]"), "{prompt}");
assert!(prompt.contains("AGENTS.md"), "{prompt}");
assert!(prompt.contains("CLAUDE.md"), "{prompt}");
assert!(!prompt.contains("Cadence-skills/"), "{prompt}");
~~~

- [ ] **Step 4: 将运行时模板的真实渲染测试切换为项目文件**

在 tests/it_core/runtime_prompt_routing.rs 和 tests/it_core/context_builder.rs 的 planning 与 P4 Provider node 循环中保留节点、阶段和 section 完整性检查，替换外部路径检查为：

~~~rust
assert!(prompt.contains("[cadence_project_rules]"), "{node_id}: {prompt}");
assert!(prompt.contains("AGENTS.md") && prompt.contains("CLAUDE.md"), "{node_id}: {prompt}");
assert!(!prompt.contains("Cadence-skills/"), "{node_id}: {prompt}");
~~~

这覆盖 runtime_units/prompt_template_registry.rs 的 N04、N05、N07、N11 及 generic 节点，无需复制或修改模板实现。

- [ ] **Step 5: 运行新断言并确认旧集中常量导致 RED**

Run the following commands:

~~~bash
cargo test --locked --lib direct_reference_requires_current_project_rule_files_and_fails_closed
cargo test --locked --lib initial_author_inputs_directly_route_every_workspace_artifact_type
cargo test --locked --lib workspace_author_workflows_directly_reference_cadence_routing_rules
cargo test --locked --lib coding_prompt_requires_material_driven_execution_without_fixed_stack_terms
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
cargo test --locked --lib work_item_plan_outline_prompt_includes_runtime_contracts
cargo test --locked --test it_core runtime_prompts_directly_reference_rules_and_declare_their_actual_stage
~~~

Expected: 每条命令因 prompt 尚未包含 [cadence_project_rules]、AGENTS.md 或 CLAUDE.md 而失败；失败不得来自编译错误、无关测试或外部 Provider 调用。

### Task 2: 以单一集中片段实现项目规则读取并使所有契约转绿

**对应 OpenSpec 工作包：** 1.1、2.1、2.2。

**Files:**

- Modify: src/product/cadence_skills/routing_reference.rs:1-81
- Test: Task 1 列出的所有单元和集成测试文件。

**Interfaces:**

- Consumes: 全部既有调用点的 direct_cadence_routing_rules_reference()。
- Produces: 不变的 &'static str 返回类型；每个调用点无需改动便获得同一段项目规则读取要求。

- [ ] **Step 1: 用项目相对规则文本替换唯一集中常量**

删除 AGENT_ROUTING_KERNEL_RULE_PATH 与 OPENSPEC_SUPERPOWERS_WORKFLOW_RULE_PATH，并将函数体中的 REFERENCE 精确替换为：

~~~rust
pub(crate) fn direct_cadence_routing_rules_reference() -> &'static str {
    const REFERENCE: &str = concat!(
        "[cadence_project_rules]\n",
        "当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据，必须直接读取并遵守其中适用规则。\n",
        "[cadence_rule_read_gate]\n",
        "开始任务前必须使用当前 Provider 可用的原生文件读取工具完整读取 AGENTS.md 和 CLAUDE.md；不得以本 prompt 摘要、内部状态机、外部规则或声称已读替代工具读取。\n",
        "任一文件或工具不可用时，只报告阻塞；不得继续输出候选 artifact、代码、审查结论或 JSON。\n",
    );

    REFERENCE
}
~~~

不要更名函数或修改任一 prompt 构造器；workspace_engine 对 system context 的包含检查仍以完整返回字符串判断，因此无需新增分支。

- [ ] **Step 2: 运行聚焦测试确认 GREEN**

Run the following commands:

~~~bash
cargo test --locked --lib direct_reference_
cargo test --locked --lib initial_author_inputs_directly_route_every_workspace_artifact_type
cargo test --locked --lib initial_author_reuses_system_routing_reference_for_every_workspace_type
cargo test --locked --lib full_revision_prompt_does_not_repeat_schema_from_generation_context
cargo test --locked --lib workspace_author_workflows_directly_reference_cadence_routing_rules
cargo test --locked --lib failed_review_comments_never_enter_story_design_or_work_item_revision_prompts
cargo test --locked --lib coding_prompt_requires_material_driven_execution_without_fixed_stack_terms
cargo test --locked --lib coding_lifecycle_protocols_reuse_the_canonical_cadence_routing_reference
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
cargo test --locked --lib work_item_plan_prompts_keep_json_contract_without_markdown_schema
cargo test --locked --lib realistic_chinese_serial_prompt_stays_within_quality_budget
cargo test --locked --test it_core runtime_prompts_directly_reference_rules_and_declare_their_actual_stage
cargo test --locked --test it_core context_builder_renders_each_planning_prompt_and_maps_adapter_input
cargo test --locked --test it_core context_builder_renders_p4_provider_nodes_and_rejects_missing_required_inputs
~~~

Expected: 全部通过，且 Work Item Draft 预算断言保持原有阈值与行为；不启动真实 Provider，也不执行 Git push。

- [ ] **Step 3: 格式化并提交实现与契约测试**

Run the following commands:

~~~bash
cargo fmt
git diff --check -- src/product/cadence_skills/routing_reference.rs src/product/workspace_engine/tests src/web/workspace_context/tests.rs src/product/work_item_split_engine/tests src/product/coding_workspace_engine/tests src/product/tester_agent_loop/tests.rs tests/it_core
~~~

然后先用 git diff --cached --name-only 确认暂存区没有无关文件，再只暂存本 Task 列出的 source/test 文件并提交：

~~~bash
git add src/product/cadence_skills/routing_reference.rs src/product/workspace_engine/tests/part_10.rs src/product/workspace_engine/tests/part_16.rs src/product/workspace_engine/tests/part_31.rs src/web/workspace_context/tests.rs src/product/work_item_split_engine/tests/prompt_contract.rs src/product/work_item_split_engine/tests/part_01.rs src/product/coding_workspace_engine/tests/parser_prompt.rs src/product/coding_workspace_engine/tests/parser_prompt/plan_defect_prompt.rs src/product/coding_workspace_engine/tests/provider_execution_context.rs src/product/tester_agent_loop/tests.rs tests/it_core/context_builder.rs tests/it_core/runtime_prompt_routing.rs
git commit -m "fix: use project rules in provider prompts"
~~~

### Task 3: 执行全量质量门禁并收束 OpenSpec 工作包

**对应 OpenSpec 工作包：** 3.1。

**Files:**

- Modify: openspec/changes/use-project-rules-in-prompts/tasks.md:1-16
- Modify: cadence/plans/2026-07-26_计划文档_功能开发_ProviderPrompt项目规则引用_v1.0.md
- Test: Rust 格式、Clippy、检查、全量测试、OpenSpec 严格验证与限定范围文本扫描。

**Interfaces:**

- Consumes: Task 2 已提交的集中规则片段和全部 prompt 契约。
- Produces: 已勾选的 OpenSpec 高层工作包及可追溯的验证证据；不变更运行时接口。

- [ ] **Step 1: 确认旧文本已从运行时和测试范围消失**

Run the following three commands separately; each must return no matches. Exit status 1 from rg is the expected no-match result.

~~~bash
rg -n -F '[cadence_original_routing_rules]' src tests
rg -n -F 'Cadence-skills/' src tests
rg -n -F 'KnowledgeBase 的 manifest 或内容' src tests
~~~

若任一命令打印匹配，先修正该 prompt 或契约断言；不得修改 cadence/plans 中其他人现有的历史计划来伪造扫描结果。

- [ ] **Step 2: 运行项目全量质量门禁**

Run the following commands:

~~~bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
openspec validate use-project-rules-in-prompts --strict
git diff --check -- src/product/cadence_skills/routing_reference.rs src/product/workspace_engine/tests src/web/workspace_context/tests.rs src/product/work_item_split_engine/tests src/product/coding_workspace_engine/tests src/product/tester_agent_loop/tests.rs tests/it_core openspec/changes/use-project-rules-in-prompts cadence/plans/2026-07-26_计划文档_功能开发_ProviderPrompt项目规则引用_v1.0.md
~~~

Expected: 所有门禁成功；如全量测试失败，保留完整失败输出，先按 systematic-debugging 定位是否与本变更相关，不得在没有根因的情况下改动实现。

- [ ] **Step 3: 仅根据获得的绿色证据更新任务并提交文档收尾**

将 openspec/changes/use-project-rules-in-prompts/tasks.md 的 1.1 至 3.1 全部改为 [x]。再次执行 openspec validate use-project-rules-in-prompts --strict，然后只暂存本计划与该 tasks.md：

~~~bash
git add openspec/changes/use-project-rules-in-prompts/tasks.md cadence/plans/2026-07-26_计划文档_功能开发_ProviderPrompt项目规则引用_v1.0.md
git commit -m "docs: complete project rule prompt change"
~~~

## Plan Self-Review

- Spec coverage：Task 1 覆盖项目规则、直接读取、失败关闭和所有受影响生命周期；Task 2 通过单一共享入口实现；Task 3 覆盖外部路径/知识库禁令移除与所有验收门禁。
- Placeholder scan：计划不含未定义的实现留白；每个修改文件、测试名称、命令、期望结果和提交范围均已指定。
- Type consistency：唯一实现接口保持 direct_cadence_routing_rules_reference() -> &'static str；所有消费者继续复用该接口，未引入新类型或调用路径。
