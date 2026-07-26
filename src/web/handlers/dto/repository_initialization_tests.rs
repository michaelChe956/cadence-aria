#[test]
fn repository_initialization_result_dto_sanitizes_paths_and_changed_paths() {
    let mut success = super::product_resources::create_repository_tests::registration_success();
    success.changed_paths = vec![
        "/private/repo/generated".to_string(),
        ".claude/rules/project.md".to_string(),
        "src/monkey.rs".to_string(),
    ];

    let value = serde_json::to_value(repository_initialization_result_dto(&success))
        .expect("repository initialization result dto");

    assert_eq!(value["repository"]["path"], "<path>");
    assert_eq!(value["repository"]["runtime_root"], "<path>");
    assert_eq!(
        value["initialization"]["changed_paths"],
        serde_json::json!(["<path>", ".claude/rules/project.md", "src/monkey.rs"])
    );
}

#[test]
fn repository_registration_success_preserves_all_source_modes() {
    for source_mode in ["online_clone", "online_update", "offline"] {
        let mut success = super::product_resources::create_repository_tests::registration_success();
        success.initialization.source_mode = source_mode.to_string();
        let value = serde_json::to_value(repository_initialization_result_dto(&success))
            .expect("repository initialization result dto");
        assert_eq!(value["initialization"]["source"], source_mode);
        assert!(value.get("warnings").is_none() && value.get("completed_at").is_none());
        assert!(value["initialization"].get("warnings").is_some());
    }
}

#[test]
fn repository_initialization_result_dto_exposes_git_finalize_warning() {
    let mut success = super::product_resources::create_repository_tests::registration_success();
    success.git_finalize_warning = Some(
        "git_finalize: 无 remote，已跳过 push，请手动推送".to_string(),
    );

    let value = serde_json::to_value(repository_initialization_result_dto(&success))
        .expect("repository initialization result dto");

    assert_eq!(
        value["initialization"]["git_finalize_warning"],
        "git_finalize: 无 remote，已跳过 push，请手动推送",
    );
}
