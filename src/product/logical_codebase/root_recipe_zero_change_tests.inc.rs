// REQ-BOOT-17（root recipe turn 零变更必须显式失败）的单测：
// 零动作 ⇒ 该 step 判失败（失败事实含 provider / step / 零变更）；
// 有动作 ⇒ 照既有语义成功。
//
// 判据来源：既有 receipt auditor 的「有无变更」事实（`observed_changes`
// 是否为空）——本模块不引入第二套「有无变更」判定。

#[cfg(test)]
mod zero_change_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index;
    use crate::product::models::ProviderName;

    const PROJECT_ID: &str = "project_0001";
    const LC_ID: &str = "logical_codebase_0001";
    const OPERATION_ID: &str = "aggregate_initialization_0001";
    const RECORDED_AT: &str = "2026-10-01T00:00:00Z";

    /// 非 Git 聚合根 + per-LC receipt store 的最小 fixture。
    fn fixture() -> (
        tempfile::TempDir,
        RootRecipeReceiptStore,
        std::path::PathBuf,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().to_path_buf());
        let store = RootRecipeReceiptStore::for_lc(paths.clone(), LC_ID);
        let root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(root.join(".aria/aggregate")).unwrap();
        let root = root.canonicalize().unwrap();
        (temp, store, root)
    }

    /// 固定命令索引的第 command_index 条（1..=4）。
    fn command_spec(command_index: usize) -> (AggregateInitializationStepKind, &'static str) {
        let (step, _, command) = root_recipe_command_index()[command_index - 1];
        (step, command)
    }

    /// 跑一条命令的完整审计：`write` 决定窗口内是否产生落盘差异。
    fn audited_command(
        root: &std::path::Path,
        command_index: usize,
        write: Option<&str>,
    ) -> RootRecipeCommandReceipt {
        let auditor = RootRecipeFilesystemAuditor::new();
        let (step, command) = command_spec(command_index);
        let watch = auditor
            .before_command(OPERATION_ID, root, step, command_index, command)
            .unwrap();
        if let Some(artifact) = write {
            std::fs::write(root.join(".aria/aggregate").join(artifact), "{}\n").unwrap();
        }
        auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap()
    }

    /// 零动作的 turn（窗口内零落盘差异、verdict 仍 `Allowed`）⇒ 该 step
    /// 判失败，失败事实含 **provider / step / 零变更** 三项。
    #[test]
    fn zero_change_turn_fails_with_provider_step_and_zero_change_facts() {
        let (_temp, store, root) = fixture();
        let receipt = audited_command(&root, 1, None);
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
        assert!(
            receipt.observed_changes.is_empty(),
            "the audited turn must be a zero-action turn: {:?}",
            receipt.observed_changes
        );

        let reason = store
            .append_turn_receipts(
                PROJECT_ID,
                receipt.step,
                &ProviderName::ClaudeCode,
                vec![receipt],
            )
            .expect("the audit facts must stay durable")
            .expect("a zero-action root recipe turn must fail the step");
        assert!(
            reason.contains("claude_code"),
            "the failure fact must name the provider: {reason}"
        );
        assert!(
            reason.contains("pre_check"),
            "the failure fact must name the step: {reason}"
        );
        assert!(
            reason.contains("zero_change"),
            "the failure fact must state the zero change: {reason}"
        );
    }

    /// 有动作的 turn ⇒ 照既有语义成功（审计事实照旧 durable 落盘）。
    #[test]
    fn changed_turn_keeps_the_existing_success_semantics() {
        let (_temp, store, root) = fixture();
        let receipt = audited_command(&root, 1, Some("precheck-report.json"));
        assert!(!receipt.observed_changes.is_empty());
        let zero_change = store
            .append_turn_receipts(
                PROJECT_ID,
                receipt.step,
                &ProviderName::ClaudeCode,
                vec![receipt],
            )
            .expect("a turn with changes must keep the existing success semantics");
        assert!(
            zero_change.is_none(),
            "a turn with changes must not be judged a zero-change turn: {zero_change:?}"
        );
        let stored = store
            .list_commands(PROJECT_ID, OPERATION_ID)
            .expect("command receipts durable");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].verdict, RootRecipeCommandVerdict::Allowed);
    }
}
