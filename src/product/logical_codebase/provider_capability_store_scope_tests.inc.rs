// provider capability store 的**作用域唯一性**用例（REQ-PCR-07）。
//
// 纯搬移：原内联在 `provider_capability_store.rs` 的 `mod tests` 里，为满足
// `tests/it_core/large_file_guard.rs` 的行数门禁（该文件的登记上限不可增长）
// 而拆出——用例体逐字不变，只换承载文件。

/// REQ-PCR-07：capability 记录的**唯一作用域 = 机器级**——① 机器级文件存在；
/// ② 项目目录与逻辑代码库子树下**不存在** capability 记录（出现即为第二
/// 作用域 = 缺陷）。
#[test]
fn provider_capability_store_scope_is_machine_global_only() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let store = ProviderCapabilityStore::new(paths.clone());
    store.ensure_bootstrap().unwrap();

    let global_path = paths
        .provider_capabilities_root()
        .join("capabilities.json");
    assert!(
        global_path.is_file(),
        "capability 记录必须落机器级路径: {}",
        global_path.display()
    );
    let project_root = paths.project_root("project_0001");
    assert!(
        !project_root.join("capabilities.json").exists(),
        "项目目录下不得存在 capability 记录（唯一作用域 = 机器级）"
    );
    assert!(
        !paths
            .logical_codebase_root("project_0001")
            .join("capabilities.json")
            .exists(),
        "项目级 logical-codebase 子树下不得存在 capability 记录（唯一作用域 = 机器级）"
    );
    let lc_root = paths.logical_codebases_root("project_0001").join("lc_0001");
    assert!(
        !lc_root.join("capabilities.json").exists(),
        "逻辑代码库子树下不得存在 capability 记录（唯一作用域 = 机器级）"
    );
}
