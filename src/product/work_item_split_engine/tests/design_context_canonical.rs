// Bug B 回归：design-context 能力检测器必须识别 Aria 规范 Design Spec 结构。
// 规范 Design Spec 由 workspace_engine parser 强制 设计范围/设计决策/公共组件/API 契约/
// 数据模型/风险/追踪关系 标题（见 artifact_constraints.rs），旧启发式只认
// 架构概览/模块划分/技术选型/测试策略/关键路径 类标题，导致任何规范 design spec
// 都被误判为 5 项上下文全缺失，并把误导性 gaps 注入 outline prompt。


/// 精简自 .aria/projects/project_0001/issues/issue_0001/versions/design_spec_0001/
/// version_0002.json 的规范 Design Spec 结构（保留 parser 强制的全部标题）。
const CANONICAL_DESIGN_SPEC_MARKDOWN: &str = r#"# 设计 Spec：紧凑时长格式化库函数与演示页面（design_spec_0001）

> 关联 Story Spec：story_spec_0001（v2）；Issue：issue_0001（验证专用）

## 设计范围

- 本设计覆盖库函数模块 `src/formatCompactDuration.js`、演示页面 `demo/index.html` 与单元测试。
  - 关联：[REQ-001]~[REQ-003]；source id: `issue_0001#需求1`

## 设计决策

- **[DEC-001] 库函数为无副作用纯函数，输入契约限定非负安全整数**
  - `formatCompactDuration(totalSeconds)` 不依赖 DOM、不读写全局状态。

## 公共组件

- 无跨模块公共组件；`formatCompactDuration` 为唯一命名导出。

## API 契约

- `formatCompactDuration(totalSeconds: number): string`

## 数据模型

- 无持久化数据模型；仅纯函数输入输出。

## 风险

- 浏览器直开 `file://` 限制已在设计范围声明为非目标。

## 追踪关系

- [REQ-001] -> design_spec_0001#DEC-001
"#;

#[test]
fn canonical_design_spec_provides_all_context_capabilities() {
    let capabilities = extract_design_context_capabilities(CANONICAL_DESIGN_SPEC_MARKDOWN);

    assert!(
        capabilities.has_architecture,
        "规范 Design Spec 必须视为具备架构上下文"
    );
    assert!(
        capabilities.has_module_breakdown,
        "规范 Design Spec 必须视为具备模块划分上下文"
    );
    assert!(
        capabilities.has_tech_stack,
        "规范 Design Spec 必须视为具备技术选型上下文"
    );
    assert!(
        capabilities.has_test_strategy,
        "规范 Design Spec 必须视为具备测试策略上下文"
    );
    assert!(
        capabilities.has_key_paths,
        "规范 Design Spec 必须视为具备关键路径上下文"
    );
    assert!(
        design_context_gaps(&capabilities).is_empty(),
        "规范 Design Spec 不得注入任何 missing_* gap：{:?}",
        design_context_gaps(&capabilities)
    );
}

#[test]
fn canonical_design_spec_with_english_alias_headings_provides_all_capabilities() {
    // 与 parser 契约对齐：必须覆盖 artifact_constraints.rs 中 WorkspaceType::Design
    // 全部强制标题的英文别名，缺一即回落启发式。
    let markdown = r#"# Design Spec

## Design Scope
Covered scope.

## design decisions
Decisions made.

## SHARED COMPONENTS
None.

## API Contract
formatCompactDuration(totalSeconds: number): string

## Data Model
None.

## Risks
None.

## Traceability
[REQ-001] -> design_spec_0001#DEC-001
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(capabilities.has_architecture);
    assert!(capabilities.has_module_breakdown);
    assert!(capabilities.has_tech_stack);
    assert!(capabilities.has_test_strategy);
    assert!(capabilities.has_key_paths);
}

/// Bug C 回归（编号标题）：parser 明确接受编号规范标题（见
/// workspace_engine/tests/part_12.rs 的 design_artifact_gate_accepts_numbered_canonical_headings），
/// 能力检测必须与 parser 口径一致，容忍 "## 1. 设计范围" 这类编号前缀。
#[test]
fn numbered_canonical_design_spec_provides_all_context_capabilities() {
    let markdown = r#"# 设计 Spec：provider 依赖自检（design_spec_0001）

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 2. 设计决策

- [DEC-001] 新建 ProviderCatalog。

## 3. 公共组件

- [CMP-001] ProviderCatalog。

## 4. API 契约

- [API-001] ProviderCatalog::probe。

## 5. 数据模型

- ProviderCapability。

## 6. 风险

无。

## 7. 追踪关系

- source ids: Story Spec story_spec_0001, Issue issue_0001
- [DEC-001] -> [REQ-001]
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(
        capabilities.has_architecture,
        "编号规范标题必须视为具备架构上下文"
    );
    assert!(capabilities.has_module_breakdown);
    assert!(capabilities.has_tech_stack);
    assert!(capabilities.has_test_strategy);
    assert!(capabilities.has_key_paths);
    assert!(
        design_context_gaps(&capabilities).is_empty(),
        "编号规范 Design Spec 不得注入任何 missing_* gap：{:?}",
        design_context_gaps(&capabilities)
    );
}

/// nit：混合场景——同一文档内部分规范标题带编号、部分不带编号，只要 parser
/// 强制标题齐备就应视为规范结构。
#[test]
fn mixed_numbered_and_plain_canonical_headings_provide_all_capabilities() {
    let markdown = r#"# 设计 Spec：混合编号示例（design_spec_0001）

## 1. 设计范围

范围描述。

## 设计决策

- [DEC-001] 决策描述。

## 2. 公共组件

- [CMP-001] 组件描述。

## API 契约

- [API-001] 接口描述。

## 3. 数据模型

- 无持久化数据模型。

## 风险

无。

## 4. 追踪关系

- [DEC-001] -> [REQ-001]
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(capabilities.has_architecture);
    assert!(capabilities.has_module_breakdown);
    assert!(capabilities.has_tech_stack);
    assert!(capabilities.has_test_strategy);
    assert!(capabilities.has_key_paths);
    assert!(design_context_gaps(&capabilities).is_empty());
}

#[test]
fn numbered_non_canonical_design_spec_falls_back_to_heuristic() {
    // 编号标题但并非规范结构（缺少多个 parser 强制标题）：必须回落启发式，
    // 且该文档也无启发式标题，5 项能力全部缺失。
    let markdown = r#"# 旧版设计文档

## 1. 背景

背景描述。

## 2. 方案

方案描述。
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(!capabilities.has_architecture);
    assert!(!capabilities.has_module_breakdown);
    assert!(!capabilities.has_tech_stack);
    assert!(!capabilities.has_test_strategy);
    assert!(!capabilities.has_key_paths);
    assert_eq!(design_context_gaps(&capabilities).len(), 5);
}

#[test]
fn partial_canonical_headings_do_not_satisfy_parser_contract() {
    // minor-1 收紧：检测口径与 parser 强制标题完全一致。旧检测集只要求
    // 设计范围/设计决策/公共组件/追踪关系 4 个标题，会把缺少 API 契约/数据模型/风险
    // 的不完整文档误判为规范结构；收紧后这类文档必须回落启发式。
    let markdown = r#"# 设计 Spec

## 设计范围
范围描述。

## 设计决策
决策描述。

## 公共组件
组件描述。

## 追踪关系
[REQ-001] -> design_spec_0001#DEC-001
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(
        !capabilities.has_architecture,
        "缺少 API 契约/数据模型/风险的文档不得视为规范结构"
    );
    assert!(!capabilities.has_module_breakdown);
    assert!(!capabilities.has_tech_stack);
    assert!(!capabilities.has_test_strategy);
    assert!(!capabilities.has_key_paths);
    assert_eq!(design_context_gaps(&capabilities).len(), 5);
}

#[test]
fn merged_design_context_treats_collect_prefix_plus_canonical_spec_as_capable() {
    // collect_design_context 会在 markdown 前注入 "Design Spec: {title} ({id})" 前缀行。
    let prefixed = format!(
        "Design Spec: 验证专用 Design Spec (design_spec_0001)\n{CANONICAL_DESIGN_SPEC_MARKDOWN}"
    );

    let merged = crate::product::work_item_split_engine::merge_design_context_capabilities(
        std::slice::from_ref(&prefixed),
    );
    assert!(merged.has_architecture);
    assert!(merged.has_module_breakdown);
    assert!(merged.has_tech_stack);
    assert!(merged.has_test_strategy);
    assert!(merged.has_key_paths);
    assert!(design_context_gaps(&merged).is_empty());
}

#[test]
fn partial_canonical_headings_fall_back_to_existing_heuristic() {
    // 缺少 追踪关系 等 parser 强制标题：不构成规范结构，退回启发式；该文档也无启发式标题。
    let markdown = r#"# 设计 Spec

## 设计范围
范围描述。

## 设计决策
决策描述。

## 公共组件
组件描述。
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(!capabilities.has_architecture);
    assert!(!capabilities.has_module_breakdown);
    assert!(!capabilities.has_tech_stack);
    assert!(!capabilities.has_test_strategy);
    assert!(!capabilities.has_key_paths);
    assert_eq!(design_context_gaps(&capabilities).len(), 5);
}

#[test]
fn non_canonical_design_spec_keeps_existing_heuristic() {
    let markdown = r#"# 旧版设计文档

## 架构概览
整体架构描述。

## 技术选型
Rust + TypeScript。

## 无关章节
不会命中任何能力。
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    assert!(capabilities.has_architecture);
    assert!(capabilities.has_tech_stack);
    assert!(!capabilities.has_module_breakdown);
    assert!(!capabilities.has_test_strategy);
    assert!(!capabilities.has_key_paths);
}

#[test]
fn design_context_extracts_only_explicit_requirement_ids_in_stable_order() {
    let context = vec![
        "Design Spec: x\n[REQ-003] and [NFR-001], then REQ-003 again".to_string(),
        "## Traceability\n[REQ-002] -> DEC-001; unrelated requirement text".to_string(),
    ];

    assert_eq!(
        crate::product::work_item_split_engine::context::extract_design_requirement_ids(&context),
        vec!["NFR-001", "REQ-002", "REQ-003"]
    );
}

#[test]
fn design_context_does_not_invent_requirement_ids_when_none_are_present() {
    assert!(crate::product::work_item_split_engine::context::extract_design_requirement_ids(&[
        "Design Spec: no explicit requirement references".to_string(),
    ])
    .is_empty());
}

#[test]
fn empty_design_spec_has_no_capabilities() {
    let capabilities = extract_design_context_capabilities("");
    assert!(!capabilities.has_architecture);
    assert!(!capabilities.has_module_breakdown);
    assert!(!capabilities.has_tech_stack);
    assert!(!capabilities.has_test_strategy);
    assert!(!capabilities.has_key_paths);
    assert_eq!(design_context_gaps(&capabilities).len(), 5);

    let merged = crate::product::work_item_split_engine::merge_design_context_capabilities(&[]);
    assert_eq!(design_context_gaps(&merged).len(), 5);
}
