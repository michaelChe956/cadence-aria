// ---------- 红测⑧:游标持久化写放大有界(不随基线条目数增长) ----------

/// 在 worktree 里铺 N 个普通文件(捕获为基线条目;恢复时每条都走
/// 「比对 + 跳过」路径)。
fn seed_bulk_files(worktree: &Path, count: usize) {
    let dir = worktree.join("bulk");
    fs::create_dir_all(&dir).expect("bulk dir");
    for index in 0..count {
        fs::write(
            dir.join(format!("f{index:05}.txt")),
            format!("bulk-{index}\n"),
        )
        .expect("bulk file");
    }
}

fn cursor_sidecar_path(store: &ReviewerSnapshotStore, window_id: &str) -> PathBuf {
    store
        .window_dir(window_id)
        .join(crate::product::reviewer_snapshot_cursor::CURSOR_SIDECAR_FILE)
}

#[tokio::test]
async fn restore_cursor_persistence_write_amplification_is_bounded() {
    let bulk = 2000usize;
    let fixture = build_fixture(Layout::Plain);
    seed_bulk_files(&fixture.worktree, bulk);
    let identity = lock_identity(&fixture.worktree, "lease-wa");
    let evidence = quiescence_evidence("win-wa").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-wa", &identity, &evidence))
        .expect("capture");
    let manifest = store.load_manifest("win-wa").expect("manifest");
    let entry_count = manifest.entries.len();
    assert!(entry_count >= bulk, "基线规模应含 bulk 条目: {entry_count}");
    let manifest_len = fs::metadata(store.manifest_path("win-wa"))
        .expect("manifest stat")
        .len();
    assert!(
        manifest_len > 100_000,
        "大快照 manifest 应为 MB 级文档: {manifest_len}"
    );

    // 只制造少量差异:其余条目走「比对 → 跳过」
    let targets: Vec<BaselineEntry> = manifest
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                BaselineEntryKind::File | BaselineEntryKind::Gitfile
            ) && entry.rel_path != ".git"
        })
        .take(3)
        .cloned()
        .collect();
    assert_eq!(targets.len(), 3);
    for entry in &targets {
        fs::write(
            fixture.worktree.join(&entry.rel_path),
            b"reviewer-mangled\n",
        )
        .expect("mangle");
    }

    // 用新实例计量(计数器随实例归零)
    let restoring = ReviewerSnapshotStore::new(&fixture.store_root);
    let report = restore_report(
        restoring
            .restore_baseline("win-wa", &identity)
            .expect("restore"),
    );
    assert_eq!(
        report.worktree_fixed_entries.len(),
        3,
        "只恢复差异条目: {:?}",
        report.worktree_fixed_entries
    );

    let (manifest_rewrites, sidecar_writes) = restoring.restore_cursor_write_stats();
    let sidecar_len = fs::metadata(cursor_sidecar_path(&restoring, "win-wa"))
        .map(|meta| meta.len())
        .unwrap_or(0);
    let written =
        manifest_rewrites as u64 * manifest_len + sidecar_writes as u64 * sidecar_len.max(1);
    eprintln!(
        "write-amplification: entries={entry_count} manifest_len={manifest_len} \
             manifest_rewrites={manifest_rewrites} sidecar_writes={sidecar_writes} \
             sidecar_len={sidecar_len} total_written={written}"
    );
    // 整份 manifest 的重写次数 = 阶段边界数(常数),与条目数无关。
    // 红态(逐条重写整份 manifest)= entry_count + 1,随 N 线性增长。
    assert!(
        manifest_rewrites <= 8,
        "manifest 整体重写次数必须为阶段数常数,实测 {manifest_rewrites}(entries={entry_count})"
    );
    // 游标持久化总写入字节 < 基线 manifest 大小的常数倍(旧实现 ≈ N × manifest_len)
    assert!(
        written <= 20 * manifest_len,
        "游标持久化总写入字节超界: {written} vs 20×{manifest_len}"
    );
    // 增量写确实发生了(条目级进度落 sidecar,而非完全不持久化)
    assert!(
        sidecar_writes >= entry_count,
        "每条推进都必须落 sidecar: {sidecar_writes} < {entry_count}"
    );
}

// ---------- 红测⑨:条目级进度在 sidecar,崩溃后从中途续跑 ----------

#[tokio::test]
async fn restore_resumes_from_sidecar_cursor_after_crash() {
    let fixture = build_fixture(Layout::Plain);
    seed_bulk_files(&fixture.worktree, 64);
    let identity = lock_identity(&fixture.worktree, "lease-crash");
    let evidence = quiescence_evidence("win-crash").await;
    let store = ReviewerSnapshotStore::new(&fixture.store_root);
    store
        .capture_baseline(capture_request(&fixture, "win-crash", &identity, &evidence))
        .expect("capture");
    let manifest = store.load_manifest("win-crash").expect("manifest");

    // 每条文件条目都漂移:每条都必须恢复 → 可用「剩余差异数」证明
    // 续跑没有从头重做。
    let mutable: Vec<BaselineEntry> = manifest
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                BaselineEntryKind::File | BaselineEntryKind::Gitfile
            ) && entry.rel_path != ".git"
        })
        .cloned()
        .collect();
    assert!(mutable.len() > 50, "fixture 不足: {}", mutable.len());
    for entry in &mutable {
        fs::write(
            fixture.worktree.join(&entry.rel_path),
            b"reviewer-mangled-bulk\n",
        )
        .expect("mangle");
    }

    // 中断:第 40 次游标持久化后注入故障(仍在中途,未到阶段边界)
    let faulted = ReviewerSnapshotStore::new(&fixture.store_root);
    faulted.set_restore_failpoint_after_cursor_writes(40);
    let err = faulted
        .restore_baseline("win-crash", &identity)
        .expect_err("必须注入中断");
    assert!(
        matches!(&err, ReviewerSnapshotError::Io(detail)
                if detail.contains("reviewer_snapshot_injected_failure")),
        "{err:?}"
    );

    // ① 条目级进度必须已落 sidecar(崩溃续跑的载体)
    let sidecar_file = cursor_sidecar_path(&faulted, "win-crash");
    assert!(
        sidecar_file.is_file(),
        "条目级游标必须增量落盘 sidecar: {} 缺失",
        sidecar_file.display()
    );
    let sidecar: crate::product::reviewer_snapshot_cursor::RestoreCursorSidecar =
        read_json(&sidecar_file).expect("read sidecar");
    assert_eq!(
        sidecar.schema,
        crate::product::reviewer_snapshot_cursor::CURSOR_SIDECAR_SCHEMA
    );
    assert_eq!(sidecar.window_id, "win-crash");
    assert_eq!(sidecar.cursor.phase, RestorePhase::WorktreeRestore);
    assert!(
        sidecar.cursor.entry_index > 1,
        "sidecar 必须携带推进后的游标: {:?}",
        sidecar.cursor
    );

    // ② 整份 manifest 仍停在阶段边界 —— 续跑位置只能来自 sidecar
    let on_disk: ReviewerBaselineManifest =
        read_json(&faulted.manifest_path("win-crash")).expect("raw manifest");
    assert_eq!(on_disk.restore_cursor.phase, RestorePhase::WorktreeRestore);
    assert_eq!(
        on_disk.restore_cursor.entry_index, 0,
        "整份 manifest 只在阶段边界落盘;条目级进度在 sidecar"
    );

    // 故障时刻还剩多少条目需要恢复
    let mut remaining = 0usize;
    for entry in &mutable {
        let bytes = fs::read(fixture.worktree.join(&entry.rel_path)).expect("read");
        if Some(sha256_hex(&bytes)) != entry.content_sha256 {
            remaining += 1;
        }
    }
    assert!(
        remaining > 0 && remaining < mutable.len(),
        "中断必须发生在条目中途: remaining={remaining} 总={}",
        mutable.len()
    );

    // ③ 重启:从 sidecar 游标续接,只修剩余差异(不从头重做)
    let resumed_store = ReviewerSnapshotStore::new(&fixture.store_root);
    let report = restore_report(
        resumed_store
            .restore_baseline("win-crash", &identity)
            .expect("resume"),
    );
    assert_eq!(
        report.resumed_from, sidecar.cursor,
        "续跑必须从 sidecar 游标接续(而非阶段边界)"
    );
    assert_eq!(
        report.worktree_fixed_entries.len(),
        remaining,
        "续跑只恢复剩余差异,不重做已完成条目"
    );
    assert_dirty_matrix_restored(&fixture);
    let final_manifest = resumed_store.load_manifest("win-crash").expect("final");
    assert_eq!(final_manifest.restore_cursor.phase, RestorePhase::Completed);
}
