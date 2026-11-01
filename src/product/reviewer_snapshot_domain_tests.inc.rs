// 零写对账域测试(owner 2026-10-23 裁定「乙」/REQ-ENV-REVIEW-B)。
// 经 include! 挂入 reviewer_snapshot::tests,复用其夹具与 git 辅助。

use crate::product::reviewer_snapshot_domain::DOMAIN_BOUNDARY_DISCLOSURE;

fn rel_set(manifest: &ReviewerBaselineManifest) -> HashSet<String> {
    manifest
        .entries
        .iter()
        .map(|entry| entry.rel_path.clone())
        .collect()
}

// ---------- 域口径:ignored 不入基线,tracked/untracked 非 ignored 在域内 ----------

#[tokio::test]
async fn capture_excludes_ignored_but_keeps_tracked_and_untracked() {
    let fixture = build_fixture(Layout::Plain);
    let worktree = fixture.worktree.clone();
    // ignored 产物:node_modules/、dist/、ignored.log(既有模式);
    // .gitignore 是 tracked 文件:改写保留为 Coder dirty,仍在域内。
    fs::write(
        worktree.join(".gitignore"),
        "ignored.log\nnode_modules/\ndist/\n*.forced\n",
    )
    .unwrap();
    fs::create_dir_all(worktree.join("node_modules/dep")).unwrap();
    fs::write(worktree.join("node_modules/dep/dep.js"), "dep\n").unwrap();
    fs::create_dir_all(worktree.join("dist")).unwrap();
    fs::write(worktree.join("dist/out.js"), "out\n").unwrap();
    // tracked 但命中 ignore 模式(git add -f):git 口径仍在域内。
    fs::write(worktree.join("forced.gen"), "forced\n").unwrap();
    git_ok(&worktree, &["add", "-f", "forced.gen"]);

    let identity = lock_identity(&worktree, "lease-dom-1");
    let evidence = quiescence_evidence("win-dom-1").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-dom-1", &identity, &evidence))
        .expect("capture baseline");
    let manifest = store.load_manifest("win-dom-1").expect("load manifest");
    let rels = rel_set(&manifest);
    assert!(
        !rels.iter().any(|rel| rel.starts_with("node_modules/")),
        "node_modules 不入基线(裁定乙): {rels:?}"
    );
    assert!(
        !rels.iter().any(|rel| rel.starts_with("dist/")),
        "dist 不入基线"
    );
    assert!(!rels.contains("ignored.log"), "ignored.log 不入基线");
    assert!(
        rels.contains("forced.gen"),
        "tracked 但命中 ignore 模式仍在域内"
    );
    assert!(
        rels.contains("untracked_new.txt"),
        "untracked 非 ignored 仍在域内"
    );
    assert!(rels.contains("tracked.txt"), "tracked 仍在域内");
    assert!(rels.contains(".gitignore"), ".gitignore(tracked)在域内");
    // 冻结:根 .gitignore 基线内容 + git 枚举的 ignored 清单 + 披露。
    assert_eq!(
        manifest.ignore_rules_freeze.gitignores.get(""),
        Some(&"ignored.log\nnode_modules/\ndist/\n*.forced\n".to_string()),
        "根 .gitignore 内容须冻结"
    );
    assert!(
        manifest
            .ignore_rules_freeze
            .ignored_paths
            .iter()
            .any(|path| path == "node_modules/" || path.starts_with("node_modules/")),
        "基线 ignored 清单须含 node_modules: {:?}",
        manifest.ignore_rules_freeze.ignored_paths
    );
    assert!(
        manifest
            .ignore_rules_freeze
            .ignored_paths
            .iter()
            .any(|path| path == "ignored.log"),
        "基线 ignored 清单须含 ignored.log"
    );
    assert_eq!(manifest.domain_boundary, DOMAIN_BOUNDARY_DISCLOSURE);
}

// ---------- 恢复不触碰、不删除 ignored 产物 ----------

#[tokio::test]
async fn restore_never_touches_or_deletes_ignored_artifacts() {
    let fixture = build_fixture(Layout::Plain);
    let worktree = fixture.worktree.clone();
    fs::write(
        worktree.join(".gitignore"),
        "ignored.log\nnode_modules/\ndist/\n",
    )
    .unwrap();
    fs::create_dir_all(worktree.join("node_modules/dep")).unwrap();
    fs::write(worktree.join("node_modules/dep/dep.js"), "dep\n").unwrap();

    let identity = lock_identity(&worktree, "lease-dom-2");
    let evidence = quiescence_evidence("win-dom-2").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-dom-2", &identity, &evidence))
        .expect("capture baseline");

    // Reviewer 折腾 ignored 产物:改既有 + 新建 + 改 ignored.log。
    fs::write(
        worktree.join("node_modules/dep/dep.js"),
        "reviewer-mangled\n",
    )
    .unwrap();
    fs::write(worktree.join("node_modules/reviewer-new.txt"), "new\n").unwrap();
    fs::write(worktree.join("ignored.log"), "reviewer-secret\n").unwrap();

    let report = restore_report(
        store
            .restore_baseline("win-dom-2", &identity)
            .expect("restore"),
    );
    // 原样保留:不恢复、不删除(已知边界)。
    assert_eq!(
        fs::read_to_string(worktree.join("node_modules/dep/dep.js")).unwrap(),
        "reviewer-mangled\n"
    );
    assert_eq!(
        fs::read_to_string(worktree.join("node_modules/reviewer-new.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(
        fs::read_to_string(worktree.join("ignored.log")).unwrap(),
        "reviewer-secret\n"
    );
    // 不入恢复动作与残留清单;报告须披露边界,不得冒称全域零写。
    assert!(
        !report
            .worktree_removed_extras
            .iter()
            .any(|rel| rel.starts_with("node_modules/")
                || rel.starts_with("dist/")
                || rel == "ignored.log"),
        "ignored 产物不得当残留清除: {:?}",
        report.worktree_removed_extras
    );
    assert!(
        !report
            .worktree_fixed_entries
            .iter()
            .any(|rel| rel.starts_with("node_modules/") || rel == "ignored.log"),
        "ignored 产物不得入恢复动作: {:?}",
        report.worktree_fixed_entries
    );
    assert_eq!(report.domain_boundary, DOMAIN_BOUNDARY_DISCLOSURE);
}

// ---------- 域内写入仍严格零写:检出并恢复 ----------

#[tokio::test]
async fn restore_still_detects_tracked_and_untracked_domain_writes() {
    let fixture = build_fixture(Layout::Plain);
    let worktree = fixture.worktree.clone();
    let identity = lock_identity(&worktree, "lease-dom-3");
    let evidence = quiescence_evidence("win-dom-3").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-dom-3", &identity, &evidence))
        .expect("capture baseline");

    // Reviewer 域内写入:改 tracked、新建 untracked 非 ignored、域内残留。
    fs::write(worktree.join("tracked.txt"), "reviewer\n").unwrap();
    fs::write(worktree.join("evil.txt"), "evil\n").unwrap();
    fs::create_dir_all(worktree.join("build")).unwrap();
    fs::write(worktree.join("build/artifact.bin"), "junk\n").unwrap();

    let report = restore_report(
        store
            .restore_baseline("win-dom-3", &identity)
            .expect("restore"),
    );
    assert_eq!(
        fs::read_to_string(worktree.join("tracked.txt")).unwrap(),
        "base\n",
        "tracked 写入必须恢复"
    );
    assert!(
        !worktree.join("evil.txt").exists(),
        "untracked 非 ignored 写入必须清除"
    );
    assert!(
        !worktree.join("build").exists(),
        "域内构建残留必须清除(B 档语义不变)"
    );
    assert!(
        report
            .worktree_fixed_entries
            .contains(&"tracked.txt".to_string())
    );
    assert!(
        report
            .worktree_removed_extras
            .contains(&"evil.txt".to_string())
    );
    assert!(
        report
            .worktree_removed_extras
            .contains(&"build/artifact.bin".to_string())
    );
}

// ---------- 契约 Scenario:改写 .gitignore 不得缩小对账域 ----------

#[tokio::test]
async fn reviewer_gitignore_rewrite_cannot_shrink_domain() {
    let fixture = build_fixture(Layout::Plain);
    let worktree = fixture.worktree.clone();
    let identity = lock_identity(&worktree, "lease-dom-4");
    let evidence = quiescence_evidence("win-dom-4").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-dom-4", &identity, &evidence))
        .expect("capture baseline");

    // Reviewer 改写 tracked .gitignore 试图把残留藏进 ignored,并新建
    // 嵌套 .gitignore 进一步遮蔽。
    let baseline_gitignore = fs::read_to_string(worktree.join(".gitignore")).unwrap();
    fs::write(
        worktree.join(".gitignore"),
        format!("{baseline_gitignore}evil.txt\n"),
    )
    .unwrap();
    fs::create_dir_all(worktree.join("hide")).unwrap();
    fs::write(worktree.join("hide/.gitignore"), "*\n").unwrap();
    fs::write(worktree.join("hide/evil.txt"), "hidden\n").unwrap();
    fs::write(worktree.join("evil.txt"), "evil\n").unwrap();

    let report = restore_report(
        store
            .restore_baseline("win-dom-4", &identity)
            .expect("restore"),
    );
    // .gitignore 自身是域内 tracked 文件:恢复基线内容,按域内差异处理。
    assert_eq!(
        fs::read_to_string(worktree.join(".gitignore")).unwrap(),
        baseline_gitignore,
        "改写 .gitignore 必须恢复基线内容"
    );
    assert!(
        report
            .worktree_fixed_entries
            .contains(&".gitignore".to_string())
    );
    // 冻结规则(基线内容)不认 Reviewer 新增的 ignore 行/文件:域不缩水。
    assert!(
        !worktree.join("evil.txt").exists(),
        "根 evil.txt 按冻结规则仍在域内"
    );
    assert!(
        !worktree.join("hide").exists(),
        "Reviewer 新建 .gitignore 无法遮蔽域"
    );
    assert!(
        report
            .worktree_removed_extras
            .contains(&"evil.txt".to_string())
    );
    assert!(
        report
            .worktree_removed_extras
            .contains(&"hide/.gitignore".to_string())
            || report.worktree_removed_extras.contains(&"hide".to_string()),
        "遮蔽文件自身按残留清除: {:?}",
        report.worktree_removed_extras
    );
}

// ---------- 空目录/git 不可见特殊文件的处置 ----------

#[tokio::test]
async fn empty_dirs_are_captured_but_fifos_are_not_in_domain() {
    let fixture = build_fixture(Layout::Plain);
    let worktree = fixture.worktree.clone();
    // FIFO:git 不可见(ls-files 不枚举、porcelain 不显示)→ 域外。
    let fifo = worktree.join("pipe");
    unsafe {
        let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(libc::mkfifo(c.as_ptr(), 0o644), 0);
    }

    let identity = lock_identity(&worktree, "lease-dom-5");
    let evidence = quiescence_evidence("win-dom-5").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-dom-5", &identity, &evidence))
        .expect("FIFO 域外:capture 不得拒绝");
    let manifest = store.load_manifest("win-dom-5").expect("load manifest");
    let rels = rel_set(&manifest);
    assert!(!rels.contains("pipe"), "FIFO git 不可见,不入基线: {rels:?}");
    // 空目录:不入 git 口径,但保留为 Directory 条目以保 Coder 现场
    // (恢复不得误删 Reviewer 之前的空目录)。
    assert!(
        manifest.entries.iter().any(
            |entry| entry.rel_path == "empty_dir" && entry.kind == BaselineEntryKind::Directory
        ),
        "空目录须保留为 Directory 条目: {rels:?}"
    );
    drop(manifest);
    fs::remove_file(&fifo).unwrap();
}
