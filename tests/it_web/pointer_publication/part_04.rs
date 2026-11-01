// 场景 E（revoke）：撤回远端分支 + ReviewRequest.revoked + 重复 revoke 幂等；
// 删除失败（注入 origin 缺失）→ pointer_revoke_failed 可重试；标记失败（注入
// review-requests 目录替换为普通文件）→ 重试只补标记（删除幂等）。

async fn get_publication(app: &axum::Router, publication_id: &str) -> (StatusCode, Value) {
    request_json(
        app.clone(),
        Method::GET,
        &format!("{PUBLICATIONS_URI}/{publication_id}"),
        json!({}),
    )
    .await
}

#[tokio::test]
async fn pointer_publication_scenario_e_revoke_deletes_branches_and_is_idempotent() {
    let fixture = setup_pointer_fixture(&[("api", true)]);
    let member = &fixture.members[0];
    let member_repo_id = member.logical_id.0.to_string();
    let (status, publication) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK);
    let publication_id = publication["id"].as_str().unwrap().to_string();
    let branch_name = format!("aria-pointer/{member_repo_id}/{publication_id}");
    assert!(remote_has_branch(
        member.bare_remote.as_ref().unwrap(),
        &branch_name
    ));

    let (status, revoked) = revoke(&fixture.app, &publication_id).await;
    assert_eq!(status, StatusCode::OK, "revoke must succeed: {revoked}");
    assert_eq!(revoked["status"], "revoked");
    let revoked_entry = entry(&revoked, &member_repo_id);
    assert_eq!(revoked_entry["state"], "revoked", "{revoked}");

    // 远端分支删除 + ReviewRequest.revoked。
    assert!(
        !remote_has_branch(member.bare_remote.as_ref().unwrap(), &branch_name),
        "remote branch must be deleted"
    );
    assert_pointer_review_requests(&fixture, &publication_id, true);

    // 重复 revoke 幂等：仍返回 revoked 态，不报错。
    let (status, again) = revoke(&fixture.app, &publication_id).await;
    assert_eq!(status, StatusCode::OK, "repeat revoke must succeed: {again}");
    assert_eq!(again["status"], "revoked");
}

#[tokio::test]
async fn pointer_publication_scenario_e_revoke_delete_failure_is_retriable() {
    let fixture = setup_pointer_fixture(&[("api", true)]);
    let member = &fixture.members[0];
    let member_repo_id = member.logical_id.0.to_string();
    let (status, publication) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK);
    let publication_id = publication["id"].as_str().unwrap().to_string();
    let branch_name = format!("aria-pointer/{member_repo_id}/{publication_id}");

    // 删除失败注入：移除 origin（origin 缺失 ≠ 远端 ref 缺失）。
    let bare = member.bare_remote.clone().unwrap();
    git(&member.repo_path, &["remote", "remove", "origin"]);

    let (status, error) = revoke(&fixture.app, &publication_id).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "delete failure must be 503: {error}"
    );
    assert_eq!(error["code"], "pointer_revoke_failed");

    // publication 保持可重试态（completed_all），远端分支仍在。
    let (status, after) = get_publication(&fixture.app, &publication_id).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["status"], "completed_all", "{after}");
    assert!(remote_has_branch(&bare, &branch_name), "branch must remain");

    // 恢复 origin 后重试 revoke → 成功删除并标记。
    git(
        &member.repo_path,
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    let (status, revoked) = revoke(&fixture.app, &publication_id).await;
    assert_eq!(status, StatusCode::OK, "retry revoke must succeed: {revoked}");
    assert_eq!(revoked["status"], "revoked");
    assert!(!remote_has_branch(&bare, &branch_name), "branch must be deleted");
    assert_pointer_review_requests(&fixture, &publication_id, true);
}

#[tokio::test]
async fn pointer_publication_scenario_e_revoke_mark_failure_retry_only_marks() {
    let fixture = setup_pointer_fixture(&[("api", true)]);
    let member = &fixture.members[0];
    let member_repo_id = member.logical_id.0.to_string();
    let (status, publication) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK);
    let publication_id = publication["id"].as_str().unwrap().to_string();
    let branch_name = format!("aria-pointer/{member_repo_id}/{publication_id}");

    // 保存 review request 文件内容（恢复用）。
    let rr_root = fixture.review_requests_root(&publication_id);
    let requests = CodingAttemptStore::new(fixture.app_paths())
        .list_pointer_review_requests(PROJECT_ID, &publication_id)
        .unwrap();
    assert_eq!(requests.len(), 1);
    let request_file = rr_root.join(format!("{}.json", requests[0].id));
    let saved = std::fs::read(&request_file).unwrap();

    // 标记失败注入：review-requests 目录替换为普通文件 → 标记阶段 list 失败。
    std::fs::remove_dir_all(&rr_root).unwrap();
    std::fs::write(&rr_root, b"not a directory").unwrap();

    let (status, error) = revoke(&fixture.app, &publication_id).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "mark failure must be 503: {error}"
    );
    assert_eq!(error["code"], "pointer_revoke_failed");

    // publication 未置 Revoked（可重试）；Phase 1 已把远端分支删除（删除幂等依据）。
    let (status, after) = get_publication(&fixture.app, &publication_id).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["status"], "completed_all", "{after}");
    assert!(
        !remote_has_branch(member.bare_remote.as_ref().unwrap(), &branch_name),
        "phase 1 must have deleted the remote branch"
    );

    // 恢复目录 + request 文件后重试：删除幂等（分支已删），只补标记。
    std::fs::remove_file(&rr_root).unwrap();
    std::fs::create_dir_all(&rr_root).unwrap();
    std::fs::write(&request_file, saved).unwrap();
    let (status, revoked) = revoke(&fixture.app, &publication_id).await;
    assert_eq!(status, StatusCode::OK, "retry revoke must succeed: {revoked}");
    assert_eq!(revoked["status"], "revoked");
    assert_pointer_review_requests(&fixture, &publication_id, true);
}
