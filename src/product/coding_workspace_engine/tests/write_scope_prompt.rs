//! F1/F2：写入范围规范段在 prompt 面的落地契约。
//!
//! - F1：group（schema v2 投影渲染）主路径的 coder / reviewer prompt 必须
//!   携带与 legacy markdown **逐字同源**的写入范围规范段（判定基准／匹配
//!   语义非 glob／空列表语义／默认拒绝），数据取投影的 write_policy（与机器
//!   门同一份取值）；同一 prompt 内范围信息只能有一份；delta 轮不带该段。
//! - F2：reviewer 协议必须点名材料里实际使用的段标题（锚点可检索），协议与
//!   材料标题同源（同一常量）。

use super::*;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;
use crate::product::coding_work_item_context::{
    WRITE_SCOPE_EXCLUSIVE_LIST_HEADING, WRITE_SCOPE_FORBIDDEN_LIST_HEADING,
    WRITE_SCOPE_SECTION_HEADING, write_scope_section_text,
};
use crate::product::work_item_contract::WorkItemWritePolicy;
use std::fs;

/// 写入范围事实（非空才有判别力——空表只渲染「未声明」占位）。
fn scoped_write_policy() -> WorkItemWritePolicy {
    WorkItemWritePolicy {
        exclusive_scopes: vec!["src/product/**".to_string()],
        forbidden_scopes: vec!["tests/**".to_string()],
    }
}

/// 建 group attempt（带非空写入范围事实）并跑完一次 coder 渲染，返回
/// (`TempDir`, `store`, `coded_attempt`, coder 的实际 prompt)。
///
/// 调用方必须持有返回的 `TempDir` 到测试结束。
async fn run_group_coder_with_scope() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    String,
) {
    let root = tempfile::tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    super::init_test_git_repo(&worktree);
    let head = super::git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .unwrap();
    super::seed_group_attempt_fixture_with_write_policy(
        &store,
        &attempt,
        true,
        false,
        scoped_write_policy(),
    );
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    attempt.head_commit = Some(head);
    attempt.stage = CodingExecutionStage::Coding;
    store.write_coding_attempt_for_test(&attempt).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let coder = super::provider_execution_context::CapturingProjectionProvider::new(
        super::provider_execution_context::current_plan_defect_output(),
    );
    let coded = engine
        .execute_coding(&attempt, &coder, &CodingExecutionContext::default())
        .await
        .unwrap();
    let prompt = coder.input().prompt.clone();
    (root, store, coded, prompt)
}

/// F1：group coder prompt 必须带与 legacy 逐字同源的写入范围规范段，且只有
/// 一份（无重复可读列表）。
#[tokio::test]
async fn group_coder_prompt_carries_verbatim_write_scope_section() {
    let (_root, store, coded, prompt) = run_group_coder_with_scope().await;
    let policy = scoped_write_policy();
    let expected = write_scope_section_text(&policy.exclusive_scopes, &policy.forbidden_scopes);

    assert!(
        prompt.contains(&expected),
        "group coder prompt 必须携带与 legacy 逐字同源的写入范围规范段:\n{prompt}"
    );
    // 规范语义四要素在场（判定基准／匹配语义非 glob／空列表语义／默认拒绝）。
    assert!(
        prompt.contains("- 判定基准：工作树根相对路径"),
        "范围规范段必须写明判定基准:\n{prompt}"
    );
    assert!(
        prompt.contains("- 匹配语义：前缀匹配，不是 shell glob 展开。"),
        "范围规范段必须写明匹配语义非 glob:\n{prompt}"
    );
    assert!(
        prompt.contains("- 空列表语义：exclusive 为空 = 不限制可写范围"),
        "范围规范段必须写明空列表语义:\n{prompt}"
    );
    assert!(
        prompt.contains("- 默认拒绝：以上范围之外的仓库文件一律不得修改"),
        "范围规范段必须写明默认拒绝:\n{prompt}"
    );
    // 列表逐字来自 projection.write_policy。
    assert!(
        prompt.contains(&format!(
            "### {WRITE_SCOPE_EXCLUSIVE_LIST_HEADING}\n\n- src/product/**\n"
        )),
        "允许修改范围列表必须逐字来自 projection.write_policy:\n{prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "### {WRITE_SCOPE_FORBIDDEN_LIST_HEADING}\n\n- tests/**\n"
        )),
        "禁止修改范围列表必须逐字来自 projection.write_policy:\n{prompt}"
    );
    // 同一 prompt 内范围信息只有一份。
    assert_eq!(
        prompt.matches(WRITE_SCOPE_SECTION_HEADING).count(),
        1,
        "同一 prompt 内写入范围规范段只能有一份:\n{prompt}"
    );
    assert_eq!(
        prompt.matches(WRITE_SCOPE_FORBIDDEN_LIST_HEADING).count(),
        1,
        "同一 prompt 内禁止修改范围列表只能有一份:\n{prompt}"
    );

    // 追加发生在 render→bind 之后：被哈希的渲染上下文本身不含该段
    // （执行上下文哈希不受影响），且哈希已绑定。
    let run = store
        .get_active_unit_run(&coded)
        .expect("rendered unit run");
    assert!(
        run.coder_execution_context_hash.is_some(),
        "渲染路径必须绑定执行上下文哈希"
    );
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let rendered = engine
        .render_coder_unit_run_context(&coded, &ProviderName::Codex)
        .expect("re-render coder context")
        .expect("group context");
    assert!(
        !rendered.text.contains(WRITE_SCOPE_SECTION_HEADING),
        "范围规范段不得进入被哈希的渲染 envelope:\n{}",
        rendered.text
    );
}

/// F1：group reviewer prompt 必须带与 legacy 逐字同源的写入范围规范段。
#[tokio::test]
async fn group_reviewer_prompt_carries_verbatim_write_scope_section() {
    let (root, store, coded, _coder_prompt) = run_group_coder_with_scope().await;
    let worktree = root.path().join("worktree");
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let reviewer = super::provider_execution_context::CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "request_changes",
            "summary": "缺少范围合规说明",
            "findings": [{
                "severity": "warning",
                "file_path": "src/lib.rs",
                "line": 1,
                "message": "请说明写入范围合规性",
                "required_action": "补充范围核对说明",
                "source_stage": "code_review"
            }]
        })
        .to_string(),
    );
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    engine
        .execute_code_review_with_commands(&coded, &reviewer, &mut cmd_rx)
        .await
        .unwrap();

    let prompt = reviewer.input().prompt.clone();
    let policy = scoped_write_policy();
    let expected = write_scope_section_text(&policy.exclusive_scopes, &policy.forbidden_scopes);
    assert!(
        prompt.contains(&expected),
        "group reviewer prompt 必须携带与 legacy 逐字同源的写入范围规范段:\n{prompt}"
    );
    assert_eq!(
        prompt.matches(WRITE_SCOPE_SECTION_HEADING).count(),
        1,
        "同一 prompt 内写入范围规范段只能有一份:\n{prompt}"
    );
}

/// F1：delta 轮不带范围规范段（首轮 full 已带；与 legacy delta 同口径）。
#[test]
fn delta_prompt_carries_no_write_scope_section() {
    let (_root, _store, attempt) = super::running_attempt_with_worktree();
    let delta = super::super::prompts::build_coding_delta_prompt(
        &attempt,
        &CodingExecutionContext::default(),
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );
    assert!(
        !delta.contains(WRITE_SCOPE_SECTION_HEADING),
        "delta prompt 不得重复携带写入范围规范段:\n{delta}"
    );
}

/// F2：reviewer 协议必须点名材料里实际使用的段标题（锚点可检索），且协议与
/// 材料标题同源（同一常量）。
#[test]
fn reviewer_protocols_anchor_the_material_write_scope_heading() {
    let material =
        write_scope_section_text(&["src/product/**".to_string()], &["tests/**".to_string()]);
    assert!(
        material.contains(WRITE_SCOPE_FORBIDDEN_LIST_HEADING),
        "材料必须使用统一的禁止修改范围段标题:\n{material}"
    );
    assert!(
        material.contains(WRITE_SCOPE_EXCLUSIVE_LIST_HEADING),
        "材料必须使用统一的允许修改范围段标题:\n{material}"
    );

    for (name, protocol) in [
        (
            "reviewer_process_evidence_boundary",
            super::super::prompts::reviewer_process_evidence_boundary_contract(),
        ),
        (
            "code_review_material_protocol",
            super::super::prompts::code_review_material_protocol(&RoutingReferenceContext::Legacy),
        ),
        (
            "group_final_review_material_protocol",
            super::super::prompts::group_final_review_material_protocol(
                &RoutingReferenceContext::Legacy,
            ),
        ),
    ] {
        assert!(
            protocol.contains(WRITE_SCOPE_FORBIDDEN_LIST_HEADING),
            "{name} 协议必须点名材料实际使用的段标题「{WRITE_SCOPE_FORBIDDEN_LIST_HEADING}」:\n{protocol}"
        );
    }
}
