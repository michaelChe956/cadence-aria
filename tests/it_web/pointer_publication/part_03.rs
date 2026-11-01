// 场景 C（冲突人工）：预置成员仓已有不一致标记块 → 条目 Conflict → retry-repo 409
// `pointer_conflict_unresolved` → 修复文件（移除不一致块）→ retry-repo → Pushed/ReviewCreated。

#[tokio::test]
async fn pointer_publication_scenario_c_conflict_requires_manual_fix_then_retry_pushes() {
    let fixture = setup_pointer_fixture(&[("api", true)]);
    let member = &fixture.members[0];
    let member_repo_id = member.logical_id.0.to_string();

    // 预置不一致标记块：repo_id 与成员不符 → classify_merge → Conflict。
    let conflicting_block = render_pointer_block(&PointerBlockFields {
        logical_codebase_id: fixture.logical_codebase_id.clone(),
        repo_id: "00000000-0000-0000-0000-00000000dead".to_string(),
        canonical_policy_locator: fixture.aggregate_root.to_string_lossy().into_owned(),
        pointer_version: 1,
    });
    std::fs::write(member.repo_path.join(".aria-pointer.md"), conflicting_block).unwrap();

    let (status, publication) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK, "publish must succeed: {publication}");
    assert_eq!(publication["status"], "completed_partial");
    let publication_id = publication["id"].as_str().unwrap().to_string();
    let conflict_entry = entry(&publication, &member_repo_id);
    assert_eq!(conflict_entry["state"], "conflict", "{publication}");
    assert!(conflict_entry["conflict_detail"].is_string());
    assert!(conflict_entry["branch_name"].is_null());

    // retry-repo：冲突未解决 → 409 pointer_conflict_unresolved。
    let (status, retry_body) = retry_repo(&fixture.app, &publication_id, &member_repo_id).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "unresolved conflict must be 409: {retry_body}"
    );
    assert_eq!(retry_body["code"], "pointer_conflict_unresolved");

    // 修复：移除不一致标记块 → classify_merge → Append → 全量流水 → ReviewCreated。
    std::fs::remove_file(member.repo_path.join(".aria-pointer.md")).unwrap();
    let (status, retried) = retry_repo(&fixture.app, &publication_id, &member_repo_id).await;
    assert_eq!(status, StatusCode::OK, "retry after fix must succeed: {retried}");
    assert_eq!(retried["status"], "completed_all");
    let fixed_entry = entry(&retried, &member_repo_id);
    assert_eq!(fixed_entry["state"], "review_created", "{retried}");
    let branch_name = fixed_entry["branch_name"].as_str().unwrap();
    assert_eq!(
        branch_name,
        format!("aria-pointer/{member_repo_id}/{publication_id}")
    );
    let bare = member.bare_remote.as_ref().unwrap();
    assert!(
        remote_has_branch(bare, branch_name),
        "fixed member must have remote pointer branch"
    );
    assert_pointer_review_requests(&fixture, &publication_id, false);
}

// 场景 D（partial）：注入 push 失败（成员无 origin 远端）→ CompletedPartial + 失败条目
// Failed；修复远端 → retry 后全推（ReviewCreated + completed_all）。

#[tokio::test]
async fn pointer_publication_scenario_d_partial_push_failure_then_retry_all_pushed() {
    // "ok" 成员有 bare origin；"broken" 成员无 origin → git push 失败（条目 Failed）。
    let fixture = setup_pointer_fixture(&[("ok", true), ("broken", false)]);
    let ok_member = fixture.members.iter().find(|m| m.repo_path.ends_with("ok")).unwrap();
    let broken_member = fixture
        .members
        .iter()
        .find(|m| m.repo_path.ends_with("broken"))
        .unwrap();
    let ok_id = ok_member.logical_id.0.to_string();
    let broken_id = broken_member.logical_id.0.to_string();

    let (status, publication) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK, "publish must succeed: {publication}");
    assert_eq!(publication["status"], "completed_partial", "{publication}");
    let publication_id = publication["id"].as_str().unwrap().to_string();

    let ok_entry = entry(&publication, &ok_id);
    assert_eq!(ok_entry["state"], "review_created", "{publication}");
    let broken_entry = entry(&publication, &broken_id);
    assert_eq!(broken_entry["state"], "failed", "{publication}");
    assert!(
        broken_entry["push_error"].is_string(),
        "failed entry must carry push_error: {publication}"
    );
    let local_pointer_branches = git_out(
        &broken_member.repo_path,
        &["branch", "--list", "aria-pointer/*"],
    );
    assert!(
        local_pointer_branches.is_empty(),
        "failed member must clean up local aria-pointer branches before retry: {local_pointer_branches}"
    );

    // 修复 broken 成员远端（补 bare origin）后重试 → 只补推该成员 → completed_all。
    let remote_path = fixture.root.path().join("broken-origin.git");
    std::fs::create_dir_all(&remote_path).unwrap();
    git(&remote_path, &["init", "--bare"]);
    git(
        &broken_member.repo_path,
        &["remote", "add", "origin", remote_path.to_str().unwrap()],
    );
    git(&broken_member.repo_path, &["push", "-u", "origin", "main"]);

    let (status, retried) = retry_repo(&fixture.app, &publication_id, &broken_id).await;
    assert_eq!(status, StatusCode::OK, "retry after fix must succeed: {retried}");
    assert_eq!(retried["status"], "completed_all", "{retried}");
    let fixed_entry = entry(&retried, &broken_id);
    assert_eq!(fixed_entry["state"], "review_created", "{retried}");
    let branch_name = fixed_entry["branch_name"].as_str().unwrap();
    assert_eq!(
        branch_name,
        format!("aria-pointer/{broken_id}/{publication_id}")
    );
    assert!(
        remote_has_branch(&remote_path, branch_name),
        "fixed member must have remote pointer branch"
    );

    // ok 成员分支保持、两仓 review request 均落盘。
    let ok_branch = format!("aria-pointer/{ok_id}/{publication_id}");
    assert!(
        remote_has_branch(ok_member.bare_remote.as_ref().unwrap(), &ok_branch),
        "ok member branch must remain after retry of broken member"
    );
    assert_pointer_review_requests(&fixture, &publication_id, false);
}
