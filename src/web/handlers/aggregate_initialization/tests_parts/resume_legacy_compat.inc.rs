// -------------------------------------------------------------------
// 向后兼容（2026-11-03 块2 审计快照口径改造）：改造前写下的 receipt 里
// 文件条目带 `content_digest`，改造后的快照只记元数据（`content_digest`
// 恒 `None`，内容哈希只对变更清单计算）。恢复分支
// （`verify_resume_against_final_audit`）对这两种快照的比较口径由此钉住：
// 旧摘要不可复验时必须**如实**报告并给出「重新审计」的处置，绝不谎报
// `path X changed since the audited final command`（那是对存量数据的误报），
// 也绝不一律当作「没变」（新快照没有内容证据，放行等于漏报）。
// -------------------------------------------------------------------

/// 把 durable 的末命令 receipt 改写成「改造前」形态：`after_snapshot` 的
/// 文件条目带上内容摘要（旧快照对每个文件都算摘要），其余字段原样保留。
/// 返回被改写的文件条目数（0 表示该根下没有文件，用例无从构造）。
fn rewrite_command_receipt_as_legacy_receipt(path: &std::path::Path) -> usize {
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("command receipt bytes"))
            .expect("command receipt json");
    let entries = receipt["after_snapshot"]["entries"]
        .as_array_mut()
        .expect("after snapshot entries");
    let mut files = 0;
    for entry in entries.iter_mut() {
        if entry["kind"] == "file" {
            entry["content_digest"] = serde_json::json!("sha256:legacy-audit-digest");
            files += 1;
        }
    }
    std::fs::write(path, serde_json::to_vec(&receipt).expect("serialize receipt"))
        .expect("rewrite legacy command receipt");
    files
}

/// 走一次「末命令已 Allowed 审计」的中断窗口（command04 durable、发布未开始）。
async fn root_policy_interrupt_after_command_audit(
    fixture: &TrustRouteFixture,
    key: &str,
) -> (AggregateInitializationOperationStore, String) {
    let operations = root_policy_operations(fixture);
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterCommandAudit,
    );
    assert_eq!(root_policy_start(fixture, key).await, operation_id);
    let interrupted = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        interrupted.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );
    assert_eq!(
        interrupted.failed_step,
        Some(AggregateInitializationStepKind::OpenspecAndExamples)
    );
    (operations, operation_id)
}

/// durable 失败记录的展示文本（ProviderTurn 的原始 reason 落在 `action`）。
fn root_policy_failure_action(
    operation: &crate::product::logical_codebase::AggregateInitializationOperation,
) -> String {
    operation
        .error
        .as_ref()
        .expect("the blocked resume must record a durable failure")
        .action
        .clone()
}

/// 存量数据回归：旧 receipt（带内容摘要）+ 新格式快照（`content_digest`
/// 恒 `None`）⇒ **不得**报「changed since the audited final command」，
/// 必须如实表达「无法用旧摘要复验」并给出可执行处置（重新审计）。
#[tokio::test]
async fn root_policy_resume_legacy_receipt_digest_is_reported_as_unverifiable() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let (operations, operation_id) =
        root_policy_interrupt_after_command_audit(&fixture, "root-policy-resume-legacy-1").await;
    let command04 = root_policy_command_receipt_path(&fixture, &operation_id, 4);
    assert!(
        rewrite_command_receipt_as_legacy_receipt(&command04) > 0,
        "the audited aggregate root must contain files for the legacy shape to matter"
    );

    root_policy_continue(&fixture, &operation_id, "legacy-digest").await;
    let blocked = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        blocked.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "{:?}",
        blocked.error
    );
    let action = root_policy_failure_action(&blocked);
    assert!(
        !action.contains("changed since the audited final command"),
        "旧摘要不可比不得谎报「已变更」（存量数据误报），实际: {action}"
    );
    assert!(
        action.contains("legacy content digest") && action.contains("cannot re-verify"),
        "必须如实表达「无法用旧摘要复验」，实际: {action}"
    );
    assert!(
        action.contains("re-run the audited final command"),
        "必须给出可执行处置（重新审计末命令），实际: {action}"
    );
    // 重试 Continue 只会拿到同一条错误 ⇒ 该失败必须声明「重试无用」，
    // 否则 UI 会把它当成可重试，走成死循环。
    assert!(
        !blocked
            .error
            .as_ref()
            .expect("the blocked resume must record a durable failure")
            .retryable,
        "旧摘要无法复验必须先重新审计，Continue 重试无用 ⇒ retryable 必须为 false"
    );
    assert!(
        root_policy_receipts(&fixture)
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .is_none(),
        "不可复验的恢复不得签发最终 receipt"
    );
}

/// 现有行为不变（新格式一侧）：新格式 receipt + 新格式快照且根未改动 ⇒
/// 恢复照常完成、命令审计字节不变、不重新启动 provider。
/// （`root_policy_resume_after_command_audit` 对同一事实的更细断言保留不动；
/// 本条从「两种快照格式共存」的角度再钉一次。）
#[tokio::test]
async fn root_policy_resume_new_receipt_stays_resumable() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let (operations, operation_id) =
        root_policy_interrupt_after_command_audit(&fixture, "root-policy-resume-new-format-1").await;
    let command04 = root_policy_command_receipt_path(&fixture, &operation_id, 4);
    let command04_bytes = std::fs::read(&command04).expect("command04 audit is durable");
    let launches_before = fixture.factory.audit().stream_launches();

    root_policy_continue(&fixture, &operation_id, "new-format").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "resume must not launch the provider again"
    );
    assert_eq!(
        std::fs::read(&command04).expect("command04 audit bytes"),
        command04_bytes,
        "the durable command04 audit must stay byte-identical"
    );
    assert!(
        root_policy_receipts(&fixture)
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .is_some(),
        "a clean resume must freeze the final receipt"
    );
}

/// 新格式之间的比较必须仍然敏感：审计后把规则文件换成同名目录（`kind`
/// 变了）⇒ 恢复必须拒绝并点名漂移路径（绝不因「两侧都无内容摘要」而放行）。
#[tokio::test]
async fn root_policy_resume_new_receipt_keeps_metadata_sensitivity() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let (operations, operation_id) =
        root_policy_interrupt_after_command_audit(&fixture, "root-policy-resume-sensitive-1").await;
    let agents = root_policy_canonical_root(&fixture).join("AGENTS.md");
    assert!(agents.is_file(), "the audited aggregate root must hold AGENTS.md");
    std::fs::remove_file(&agents).expect("remove the audited AGENTS.md");
    std::fs::create_dir(&agents).expect("replace the audited AGENTS.md with a directory");

    root_policy_continue(&fixture, &operation_id, "metadata-sensitive").await;
    let blocked = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        blocked.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "{:?}",
        blocked.error
    );
    let action = root_policy_failure_action(&blocked);
    assert!(
        action.contains("changed since the audited final command"),
        "新格式之间的元数据差异仍必须判「已变更」，实际: {action}"
    );
    assert!(
        action.contains("AGENTS.md"),
        "漂移必须点名路径，实际: {action}"
    );
}
