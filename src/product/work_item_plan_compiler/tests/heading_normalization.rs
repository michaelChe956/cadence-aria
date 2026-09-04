//! 确定性结构标题归一化契约(TDD)。
//!
//! 输入为 2026-09-03 两次现场事故的 pi 交付原件(脱敏固化):
//! - `field-pi-zh-headings-rep1.md`:pi-full rep1,标题带英文括号注记
//!   (`### 身份 (Identity)`),原文编译失败 52 个诊断;
//! - `field-pi-zh-headings-matrix-rep2.md`:矩阵 r1 rep2,裸中文标题
//!   (`### 身份信息`),原文编译失败 253+ 个诊断。
//!
//! 契约:固定映射表把已知中文结构标题逐字映射回规范英文后必须编译通过;
//! 表外未知标题不猜不改,compiler 的 fail-closed 语义保持;正文内容零触碰。

use crate::product::work_item_plan_compiler::{
    NormalizedPlanSource, WorkItemPlanSourceContext, compile_work_item_plan,
    normalize_structural_headings,
};

const FIELD_PI_FULL_REP1: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/field-pi-zh-headings-rep1.md"
));
const FIELD_MATRIX_REP2: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/field-pi-zh-headings-matrix-rep2.md"
));

fn compile(source: &str) -> Result<usize, String> {
    compile_work_item_plan(
        source,
        &WorkItemPlanSourceContext {
            target_repository_id: "repository_0001".to_string(),
        },
    )
    .map(|ir| ir.items.len())
    .map_err(|diagnostics| {
        diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{}:{}:{}",
                    diagnostic.code, diagnostic.line, diagnostic.message
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// 正文零触碰断言:逐行对比输入与输出,只有「被改写为规范英文标题」的行允许不同,
/// 且这些行重写后必须以 `#` 开头;其余行必须逐字节一致。
fn assert_only_structural_heading_lines_changed(input: &str, normalized: &NormalizedPlanSource) {
    let input_lines: Vec<&str> = input.split('\n').collect();
    let output_lines: Vec<&str> = normalized.source.split('\n').collect();
    assert_eq!(
        input_lines.len(),
        output_lines.len(),
        "归一化不得增删任何行"
    );
    let mut changed = 0usize;
    for (index, (before, after)) in input_lines.iter().zip(&output_lines).enumerate() {
        if before == after {
            continue;
        }
        changed += 1;
        assert!(
            after.starts_with('#'),
            "第 {} 行被改写但不是标题行: {after:?}",
            index + 1
        );
        assert!(
            before.starts_with('#'),
            "第 {} 行原本不是标题行却被改写: {before:?}",
            index + 1
        );
    }
    assert_eq!(
        changed, normalized.normalized_heading_lines,
        "报告的归一化行数必须与实际改写行数一致"
    );
}

#[test]
fn field_pi_full_rep1_normalizes_and_compiles() {
    let normalized = normalize_structural_headings(FIELD_PI_FULL_REP1);

    // rep1 原件:1 个文档标题 + 1 个 Work Item 标题 + 13 个 section 标题 = 15 行。
    assert_eq!(normalized.normalized_heading_lines, 15);
    assert!(normalized.source.starts_with("# Work Item Plan\n"));
    assert!(
        normalized
            .source
            .contains("## Work Item WI-001: Hello API\n")
    );
    for section in [
        "### Identity\n",
        "### Goal\n",
        "### Non Goals\n",
        "### Dependencies\n",
        "### Inputs\n",
        "### Outputs\n",
        "### Tasks\n",
        "### Write Policy\n",
        "### Acceptance Criteria\n",
        "### Verification\n",
        "### Handoff Schema\n",
        "### Blockers\n",
        "### Traceability\n",
    ] {
        assert!(
            normalized.source.contains(section),
            "缺少规范英文标题: {section}"
        );
    }
    assert_only_structural_heading_lines_changed(FIELD_PI_FULL_REP1, &normalized);
    assert_eq!(
        compile(&normalized.source).expect("归一化后必须编译通过"),
        1
    );
}

#[test]
fn field_matrix_rep2_normalizes_and_compiles() {
    let normalized = normalize_structural_headings(FIELD_MATRIX_REP2);

    // 矩阵 r1 rep2 原件:1 个文档标题 + 3 个 Work Item 标题 + 3×13 个 section 标题 = 43 行。
    assert_eq!(normalized.normalized_heading_lines, 43);
    assert_only_structural_heading_lines_changed(FIELD_MATRIX_REP2, &normalized);
    for item_heading in [
        "## Work Item WI-001: 后端关卡数据 API 与静态托管服务",
        "## Work Item WI-002: 前端关卡选择页与 DOM 替身测试",
        "## Work Item WI-003: 前后端同源联调集成验证",
    ] {
        assert!(
            normalized.source.contains(item_heading),
            "Work Item 中文标题文本必须保留: {item_heading}"
        );
    }
    assert_eq!(
        compile(&normalized.source).expect("归一化后必须编译通过"),
        3
    );
}

#[test]
fn chinese_body_content_is_preserved_verbatim() {
    let normalized = normalize_structural_headings(FIELD_PI_FULL_REP1);

    // 正文里的中文语义(EARS 条件、能力描述、non_goals)必须逐字保留。
    for body_line in [
        "- summary: WHEN 联调冒烟需要后端问候接口 THE SYSTEM SHALL 提供仅依赖 Node 内置模块的 GET /api/hello 最小 HTTP 服务并返回 {\"message\":\"hello\"}。",
        "- non_goals: 不引入任何第三方依赖，不新增数据库、认证、外部服务或前端页面",
        "- forbidden_scopes: 除 server.js 与 test/hello.test.js 外的仓库全部既有文件与目录",
        "- source_id: issue_workitem_0001#api",
    ] {
        assert!(
            normalized.source.contains(body_line),
            "正文被改动: {body_line}"
        );
    }
}

#[test]
fn unknown_headings_are_not_normalized_and_stay_fail_closed() {
    let source = "# 未知计划\n\n## 计划项 WI-001: 标题\n\n### 身份\n- schema_version: 1\n\n### 溯源清单\n- schema_version: 1\n";

    let normalized = normalize_structural_headings(source);

    // 表内词条(`### 身份`)被归一化;表外标题(未知一级/二级/三级)逐字保留。
    assert_eq!(normalized.normalized_heading_lines, 1);
    assert!(normalized.source.contains("# 未知计划\n"));
    assert!(normalized.source.contains("## 计划项 WI-001: 标题\n"));
    assert!(normalized.source.contains("### 溯源清单\n"));
    assert!(normalized.source.contains("### Identity\n"));

    let error = compile(&normalized.source).expect_err("表外标题必须仍被 compiler 拒绝");
    assert!(
        error.contains("missing_section"),
        "缺少文档标题必须失败: {error}"
    );
    assert!(
        error.contains("unknown_structured_key"),
        "未知二级/三级标题必须失败: {error}"
    );
}

#[test]
fn canonical_english_source_is_untouched_with_zero_report() {
    let source = "# Work Item Plan\n\n## Work Item WI-001: English fixture\n\n### Identity\n- schema_version: 1\n- logical_work_item_id: WI-001\n- title: English fixture\n- kind: backend\n";

    let normalized = normalize_structural_headings(source);

    assert_eq!(normalized.normalized_heading_lines, 0);
    assert_eq!(normalized.source, source);
}

#[test]
fn heading_lines_with_trailing_whitespace_or_cr_still_match_table() {
    // parser 对 section 标题做 trim;归一化必须覆盖带尾随空白/CR 的抖动。
    let normalized = normalize_structural_headings("### 身份 \r\n");

    assert_eq!(normalized.normalized_heading_lines, 1);
    assert_eq!(normalized.source, "### Identity\n");
}

#[test]
fn non_heading_lines_starting_with_hash_are_untouched() {
    // `#`/`##`/`###` 之外的井号行(如四级标题)不属于结构标题,不得改写。
    let source = "#### 身份\n#工作项计划\n##工作项 WI-001: x\n";

    let normalized = normalize_structural_headings(source);

    assert_eq!(normalized.normalized_heading_lines, 0);
    assert_eq!(normalized.source, source);
}
