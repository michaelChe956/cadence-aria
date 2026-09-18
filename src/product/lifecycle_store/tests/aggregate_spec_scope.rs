// 聚合视野（Task 7）StorySpec/DesignSpec scope 校验与 confirm gate 测试，
// 按大文件守卫（>1200 行）拆分至本文件，经 include! 内联进父模块，
// 复用父模块 setup/create_session/PROJECT_ID 等夹具。

// === 聚合视野：StorySpec 校验（Task 7）===
// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前经 validate_relative_id
// 约束（本测试使用 project_0001 / issue_0001 等稳定 id）。
const fn stable_member_uuid(seed: u16) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[14] = (seed >> 8) as u8;
    bytes[15] = seed as u8;
    // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
    bytes[6] = 0x70;
    bytes[8] = 0x80;
    Uuid::from_bytes(bytes)
}

const CODEBASE_REF: Uuid = stable_member_uuid(0x0100);
const API_MEMBER: LogicalRepositoryId = LogicalRepositoryId(stable_member_uuid(0x0001));
const WEB_MEMBER: LogicalRepositoryId = LogicalRepositoryId(stable_member_uuid(0x0002));
const UNKNOWN_MEMBER: LogicalRepositoryId = LogicalRepositoryId(stable_member_uuid(0x00ff));

/// 构造聚合 StorySpec scope：effective 成员 = {api, web}。`involved_repository_ids`
/// 与 `focus_repository_id` 留空，由各测试用例用 `..` 语法覆盖。
fn two_effective_members_scope() -> AggregateStorySpecScope {
    AggregateStorySpecScope {
        logical_codebase_ref: CODEBASE_REF,
        effective_member_ids: vec![API_MEMBER, WEB_MEMBER],
        involved_repository_ids: Vec::new(),
        focus_repository_id: None,
    }
}

fn story_spec_dto_path(store: &LifecycleStore, story_id: &str) -> std::path::PathBuf {
    store
        .story_specs_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{story_id}.json"))
}

/// AI 未明确涉及任何仓库（involved_repository_ids 为空）→ 方案 X 阶段 1：Draft 态允许空
/// involved（AI 尚未产出），create 时恒为 Draft → 成功持久化为 Draft，不回落 primary 也不
/// blocker；Confirmed 态强制非空由 spec::tests 的 validate_aggregate_story_scope 单测覆盖。
#[test]
fn story_without_involved_repositories_persists_as_draft() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    // Draft 态允许空 involved：不回落 primary，聚合字段按原值持久化。
    assert_eq!(
        story.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
    assert!(story.involved_repository_ids.is_empty());
    assert_eq!(story.focus_repository_id, None);
    assert_eq!(story.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(story.repository_id, REPOSITORY_ID);

    // 磁盘持久化一致（不回落 primary）。
    let persisted: StorySpecRecord = read_json(&story_spec_dto_path(&store, &story.id)).unwrap();
    assert!(persisted.involved_repository_ids.is_empty());
    assert_eq!(
        persisted.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
}

/// AI 输出不在有效集合内的仓库 → blocker，不持久化 StorySpec，不回落 primary。
#[test]
fn story_with_involved_outside_effective_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![API_MEMBER, UNKNOWN_MEMBER],
                focus_repository_id: Some(API_MEMBER),
                ..two_effective_members_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("involved_repository_not_effective")),
        "越界 involved_repository_ids 应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_story_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 StorySpec"
    );
}

/// 校验通过 → 持久化 StorySpec，填充聚合视野字段。
#[test]
fn story_with_involved_within_effective_persists_aggregate_scope() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![WEB_MEMBER, API_MEMBER],
                focus_repository_id: Some(API_MEMBER),
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    assert_eq!(story.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(story.involved_repository_ids, vec![WEB_MEMBER, API_MEMBER]);
    assert_eq!(story.focus_repository_id, Some(API_MEMBER));
    // 迁移期 repository_id 仅作 primary 投影，保持传入值。
    assert_eq!(story.repository_id, REPOSITORY_ID);

    // 磁盘持久化一致。
    let persisted: StorySpecRecord = read_json(&story_spec_dto_path(&store, &story.id)).unwrap();
    assert_eq!(persisted.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(
        persisted.involved_repository_ids,
        vec![WEB_MEMBER, API_MEMBER]
    );
    assert_eq!(persisted.focus_repository_id, Some(API_MEMBER));
}

/// focus_repository_id ∈ involved_repository_ids 但不在 involved 列表中 → blocker。
#[test]
fn story_focus_repository_must_be_within_involved() {
    let (_tmp, store) = setup();
    let error = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![API_MEMBER],
                focus_repository_id: Some(WEB_MEMBER),
                ..two_effective_members_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("focus_repository_not_involved")),
        "focus 必须在 involved 集合内，错误: {error:?}"
    );
    assert_eq!(
        store.list_story_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 StorySpec"
    );
}

/// 传统单仓 issue（aggregate_codebase = None）仍走原 repository_id 单值路径，
/// 聚合视野字段保持空/None。
#[test]
fn story_without_aggregate_scope_keeps_single_repository_path() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "single repo story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    assert_eq!(story.repository_id, REPOSITORY_ID);
    assert_eq!(story.logical_codebase_ref, None);
    assert!(story.involved_repository_ids.is_empty());
    assert_eq!(story.focus_repository_id, None);
}

// === 聚合视野：DesignSpec 校验与改动顺序持久化（Task 8）===
// 复用 Task 7 稳定 UUID 常量；Design 不回落 issue.repo_id，involved_repository_ids 必须
// ⊆ effective_member_ids；change_order 由 AI 显式给出（执行顺序图，非服务调用图）。
fn two_effective_members_design_scope() -> AggregateDesignSpecScope {
    AggregateDesignSpecScope {
        logical_codebase_ref: CODEBASE_REF,
        effective_member_ids: vec![API_MEMBER, WEB_MEMBER],
        involved_repository_ids: Vec::new(),
        change_order: Vec::new(),
    }
}

fn design_spec_dto_path(store: &LifecycleStore, design_id: &str) -> std::path::PathBuf {
    store
        .design_specs_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{design_id}.json"))
}

/// AI 未明确涉及任何仓库（involved_repository_ids 为空）→ 方案 X 阶段 1：Draft 态允许空
/// involved（AI 尚未产出），create 时恒为 Draft → 成功持久化为 Draft，不回落 issue.repo_id
/// 也不 blocker；Confirmed 态强制非空由 spec::tests 的 validate_aggregate_design_scope 单测覆盖。
#[test]
fn design_without_involved_repositories_persists_as_draft() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: Vec::new(),
                change_order: Vec::new(),
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    // Draft 态允许空 involved：不回落 issue.repo_id，聚合字段按原值持久化。
    assert_eq!(
        design.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
    assert!(design.involved_repository_ids.is_empty());
    assert!(design.change_order.is_empty());
    assert_eq!(design.logical_codebase_ref, Some(CODEBASE_REF));

    // 磁盘持久化一致。
    let persisted: DesignSpecRecord = read_json(&design_spec_dto_path(&store, &design.id)).unwrap();
    assert!(persisted.involved_repository_ids.is_empty());
    assert_eq!(
        persisted.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
}

/// AI 输出不在有效集合内的仓库 → blocker，不持久化 DesignSpec，不回落 issue.repo_id。
#[test]
fn design_with_involved_outside_effective_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, UNKNOWN_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("involved_repository_not_effective")),
        "越界 involved_repository_ids 应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_design_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 DesignSpec"
    );
}

/// change_order 的任一 id ∉ involved_repository_ids → blocker（改动顺序必须覆盖全部涉及仓库）。
#[test]
fn design_change_order_outside_involved_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("change_order_repository_not_involved")),
        "change_order 越界应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_design_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 DesignSpec"
    );
}

/// change_order 出现重复 involved 仓库 → blocker（执行顺序图不得重复顶点）。
#[test]
fn design_change_order_with_duplicate_repository_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER, API_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("change_order_duplicate_repository")),
        "change_order 重复顶点应 fail-closed 为 blocker，错误: {error:?}"
    );
}

/// change_order 缺失（空）→ 非 blocker（AI 可不给改动顺序）；仅当 WorkItem 编译时若有才作
/// depends_on 依据（Task 9 消费）。involved 有效即持久化，聚合字段填充，磁盘一致。
/// Design 不回落 issue.repo_id：involved_repository_ids 全部 ∈ effective_member_ids，
/// 且 DesignSpecRecord 无 repository_id 字段（conceptual repository_id_or_none() == None）。
#[test]
fn design_change_order_optional_but_involved_valid_persists_aggregate_scope() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: Vec::new(),
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    // involved_repository_ids ⊆ effective_member_ids，且不回落 issue.repo_id。
    assert_eq!(design.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(design.involved_repository_ids, vec![API_MEMBER, WEB_MEMBER]);
    assert!(design.change_order.is_empty());

    // 磁盘持久化一致。
    let persisted: DesignSpecRecord = read_json(&design_spec_dto_path(&store, &design.id)).unwrap();
    assert_eq!(persisted.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(
        persisted.involved_repository_ids,
        vec![API_MEMBER, WEB_MEMBER]
    );
    assert!(persisted.change_order.is_empty());
}

/// change_order 由 AI 显式给出（例：公共契约 → provider → consumer）→ 持久化为执行顺序图，
/// involved_repository_ids 全部 ∈ effective_member_ids。Design 不回落 issue.repo_id。
#[test]
fn design_change_order_drives_persisted_order_and_does_not_fallback_to_repo_id() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    // change_order 被持久化、顺序保留；involved 全部 ∈ effective_member_ids。
    assert_eq!(design.change_order.len(), 2);
    assert_eq!(design.change_order, vec![API_MEMBER, WEB_MEMBER]);
    assert!(
        design
            .involved_repository_ids
            .iter()
            .all(|id| [API_MEMBER, WEB_MEMBER].contains(id))
    );
    // DesignSpecRecord 无 repository_id 字段，即 conceptual repository_id_or_none() == None。
    let encoded = serde_json::to_value(&design).unwrap();
    assert!(
        encoded.get("repository_id").is_none(),
        "Design 不应回落 issue.repo_id：encoded={encoded}"
    );

    // 磁盘持久化一致。
    let persisted: DesignSpecRecord = read_json(&design_spec_dto_path(&store, &design.id)).unwrap();
    assert_eq!(persisted.change_order, vec![API_MEMBER, WEB_MEMBER]);
}

/// 传统单仓 issue（aggregate_codebase = None）仍走原 DesignSpec 单值路径，聚合视野字段空。
#[test]
fn design_without_aggregate_scope_keeps_single_repository_path() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "single repo design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    assert_eq!(design.logical_codebase_ref, None);
    assert!(design.involved_repository_ids.is_empty());
    assert!(design.change_order.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// 确认 gate（Task 6 下沉，Blocker 2）：validate_confirm_aggregate_spec
// ─────────────────────────────────────────────────────────────────────────────

/// 多仓 Story（logical_codebase_ref Some）involved 为空 → blocker involved_repositories_undetermined。
#[test]
fn validate_confirm_aggregate_spec_rejects_multi_repo_story_without_involved() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: String::new(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    let error = store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &story.id, &WorkspaceType::Story)
        .unwrap_err();
    assert_eq!(
        error.stable_code(),
        "involved_repositories_undetermined",
        "expected involved_repositories_undetermined, got: {error:?}"
    );
    assert!(
        matches!(
            error,
            ConfirmAggregateGateError::Violation {
                violation: ConfirmGateViolation::InvolvedUndetermined,
                ..
            }
        ),
        "expected InvolvedUndetermined violation, got: {error:?}"
    );
}

/// 多仓 Design（involved=2）缺 change_order → blocker change_order_required_for_logical_codebase。
#[test]
fn validate_confirm_aggregate_spec_rejects_multi_repo_design_without_change_order() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "multi repo design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: Vec::new(),
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    let error = store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &design.id, &WorkspaceType::Design)
        .unwrap_err();
    assert_eq!(
        error.stable_code(),
        "change_order_required_for_logical_codebase",
        "expected change_order_required_for_logical_codebase, got: {error:?}"
    );
    assert!(
        matches!(
            error,
            ConfirmAggregateGateError::Violation {
                violation: ConfirmGateViolation::ChangeOrderRequired,
                ..
            }
        ),
        "expected ChangeOrderRequired violation, got: {error:?}"
    );
}

/// 多仓 Design 带 change_order → 通过 gate。
#[test]
fn validate_confirm_aggregate_spec_passes_multi_repo_design_with_change_order() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "multi repo design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &design.id, &WorkspaceType::Design)
        .expect("multi-repo design with change_order must pass the confirm gate");
}

/// 多仓 Story 带 involved → 通过 gate。
#[test]
fn validate_confirm_aggregate_spec_passes_multi_repo_story_with_involved() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: String::new(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                focus_repository_id: None,
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &story.id, &WorkspaceType::Story)
        .expect("multi-repo story with involved must pass the confirm gate");
}

/// 单仓 Story（无 aggregate）→ 不校验，gate 通过（红线）。
#[test]
fn validate_confirm_aggregate_spec_passes_single_repo_story() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "single repo story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &story.id, &WorkspaceType::Story)
        .expect("single-repo story must pass the confirm gate");
}

/// 非 Story/Design workspace（如 WorkItem）→ gate 直接通过（不校验）。
#[test]
fn validate_confirm_aggregate_spec_ignores_non_story_design_workspace() {
    let (_tmp, store) = setup();
    store
        .validate_confirm_aggregate_spec(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            &WorkspaceType::WorkItem,
        )
        .expect("non story/design workspace must skip the aggregate confirm gate");
}
