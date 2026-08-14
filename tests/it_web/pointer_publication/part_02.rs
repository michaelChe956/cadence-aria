// 场景 A（REQ-ENV-07 受控发布）：3 仓逻辑代码库 → POST 全量 → 每仓远端出现
// `aria-pointer/{repo_id}/{publication_id}` 分支（含标记块 commit）→ ReviewRequest 存在
// （owner_kind=pointer_publication）→ 成员主 checkout 无新 commit / 无指针文件 →
// 幂等重发（成员主 checkout 已含匹配指针块时）→ 条目 Skipped、无新分支。

#[tokio::test]
async fn pointer_publication_scenario_a_full_publish_pushes_branches_and_idempotent_resend_skips() {
    let fixture = setup_pointer_fixture(&[("api", true), ("web", true), ("shared", true)]);
    let heads_before: Vec<String> = fixture.members.iter().map(|member| member.head_sha()).collect();

    let (status, publication) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK, "full publish must succeed: {publication}");
    assert_eq!(publication["status"], "completed_all");
    assert_eq!(publication["batch_kind"], "full");
    assert_eq!(publication["entries"].as_array().unwrap().len(), 3);
    let publication_id = publication["id"].as_str().unwrap().to_string();

    for (index, member) in fixture.members.iter().enumerate() {
        let member_repo_id = member.logical_id.0.to_string();
        let member_entry = entry(&publication, &member_repo_id);
        assert_eq!(member_entry["state"], "review_created", "{publication}");
        let branch_name = member_entry["branch_name"].as_str().unwrap();
        assert_eq!(
            branch_name,
            format!("aria-pointer/{member_repo_id}/{publication_id}")
        );
        let commit_sha = member_entry["commit_sha"].as_str().unwrap();
        assert!(!commit_sha.is_empty());

        // 远端出现 aria-pointer 分支，且分支上的 .aria-pointer.md 含标记块 commit。
        let bare = member.bare_remote.as_ref().expect("bare remote");
        assert!(
            remote_has_branch(bare, branch_name),
            "remote branch {branch_name} must exist"
        );
        let pointer_file = remote_branch_file(bare, branch_name, ".aria-pointer.md");
        assert!(
            pointer_file.contains("aria-logical-codebase-pointer:start"),
            "pointer block start marker missing: {pointer_file}"
        );
        assert!(
            pointer_file.contains(&format!("repo_id: {member_repo_id}")),
            "pointer block repo_id missing: {pointer_file}"
        );
        assert!(
            pointer_file.contains(&format!("logical_codebase_id: {}", fixture.logical_codebase_id)),
            "pointer block logical_codebase_id missing: {pointer_file}"
        );

        // 成员主 checkout 不被污染：HEAD 不变、主 checkout 无指针文件。
        assert_eq!(
            member.head_sha(),
            heads_before[index],
            "main checkout HEAD must be unchanged"
        );
        assert!(
            !member.repo_path.join(".aria-pointer.md").exists(),
            "main checkout must not contain pointer file"
        );
    }

    // ReviewRequest 独立分区落盘，owner_kind=pointer_publication 且未 revoked。
    assert_pointer_review_requests(&fixture, &publication_id, false);

    // 幂等重发：把「已合并」的指针块写回成员主 checkout（不 commit，模拟人工合并后
    // 主分支已含标记块）→ 再次全量 → 每仓 Skipped、无新分支（branch_name 为空）。
    for member in &fixture.members {
        fixture.plant_merged_pointer_block(member);
    }
    let (status, resend) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK, "idempotent resend must succeed: {resend}");
    assert_eq!(resend["status"], "completed_all");
    let resend_id = resend["id"].as_str().unwrap().to_string();
    for member in &fixture.members {
        let member_repo_id = member.logical_id.0.to_string();
        let member_entry = entry(&resend, &member_repo_id);
        assert_eq!(member_entry["state"], "skipped", "{resend}");
        assert!(member_entry["branch_name"].is_null(), "no new branch: {resend}");
        let branch_name = format!("aria-pointer/{member_repo_id}/{resend_id}");
        assert!(
            !remote_has_branch(member.bare_remote.as_ref().unwrap(), &branch_name),
            "resend must not create new remote branch {branch_name}"
        );
    }
}

// 场景 B（增量）：新增成员 → POST Incremental → 仅新成员有新分支。

#[tokio::test]
async fn pointer_publication_scenario_b_incremental_only_publishes_new_member() {
    let mut fixture = setup_pointer_fixture(&[("api", true), ("web", true)]);
    let (status, first) = create_publication(&fixture.app, "full").await;
    assert_eq!(status, StatusCode::OK, "full publish must succeed: {first}");
    let first_id = first["id"].as_str().unwrap().to_string();
    let original_ids: Vec<String> = fixture
        .members
        .iter()
        .map(|member| member.logical_id.0.to_string())
        .collect();

    // 新增第三个成员（真实 git 仓库 + bare 远端），并更新 manifest。
    let added = fixture.add_member("gateway", true);
    let added_id = added.logical_id.0.to_string();

    let (status, incremental) = create_publication(&fixture.app, "incremental").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "incremental publish must succeed: {incremental}"
    );
    assert_eq!(incremental["status"], "completed_all");
    assert_eq!(incremental["batch_kind"], "incremental");
    assert_eq!(incremental["entries"].as_array().unwrap().len(), 1);
    let incremental_id = incremental["id"].as_str().unwrap().to_string();

    let added_entry = entry(&incremental, &added_id);
    assert_eq!(added_entry["state"], "review_created", "{incremental}");
    let added_branch = added_entry["branch_name"].as_str().unwrap();
    assert_eq!(
        added_branch,
        format!("aria-pointer/{added_id}/{incremental_id}")
    );
    assert!(
        remote_has_branch(added.bare_remote.as_ref().unwrap(), added_branch),
        "new member must have remote pointer branch"
    );

    // 旧成员在新增量批次中不得被重发：无对应条目、无该批次的远端分支。
    for original_id in &original_ids {
        assert!(
            incremental["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["member_repo_id"].as_str() != Some(original_id.as_str())),
            "old member {original_id} must not appear in incremental entries"
        );
        let original = fixture
            .members
            .iter()
            .find(|member| member.logical_id.0.to_string() == *original_id)
            .unwrap();
        let branch_name = format!("aria-pointer/{original_id}/{incremental_id}");
        assert!(
            !remote_has_branch(original.bare_remote.as_ref().unwrap(), &branch_name),
            "old member must not get a branch for the incremental batch"
        );
    }

    // 首个全量批次的旧成员分支仍存在（增量不回收历史批次分支）。
    for original_id in &original_ids {
        let original = fixture
            .members
            .iter()
            .find(|member| member.logical_id.0.to_string() == *original_id)
            .unwrap();
        let branch_name = format!("aria-pointer/{original_id}/{first_id}");
        assert!(
            remote_has_branch(original.bare_remote.as_ref().unwrap(), &branch_name),
            "full batch branch must remain after incremental publish"
        );
    }
}
