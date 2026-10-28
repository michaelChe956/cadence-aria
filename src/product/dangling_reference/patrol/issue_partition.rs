//! 分区一：issue 级巡检（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;

// ---------------------------------------------------------------------------
// 分区一：issue 级（intent / enrollment / 表外记录）
// ---------------------------------------------------------------------------

/// 遍历单个 issue 分区（`<root>/projects/<p>/issues/<i>/**` 的顶层记录文件）。
///
/// 只认三个既有落盘文件名的记录类型；同一层的其它 `*.json` 一律未归类
/// （只报不删）。子目录（stories / designs / plans / workspace-sessions /
/// codebase-attempts / entity-deletions 等）属**被引用侧**或非引用记录，
/// 不参与「引用字段」核对；墓碑因此绝不进悬空明细（REQ-DRG-09）。
pub(super) fn scan_issue_partition(
    ctx: &PatrolContext<'_>,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let issue_root = ctx.paths.issue_root(project_id, issue_id);
    let entries = match read_dir_sorted(&issue_root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &issue_root),
                FIELD_PARTITION_LEVEL,
                format!("issue 分区不可读：{error}"),
            );
            return;
        }
    };
    for (path, is_dir) in entries {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        match file_name {
            PLAN_INTENT_FILE => scan_plan_intent(ctx, &path, project_id, issue_id, builder),
            GENERATION_INTENT_FILE => {
                scan_generation_intent(ctx, &path, project_id, issue_id, builder)
            }
            ENROLLMENT_FILE => scan_enrollment(ctx, &path, project_id, issue_id, builder),
            _ => builder.push_unclassified(record_path_of(ctx.paths, &path)),
        }
    }
}

/// 自动化 intent 记录上供判据表核对的引用事实。
struct IntentFacts {
    enrollment_id: String,
    plan_id: Option<String>,
    session_id: Option<String>,
    story_ids: Vec<String>,
    design_ids: Vec<String>,
    project_id: String,
    issue_id: String,
}

/// 核对一条 intent 记录（判据表登记的全部引用字段）。
fn check_intent_record(
    ctx: &PatrolContext<'_>,
    record_path: &str,
    record_type: DanglingRecordType,
    facts: &IntentFacts,
    builder: &mut ReportBuilder,
) {
    builder.mark_covered(record_type);
    let rule = reference_rule(record_type).expect("判据表必须登记 intent 型记录");
    // 记录类型不同但字段判据同源：按该记录类型的判据表行展开字段路径。
    let mut references = Vec::new();
    for field in rule.fields {
        match field.target_kind {
            DanglingTargetKind::StorySpec => {
                for id in &facts.story_ids {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id.clone(),
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::DesignSpec => {
                for id in &facts.design_ids {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id.clone(),
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::Plan => {
                for id in non_empty(&facts.plan_id) {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id,
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::Session => {
                for id in non_empty(&facts.session_id) {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id,
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::Enrollment if !facts.enrollment_id.is_empty() => {
                references.push(PatrolReference::new(
                    field.path,
                    field.target_kind,
                    facts.enrollment_id.clone(),
                    &facts.project_id,
                    &facts.issue_id,
                ));
            }
            _ => {}
        }
    }
    for reference in &references {
        check_reference(ctx, record_path, record_type, reference, None, builder);
    }
}

/// 计划意图记录（`automation-plan-intent.json`）。
fn scan_plan_intent(
    ctx: &PatrolContext<'_>,
    path: &Path,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let read = match read_prepared_plan_intent(path) {
        Ok(read) => read,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    let facts = match read {
        PreparedPlanIntentRead::Current(intent) => IntentFacts {
            enrollment_id: intent.enrollment_id.clone(),
            plan_id: Some(intent.plan_id.clone()),
            session_id: Some(intent.session_id.clone()),
            story_ids: intent
                .source
                .stories
                .iter()
                .map(|story| story.id.clone())
                .collect(),
            design_ids: intent
                .source
                .designs
                .iter()
                .map(|design| design.id.clone())
                .collect(),
            // 记录自带作用域优先；缺失时回落到落盘位置所在的 issue。
            project_id: if intent.project_id.is_empty() {
                project_id.to_string()
            } else {
                intent.project_id.clone()
            },
            issue_id: if intent.issue_id.is_empty() {
                issue_id.to_string()
            } else {
                intent.issue_id.clone()
            },
        },
        // 旧代无绑定意图：只有身份字段，无 plan/session/source 引用。
        PreparedPlanIntentRead::LegacyUnbound { enrollment_id, .. } => IntentFacts {
            enrollment_id,
            plan_id: None,
            session_id: None,
            story_ids: Vec::new(),
            design_ids: Vec::new(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
    };
    check_intent_record(
        ctx,
        &record_path,
        DanglingRecordType::AutomationPlanIntent,
        &facts,
        builder,
    );
}

/// 生成检查点记录（`automation-generation-intent.json`）。
fn scan_generation_intent(
    ctx: &PatrolContext<'_>,
    path: &Path,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let read = match read_plan_generation_intent(path) {
        Ok(read) => read,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    let facts = match read {
        PlanGenerationIntentRead::Current(intent) => IntentFacts {
            enrollment_id: intent.enrollment_id.clone(),
            plan_id: Some(intent.plan_id.clone()),
            session_id: Some(intent.session_id.clone()),
            story_ids: intent
                .source
                .stories
                .iter()
                .map(|story| story.id.clone())
                .collect(),
            design_ids: intent
                .source
                .designs
                .iter()
                .map(|design| design.id.clone())
                .collect(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
        // 旧代无绑定检查点：只有身份字段（enrollment / plan）。
        PlanGenerationIntentRead::LegacyUnbound {
            enrollment_id,
            plan_id,
        } => IntentFacts {
            enrollment_id,
            plan_id: Some(plan_id),
            session_id: None,
            story_ids: Vec::new(),
            design_ids: Vec::new(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
    };
    check_intent_record(
        ctx,
        &record_path,
        DanglingRecordType::AutomationGenerationIntent,
        &facts,
        builder,
    );
}

/// 自动化 enrollment 记录上的引用事实（对应判据表的十行）。
struct EnrollmentFacts {
    project_id: String,
    issue_id: String,
    source_stories: Vec<String>,
    source_designs: Vec<String>,
    plan_id: Option<String>,
    session_id: Option<String>,
    current: Option<EnrollmentBindingRefs>,
    previous: Vec<EnrollmentBindingRefs>,
}

/// 一代绑定身份里的引用（`binding_history.*`）。
struct EnrollmentBindingRefs {
    plan_id: String,
    session_id: String,
    stories: Vec<String>,
    designs: Vec<String>,
}

/// 按判据表登记顺序展开一条 enrollment 记录的全部待核对引用（REQ-DRG-09）。
fn enrollment_references(facts: &EnrollmentFacts) -> Vec<PatrolReference<'static>> {
    let rule = reference_rule(DanglingRecordType::AutomationEnrollment)
        .expect("判据表必须登记 automation_enrollment");
    let mut references = Vec::new();
    for field in rule.fields {
        // 字段路径 → 该字段下的目标 id 集合（判据表新增路径时须同步接线，
        // 由测试 `dangling_patrol_registers_every_enrollment_reference_field` 把关）。
        let ids: Vec<String> = match field.path {
            "source.stories[].id" => facts.source_stories.clone(),
            "source.designs[].id" => facts.source_designs.clone(),
            "plan_id" => non_empty(&facts.plan_id),
            "session_id" => non_empty(&facts.session_id),
            "binding_history.current.plan_id" => facts
                .current
                .as_ref()
                .filter(|current| !current.plan_id.is_empty())
                .map(|current| vec![current.plan_id.clone()])
                .unwrap_or_default(),
            "binding_history.current.session_id" => facts
                .current
                .as_ref()
                .filter(|current| !current.session_id.is_empty())
                .map(|current| vec![current.session_id.clone()])
                .unwrap_or_default(),
            "binding_history.current.source.stories[].id" => facts
                .current
                .as_ref()
                .map(|current| current.stories.clone())
                .unwrap_or_default(),
            "binding_history.current.source.designs[].id" => facts
                .current
                .as_ref()
                .map(|current| current.designs.clone())
                .unwrap_or_default(),
            "binding_history.previous[].source.stories[].id" => facts
                .previous
                .iter()
                .flat_map(|previous| previous.stories.clone())
                .collect(),
            "binding_history.previous[].source.designs[].id" => facts
                .previous
                .iter()
                .flat_map(|previous| previous.designs.clone())
                .collect(),
            _ => Vec::new(),
        };
        for id in ids {
            references.push(PatrolReference::new(
                field.path,
                field.target_kind,
                id,
                &facts.project_id,
                &facts.issue_id,
            ));
        }
    }
    references
}

/// 自动化 enrollment 记录（`automation-enrollment.json`）：只报不删档位的漏检防线。
fn scan_enrollment(
    ctx: &PatrolContext<'_>,
    path: &Path,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let record: IssueAutomationEnrollment = match read_json(path) {
        Ok(record) => record,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    builder.mark_covered(DanglingRecordType::AutomationEnrollment);
    let binding =
        |identity: &crate::product::models::EnrollmentBindingIdentity| EnrollmentBindingRefs {
            plan_id: identity.plan_id.clone(),
            session_id: identity.session_id.clone(),
            stories: identity
                .source
                .stories
                .iter()
                .map(|story| story.id.clone())
                .collect(),
            designs: identity
                .source
                .designs
                .iter()
                .map(|design| design.id.clone())
                .collect(),
        };
    let facts = EnrollmentFacts {
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        source_stories: record
            .source
            .stories
            .iter()
            .map(|story| story.id.clone())
            .collect(),
        source_designs: record
            .source
            .designs
            .iter()
            .map(|design| design.id.clone())
            .collect(),
        plan_id: record.plan_id.clone(),
        session_id: record.session_id.clone(),
        current: record
            .binding_history
            .as_ref()
            .map(|history| binding(&history.current)),
        previous: record
            .binding_history
            .as_ref()
            .map(|history| history.previous.iter().map(&binding).collect())
            .unwrap_or_default(),
    };
    for reference in enrollment_references(&facts) {
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::AutomationEnrollment,
            &reference,
            None,
            builder,
        );
    }
}
