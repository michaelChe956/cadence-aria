#[test]
fn global_lookup_rejects_unique_legacy_path_identity_mismatch() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(create_input_for(
            "project_0002",
            "issue_0002",
            "work_item_0002",
        ))
        .expect("real attempt");
    let corrupt_alias_id = "coding_attempt_corrupt_alias";
    let corrupt_alias_path = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0001/coding-attempts/{corrupt_alias_id}.json"
    ));
    std::fs::create_dir_all(corrupt_alias_path.parent().expect("parent"))
        .expect("create alias parent");
    std::fs::write(
        corrupt_alias_path,
        serde_json::to_vec_pretty(&attempt).expect("serialize attempt"),
    )
    .expect("write corrupt alias");

    assert!(matches!(
        store.get_attempt_by_id(corrupt_alias_id),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            ..
        })
    ));
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("real attempt remains"),
        attempt
    );
}
