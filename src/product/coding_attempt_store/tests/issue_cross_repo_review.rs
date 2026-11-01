// issue 级跨仓最终评审持久化 store 测试（REQ-GFR-CR-03 / REQ-GFR-CR-06 /
// REQ-GFR-CR-07）。落盘口径：评审本体 `projects/{pid}/issues/{iid}/
// coding-cross-repo-reviews/review.json`，留痕 `.../acknowledgements/{id}.json`。

use std::path::PathBuf;

use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionStage, CrossRepoRepositorySection, FindingSeverity, IssueCrossRepoReview,
    IssueCrossRepoReviewStatus, IssueCrossRepoRiskAcknowledgement, ReviewFinding, ReviewVerdict,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::models::{PlanDefectClass, PlanDefectRoute};
use crate::product::project_store::{CreateProjectInput, ProjectStore};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

fn setup_store() -> (TempDir, CodingAttemptStore) {
    let tmp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "project".to_string(),
            description: None,
        })
        .unwrap();
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let store = CodingAttemptStore::new(paths);
    (tmp, store)
}

fn cross_repo_review_path(tmp: &TempDir) -> PathBuf {
    tmp.path()
        .join(".aria")
        .join("projects")
        .join(PROJECT_ID)
        .join("issues")
        .join(ISSUE_ID)
        .join("coding-cross-repo-reviews")
        .join("review.json")
}

fn sample_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(42),
        message: "跨仓调用面与上游契约不一致".to_string(),
        required_action: Some("对齐上游契约签名".to_string()),
        source_stage: CodingExecutionStage::IssueCrossRepoReview,
        evidence: vec!["evidence-1".to_string()],
        plan_defect_evidence: Vec::new(),
        related_requirements: vec!["REQ-GFR-CR-07".to_string()],
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: PlanDefectClass::ImplementationDefect,
        reason_code: None,
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: PlanDefectRoute::CoderRework,
        confidence: None,
    }
}

fn sample_review() -> IssueCrossRepoReview {
    IssueCrossRepoReview {
        id: "cross_repo_review_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: "plan_0001".to_string(),
        verdict: ReviewVerdict::RequestChanges,
        summary: "跨仓契约存在一处不一致".to_string(),
        findings: vec![sample_finding()],
        repository_sections: vec![CrossRepoRepositorySection {
            repository_name: "backend".to_string(),
            logical_repository_id: "logical_repository_0001".to_string(),
            attempt_id: "attempt_0001".to_string(),
            branch_name: "feature/backend".to_string(),
            base_commit: "base0001".to_string(),
            head_commit: "head0001".to_string(),
            work_item_ids: vec!["work_item_0001".to_string()],
            diff_truncated: true,
        }],
        status: IssueCrossRepoReviewStatus::Completed,
        failure_reason: None,
        raw_provider_output_ref: Some("raw/output.json".to_string()),
        created_at: "2026-10-23T10:00:00Z".to_string(),
    }
}

fn sample_acknowledgement() -> IssueCrossRepoRiskAcknowledgement {
    IssueCrossRepoRiskAcknowledgement {
        id: "ack_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: "plan_0001".to_string(),
        attempt_id: "attempt_0001".to_string(),
        cross_repo_review_id: "cross_repo_review_0001".to_string(),
        verdict: ReviewVerdict::RequestChanges,
        findings_summary: "error=1 warning=0 info=0".to_string(),
        acknowledged_at: "2026-10-23T10:05:00Z".to_string(),
    }
}

#[test]
fn issue_cross_repo_review_roundtrips_through_store() {
    let (tmp, store) = setup_store();

    assert!(
        store
            .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_none()
    );

    let review = sample_review();
    store.save_issue_cross_repo_review(&review).unwrap();

    // 落盘路径口径：projects/{project_id}/issues/{issue_id}/coding-cross-repo-reviews/review.json
    assert!(cross_repo_review_path(&tmp).is_file());

    let loaded = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("跨仓评审记录应可读回");
    assert_eq!(loaded.id, review.id);
    assert_eq!(loaded.project_id, review.project_id);
    assert_eq!(loaded.issue_id, review.issue_id);
    assert_eq!(loaded.plan_id, review.plan_id);
    assert_eq!(loaded.verdict, review.verdict);
    assert_eq!(loaded.summary, review.summary);
    assert_eq!(loaded.findings, review.findings);
    assert_eq!(loaded.repository_sections, review.repository_sections);
    assert_eq!(loaded.status, review.status);
    assert_eq!(loaded.failure_reason, review.failure_reason);
    assert_eq!(
        loaded.raw_provider_output_ref,
        review.raw_provider_output_ref
    );
    assert_eq!(loaded.created_at, review.created_at);

    // REQ-GFR-CR-07：跨仓 findings 的来源阶段恒为 issue_cross_repo_review。
    assert_eq!(
        loaded.findings[0].source_stage,
        CodingExecutionStage::IssueCrossRepoReview
    );
    assert_eq!(
        serde_json::to_value(&loaded).unwrap()["findings"][0]["source_stage"],
        "issue_cross_repo_review"
    );
}

#[test]
fn issue_cross_repo_review_save_overwrites_same_issue_record() {
    let (_tmp, store) = setup_store();

    let mut first = sample_review();
    first.status = IssueCrossRepoReviewStatus::Failed;
    first.failure_reason = Some("provider timeout".to_string());
    store.save_issue_cross_repo_review(&first).unwrap();

    let mut second = sample_review();
    second.id = "cross_repo_review_0002".to_string();
    second.verdict = ReviewVerdict::Approve;
    second.findings.clear();
    store.save_issue_cross_repo_review(&second).unwrap();

    // 同 issue 唯一：覆盖式写入，不新增第二份记录。
    let loaded = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("跨仓评审记录应可读回");
    assert_eq!(loaded.id, "cross_repo_review_0002");
    assert_eq!(loaded.verdict, ReviewVerdict::Approve);
    assert!(loaded.findings.is_empty());
}

#[test]
fn issue_cross_repo_review_status_skipped_is_persisted() {
    let (_tmp, store) = setup_store();

    let mut review = sample_review();
    review.status = IssueCrossRepoReviewStatus::Skipped;
    review.verdict = ReviewVerdict::Approve;
    review.findings.clear();
    store.save_issue_cross_repo_review(&review).unwrap();

    let loaded = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("跨仓评审记录应可读回");
    assert_eq!(loaded.status, IssueCrossRepoReviewStatus::Skipped);
}

#[test]
fn acknowledgement_json_has_no_acknowledged_by_or_channel() {
    let acknowledgement = sample_acknowledgement();
    let value = serde_json::to_value(&acknowledgement).unwrap();
    let object = value.as_object().expect("留痕应序列化为 JSON 对象");

    assert!(
        !object.contains_key("acknowledged_by"),
        "留痕 MUST NOT 记录确认人身份，实际键：{:?}",
        object.keys().collect::<Vec<_>>()
    );
    assert!(
        !object.contains_key("acknowledged_channel"),
        "留痕 MUST NOT 记录确认通道，实际键：{:?}",
        object.keys().collect::<Vec<_>>()
    );

    // 结构键与结论字段齐备。
    assert_eq!(object["cross_repo_review_id"], "cross_repo_review_0001");
    assert_eq!(object["attempt_id"], "attempt_0001");
    assert_eq!(object["verdict"], "request_changes");
    assert_eq!(object["acknowledged_at"], "2026-10-23T10:05:00Z");
    assert_eq!(object["findings_summary"], "error=1 warning=0 info=0");
}

#[test]
fn issue_cross_repo_acknowledgement_append_is_idempotent_per_review_and_attempt() {
    let (tmp, store) = setup_store();

    assert!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );

    assert!(
        store
            .append_issue_cross_repo_acknowledgement(&sample_acknowledgement())
            .unwrap()
    );

    let ack_path = tmp
        .path()
        .join(".aria")
        .join("projects")
        .join(PROJECT_ID)
        .join("issues")
        .join(ISSUE_ID)
        .join("coding-cross-repo-reviews")
        .join("acknowledgements")
        .join("ack_0001.json");
    assert!(ack_path.is_file());

    // 同一 (cross_repo_review_id, attempt_id) 再写 → 幂等，不追加第二条。
    assert!(
        !store
            .append_issue_cross_repo_acknowledgement(&sample_acknowledgement())
            .unwrap()
    );

    let mut other_attempt = sample_acknowledgement();
    other_attempt.id = "ack_0002".to_string();
    other_attempt.attempt_id = "attempt_0002".to_string();
    assert!(
        store
            .append_issue_cross_repo_acknowledgement(&other_attempt)
            .unwrap()
    );

    let listed = store
        .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "ack_0001");
    assert_eq!(listed[1].id, "ack_0002");
}
