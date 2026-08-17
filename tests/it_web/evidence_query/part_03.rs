// C-4 Task 9（REQ-COD-05 it_web 验收）part 3/3：场景 ⑨⑩⑪⑫。
// 依赖 part_01 的 fixture 与 helper（同模块 include，符号直接可见）；无新增 imports。

// ---- ⑨ index stale：目标成员 revision 漂移 → index_stale=true ----
#[tokio::test]
async fn index_stale_is_reported_in_response_field() {
    let fx = seed_evidence_fixture();
    // 覆盖钉住索引 record 的目标成员 revision，模拟索引漂移。
    let drifted = index_record(
        &fx.aggregate_index_id,
        fx.api_id,
        fx.web_id,
        fx.api_checkout_id,
        fx.web_checkout_id,
        "rev-api-drifted",
        "rev-web",
        &fx.aggregate_root,
    );
    write_json(
        &index_record_path(&fx.paths, &fx.aggregate_index_id),
        &drifted,
    )
    .expect("overwrite index record");

    let app = fx.app();
    let (status, body) = evidence_query(app, &fx.token, "coder", "cross_repo_symbol").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["index_stale"], json!(true));
    assert!(
        body["text"].as_str().expect("text").contains("cross_repo_symbol"),
        "stale query must still return cross-member text"
    );
}

// ---- ⑩ 422 evidence_invalid_query 与 503 evidence_query_failed ----
#[tokio::test]
async fn invalid_query_returns_422_and_uninitialized_index_returns_503() {
    // 422：空查询词。
    let fx = seed_evidence_fixture();
    let app = fx.app();
    let (status, body) = evidence_query(app.clone(), &fx.token, "coder", "").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "evidence_invalid_query");

    // 503：聚合根未建 codegraph 索引 → 真实 `codegraph query` 非零退出 → QueryFailed。
    let fx_no_index = seed_evidence_fixture_with_options(EvidenceSourceSeed::Standard, false);
    let app_no_index = fx_no_index.app();
    let (status, body) = evidence_query(app_no_index, &fx_no_index.token, "coder", "cross_repo_symbol").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "evidence_query_failed");
    assert_eq!(body["details"]["reason_code"], "codegraph_query_failed");
}

// ---- ⑪ issue 级 worktree 复用：令牌重写（旧令牌 401、新令牌 200） ----
//
// 语义说明：T8 `execute_worktree_prepare` 每次 attempt 启动/恢复幂等重写令牌
// （`issue_evidence_token` 二次签发覆盖 worktree 明文与 attempt 分区哈希），解决
// issue 级 worktree 跨 attempt 复用下旧令牌残留。本场景以同 attempt 二次签发模拟
// 「新 attempt 复用同一 issue worktree」的令牌重写：旧令牌哈希被覆盖 → 反查无命中
// → 401；新令牌命中 Running attempt → 200。
#[tokio::test]
async fn token_rewrite_old_token_401_new_token_200() {
    let fx = seed_evidence_fixture();
    let old_token = fx.token.clone();

    let new_token = issue_evidence_token(&fx.paths, &fx.repo, &fx.attempt).expect("reissue token");
    assert_ne!(old_token, new_token, "reissue must rotate the token");

    // worktree 明文令牌被重写为新值。
    let token_file = fx.worktree.join(".aria").join("evidence-token");
    assert_eq!(
        fs::read_to_string(&token_file).expect("token file"),
        new_token,
        "worktree token file must hold the rewritten token"
    );

    let app = fx.app();
    let (old_status, old_body) =
        evidence_query(app.clone(), &old_token, "coder", "cross_repo_symbol").await;
    assert_eq!(old_status, StatusCode::UNAUTHORIZED, "{old_body}");
    assert_eq!(old_body["code"], "evidence_unauthorized");

    let (new_status, new_body) =
        evidence_query(app, &new_token, "coder", "cross_repo_symbol").await;
    assert_eq!(new_status, StatusCode::OK, "{new_body}");
    assert!(
        new_body["text"].as_str().expect("text").contains("cross_repo_symbol"),
        "new token must serve cross-member text"
    );
}

// ---- ⑫ 注入文件不进 commit：git add -A 后 .aria/evidence-token 不在 index ----
#[test]
fn injected_token_file_not_staged_by_git_add() {
    let fx = seed_evidence_fixture();

    // 令牌文件确实注入到 worktree .aria/。
    let token_file = fx.worktree.join(".aria").join("evidence-token");
    assert_eq!(
        fs::read_to_string(&token_file).expect("token file"),
        fx.token,
        "injected token file must exist in worktree"
    );

    // 公共 exclude（<repo>/.git/info/exclude）已幂等追加 `.aria/`。
    let exclude = fs::read_to_string(fx.repo.join(".git").join("info").join("exclude"))
        .expect("read common exclude");
    assert!(
        exclude.lines().any(|line| line.trim() == ".aria/"),
        "common exclude must contain `.aria/`: {exclude}"
    );

    // 真实 git 操作：worktree 内 git add -A 不得暂存注入文件。
    run_git(&fx.worktree, &["add", "-A"]);
    let staged = git_output(&fx.worktree, &["diff", "--cached", "--name-only"]);
    assert!(
        !staged.contains("evidence-token"),
        "git add -A must not stage injected token file, staged: {staged}"
    );
    assert!(
        !staged.contains(".aria/"),
        "git add -A must not stage injected .aria/ dir, staged: {staged}"
    );
}
