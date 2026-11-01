// issue 级跨仓最终评审 engine 测试（REQ-GFR-CR-04「跨仓评审材料与 provider
// 选择」，设计 B §四 B2–B5）。
//
// - provider 取值顺序：快照 `.internal_reviewer` → 缺失回退同一快照的
//   `.code_reviewer` → 两者皆缺 `status=Skipped`（不启动 provider）。
// - 解析层 `default_source_stage = IssueCrossRepoReview`：provider 输出省略
//   `source_stage` 时每条 finding 落为 `issue_cross_repo_review`。
// - advisory：评审是建议性旁路，不开评审窗、不产出 acceptance。

use super::issue_cross_repo_review_fixture::*;
use super::*;
use crate::product::coding_models::IssueCrossRepoReviewStatus;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn cross_repo_review_falls_back_to_code_reviewer_provider() {
    let (_root, store, engine, attempts) = cross_repo_fixture(2);
    // 存量旧 JSON 形态：缺 `internal_reviewer`，`code_reviewer` 在场。
    seed_role_provider_config(&store, &attempts, None, Some(ProviderName::ClaudeCode));
    let provider = CountingFakeProvider::new(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review executes")
        .expect("review record");

    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_ne!(review.status, IssueCrossRepoReviewStatus::Skipped);
    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert_eq!(
        provider.starts(),
        1,
        "code_reviewer fallback must start provider exactly once"
    );
    assert_eq!(review.repository_sections.len(), 2);
    assert!(
        provider
            .first_prompt()
            .contains("Coding Workspace IssueCrossRepoFinalReview"),
        "prompt must carry the cross repo review title"
    );
    let persisted = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .expect("load persisted review")
        .expect("review persisted");
    assert_eq!(persisted.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(persisted.id, review.id);
}

#[tokio::test]
async fn cross_repo_review_skipped_when_both_providers_missing() {
    let (_root, store, engine, attempts) = cross_repo_fixture(2);
    seed_role_provider_config(&store, &attempts, None, None);
    let provider = CountingFakeProvider::new("must not run");

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review call succeeds")
        .expect("review record");

    assert_eq!(review.status, IssueCrossRepoReviewStatus::Skipped);
    assert_eq!(
        provider.starts(),
        0,
        "no provider must start when both reviewers missing"
    );
    let persisted = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .expect("load persisted review")
        .expect("skipped review persisted");
    assert_eq!(persisted.status, IssueCrossRepoReviewStatus::Skipped);
}

#[tokio::test]
async fn cross_repo_review_marks_every_finding_source_stage() {
    let (_root, _store, engine, attempts) = cross_repo_fixture(2);
    seed_role_provider_config(&_store, &attempts, Some(ProviderName::ClaudeCode), None);
    let provider = CountingFakeProvider::new(
        "{\"verdict\":\"request_changes\",\"summary\":\"跨仓契约存在不一致\",\"findings\":[\
         {\"severity\":\"warning\",\"file_path\":\"services/api/gateway.rs\",\"line\":10,\
         \"message\":\"调用面与上游契约不一致\",\"required_action\":\"建议对齐契约（仅建议）\"},\
         {\"severity\":\"info\",\"file_path\":\"crates/core/lib.rs\",\"line\":1,\
         \"message\":\"上下游交付闭环缺少说明\"}]}",
    );

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review executes")
        .expect("review record");

    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(review.findings.len(), 2);
    for finding in &review.findings {
        assert_eq!(
            finding.source_stage,
            CodingExecutionStage::IssueCrossRepoReview,
            "findings without source_stage must default to issue_cross_repo_review"
        );
    }
}

// ---------------------------------------------------------------------------
// Task 17：触发接线（REQ-GFR-CR-03「issue 级跨仓最终评审恰好一次」，
// 设计 B §七 E）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_repo_review_triggers_exactly_once_after_all_work_items_completed() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(3);
    let (resolver, starts, prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    // 收口钩子（WorkItem scope 形态）尾部触发 issue 级跨仓最终评审。
    engine
        .maybe_complete_issue_delivery(&attempts[2], &resolver)
        .await
        .expect("issue delivery completion");

    let review = wait_for_review_record(&store).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1, "跨仓评审必须恰好执行一次");
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        3,
        "材料必须覆盖全部 3 个仓"
    );
    let logical_ids = review
        .repository_sections
        .iter()
        .map(|section| section.logical_repository_id.clone())
        .collect::<Vec<String>>();
    let mut ascending = logical_ids.clone();
    ascending.sort();
    assert_eq!(
        logical_ids, ascending,
        "跨仓材料分段必须按 logical_repository_id 升序"
    );
    assert_ne!(
        logical_ids[0], logical_ids[2],
        "分段顺序不得退化为 attempt 创建顺序"
    );
    let prompt = prompts.lock().unwrap()[0].clone();
    assert_eq!(
        prompt.matches("Repository #").count(),
        3,
        "材料必须含 3 段逐仓 diff"
    );
    for index in 0..3 {
        assert!(
            prompt.contains(&format!("repo_{index}.txt")),
            "材料缺少第 {index} 仓的 diff"
        );
    }
}

#[tokio::test]
async fn cross_repo_review_is_not_triggered_twice() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(3);
    let (resolver, starts, prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[2], &resolver)
        .await
        .expect("first issue delivery completion");
    let first = wait_for_review_record(&store).await;

    // 重复进入收口钩子 → 不得重复评审、不得新增工件。
    engine
        .maybe_complete_issue_delivery(&attempts[2], &resolver)
        .await
        .expect("second issue delivery completion");
    let second = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .expect("load persisted review")
        .expect("cross repo review record");

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "重复进入收口钩子不得重复评审"
    );
    assert_eq!(prompts.lock().unwrap().len(), 1);
    assert_eq!(first.id, second.id, "评审记录按 issue 唯一");
    assert_eq!(
        first.created_at, second.created_at,
        "评审记录不得被覆盖重写"
    );
    assert!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .is_empty(),
        "重复触发不得新增留痕"
    );
    let review_root = store
        .paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-cross-repo-reviews");
    let review_files = fs::read_dir(&review_root)
        .expect("cross repo review root")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert_eq!(review_files, 1, "评审本体按 issue 唯一，不得新增第二份");
}

#[tokio::test]
async fn cross_repo_review_material_covers_attempt_awaiting_final_confirm() {
    // group 形态的收口钩子落在 attempt「已到人工最终确认、尚未 Completed」的时刻：
    // 该仓开发已定稿，必须进入跨仓材料——否则该仓缺段（甚至整份材料为空 → 评审
    // 永不执行，多仓形态形同虚设）。
    let (_root, store, engine, attempts) =
        cross_repo_trigger_fixture_with_states(2, &[false, true]);
    let (resolver, starts, prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    let review = wait_for_review_record(&store).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        2,
        "停在人工最终确认的仓也必须进入跨仓材料"
    );
    let prompt = prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("repo_0.txt"));
    assert!(prompt.contains("repo_1.txt"));
}

// ---------------------------------------------------------------------------
// P1-1 / P1-2 / P2-3 修复回归：Reviewer 窗口合同、触发不阻塞、issue 形态判据。
// ---------------------------------------------------------------------------

/// P1-1：交给 adapter 前的 input 必须已携带非空 `tool_policy`，且能通过真实
/// adapter spawn 前的同一守卫（`validate_tool_policy_for_role`）——fake
/// provider 不跑该守卫，因此此处显式调用守卫断言。修复前
/// `tool_policy = None`，守卫对 `Reviewer + None` 直接 `PolicyRequired` 拒启
/// （生产恒 `Failed("provider 启动失败…")`，测试不可见）。
#[tokio::test]
async fn cross_repo_review_input_carries_window_unbound_reviewer_contract() {
    let (_root, store, engine, attempts) = cross_repo_fixture(2);
    seed_role_provider_config(&store, &attempts, Some(ProviderName::ClaudeCode), None);
    let provider = CountingFakeProvider::new(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review executes")
        .expect("review record");

    let input = provider.first_input();
    assert_eq!(
        input.role,
        AdapterRole::Reviewer,
        "跨仓评审以 Reviewer wire 角色运行"
    );
    let policy = input
        .tool_policy
        .clone()
        .expect("input.tool_policy 必须在交给 adapter 前设置（P1-1）");
    crate::cross_cutting::streaming_provider::validate_tool_policy_for_role(
        &AdapterRole::Reviewer,
        Some(&policy),
    )
    .expect("真实 adapter 的 spawn 前守卫必须放行本策略（P1-1）");
    // 合同身份与评审窗口解绑：等于本 issue 级评审记录的 id（durable、按 issue
    // 唯一），不复用任何评审窗 id。
    assert_eq!(
        policy,
        ProviderToolPolicy::reviewer_verify(review.id.clone()),
        "ReviewerVerify 合同引用必须是本评审记录 id"
    );
    assert!(!review.id.trim().is_empty());
    // P1-2：advisory 评审自带短时限（≤120s），不与 3h 的整 run 总上限同界。
    assert!(
        input.timeout_secs <= 120,
        "advisory 评审时限必须 ≤120s，实际 {}s",
        input.timeout_secs
    );
}

/// P1-2：收口钩子触发跨仓评审必须与 provider 耗时无关地秒级返回——挂起
/// provider（楔死）下调用方 2s 内返回；评审在后台按 advisory 短时限收敛为
/// `Failed` 落盘。修复前触发同步 await provider 终态（总超时 3h），调用方
/// 被挂住直到超时。
#[tokio::test]
async fn cross_repo_review_trigger_returns_without_waiting_for_provider() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(2);
    let (resolver, starts) = hanging_review_provider();

    let began = std::time::Instant::now();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        engine.maybe_complete_issue_delivery(&attempts[1], &resolver),
    )
    .await
    .expect("触发必须与 provider 无关地秒级返回（REQ-GFR-CR-05 不阻断）")
    .expect("issue delivery completion");
    assert!(
        began.elapsed() < std::time::Duration::from_secs(2),
        "收口钩子不得等待 provider 终态，实际耗时 {:?}",
        began.elapsed()
    );

    // 登记后评审确实在后台执行（provider 被启动），而非静默不跑。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while starts.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "后台评审任务必须启动 provider"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // 楔死 provider 按 advisory 短时限收敛为 `Failed` 落盘（测试态时限 1s）。
    let review = wait_for_review_record(&store).await;
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Failed);
    assert!(
        review
            .failure_reason
            .as_deref()
            .unwrap_or_default()
            .contains("超时"),
        "挂起 provider 必须按 advisory 时限收敛为 Failed：{:?}",
        review.failure_reason
    );
}

/// P2-3：多仓 issue（`logical_codebase_id=Some`）的全部 work item 落在同一仓
/// 时仍必须评审——形态按 issue 判，不按 attempt 快照的去重仓数。修复前
/// `distinct_logical_repos < 2` 把这类 issue 全部静默跳过（无记录、无提醒）。
#[tokio::test]
async fn cross_repo_issue_with_all_work_items_in_one_repo_still_reviewed() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture_single_repo(2);
    let (resolver, starts, _prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    let review = wait_for_review_record(&store).await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "全部 work item 同仓的多仓 issue 仍必须评审（P2-3）"
    );
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        2,
        "每 attempt 一段（同仓也分段）"
    );
    let distinct_repos: std::collections::BTreeSet<&str> = review
        .repository_sections
        .iter()
        .map(|section| section.logical_repository_id.as_str())
        .collect();
    assert_eq!(distinct_repos.len(), 1, "两段同属一个逻辑仓");
}

/// P2-3 负例：单仓 issue（`logical_codebase_id=None`）即使 attempt 快照横跨
/// 多个逻辑仓也不评审——快照仓数不再是触发判据，REQ-GFR-CR-02「单仓不产生
/// 跨仓评审」的引擎侧兜底改挂在 issue 形态上。
#[tokio::test]
async fn single_repo_issue_never_cross_repo_reviewed_even_with_multi_repo_snapshots() {
    let (_root, store, engine, attempts) = single_repo_trigger_fixture(2);
    let (resolver, starts, _prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    // 修复前：attempt 快照跨 2 仓 → 评审执行、记录立即落盘。修复后：单仓
    // issue 不评审——轮询确认记录在宽限期内始终缺失。
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        assert!(
            store
                .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
                .expect("load review")
                .is_none(),
            "单仓 issue 不得产生跨仓评审记录（P2-3）"
        );
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "单仓 issue 不得启动评审 provider"
    );
}
