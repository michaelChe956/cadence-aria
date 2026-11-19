// Synthetic repos only: do not infer the contents of any customer member.
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct AdvancingCoverage(Arc<AtomicU64>);
impl AggregateCoverageProbe for AdvancingCoverage {
    fn coverage(
        &self,
        root: &Path,
        names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        self.0.fetch_add(3_100, Ordering::SeqCst);
        ScriptedCoverage {
            member_covered: names
                .iter()
                .map(|name| (name.clone(), !name.starts_with("synthetic_uncovered_")))
                .collect(),
        }
        .coverage(root, names)
    }
}

#[derive(Clone, Copy)]
enum VisibleCase {
    ConfigOnly,
    Expires,
    Mixed,
}
struct VisibilityEvidence {
    clock: Arc<AtomicU64>,
    calls: AtomicUsize,
    case: VisibleCase,
}
impl MemberSourceVisibility for VisibilityEvidence {
    fn visible_files(
        &self,
        root: &Path,
        _: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match (self.case, call) {
            (VisibleCase::Expires, 0) => {
                self.clock.fetch_add(2_200, Ordering::SeqCst);
            }
            (VisibleCase::Mixed, 0) => {
                return Err(AggregateIndexError::Failed {
                    code: "aggregate_index_source_scan_io",
                    message: "fixture read failure".into(),
                });
            }
            (VisibleCase::Mixed, 1) => {
                std::fs::write(root.join("module.rs"), "fn feature() {}\n").unwrap();
                return Ok(vec!["module.rs".into()]);
            }
            _ => {}
        }
        Ok(vec!["policy.json".into()])
    }
}

fn synthetic_sixty_nine() -> (SourceScopeFixture, LogicalCodebaseManifest) {
    let fixture = SourceScopeFixture::new();
    let mut manifest = fixture
        .logical
        .load_manifest("project_0001")
        .unwrap()
        .unwrap();
    for i in 0..67 {
        let name = if i < 62 {
            format!("synthetic_covered_{i}")
        } else {
            format!("synthetic_uncovered_{i}")
        };
        let dir = fixture.root.join(&name);
        std::fs::create_dir(&dir).unwrap();
        if i < 62 {
            std::fs::write(dir.join("module.rs"), "fn feature() {}\n").unwrap();
        } else {
            std::fs::write(dir.join("policy.json"), "{}\n").unwrap();
        }
        let (member, checkout) = member_with_checkout(&fixture.root, &name, "c".repeat(40));
        fixture
            .logical
            .save_member("project_0001", &member)
            .unwrap();
        fixture
            .logical
            .save_checkout("project_0001", &checkout)
            .unwrap();
        manifest.member_ids.push(member.logical_repository_id);
    }
    manifest.membership_revision += 1;
    fixture
        .logical
        .save_manifest("project_0001", &manifest)
        .unwrap();
    fixture.allow_all_exclusion_queries();
    (fixture, manifest)
}

fn exercise_coverage_scan(
    fixture: &SourceScopeFixture,
    manifest: &LogicalCodebaseManifest,
    case: VisibleCase,
) -> (Result<AggregateIndexRecord, AggregateIndexError>, usize) {
    let elapsed = Arc::new(AtomicU64::new(0));
    let epoch = Instant::now();
    let clock = || epoch + Duration::from_millis(elapsed.load(Ordering::SeqCst));
    let visibility = Arc::new(VisibilityEvidence {
        clock: elapsed.clone(),
        calls: AtomicUsize::new(0),
        case,
    });
    let result = fixture
        .operation()
        .with_coverage_probe(Arc::new(AdvancingCoverage(elapsed.clone())))
        .with_source_visibility(visibility.clone())
        .apply_index_with_clock(
            "project_0001",
            manifest,
            IndexApplicationMode::Rebuild,
            None,
            &clock,
        );
    (result, visibility.calls.load(Ordering::SeqCst))
}

#[test]
fn slow_coverage_does_not_consume_scan_deadline_and_real_scan_timeout_is_not_skipped() {
    use super::super::scan_diagnostics::ScanUnknownReason;
    let (fixture, manifest) = synthetic_sixty_nine();
    let (ok, calls) = exercise_coverage_scan(&fixture, &manifest, VisibleCase::ConfigOnly);
    let active = ok.expect("a slow coverage phase must not starve the five subsequent scans");
    assert_eq!(active.status, AggregateIndexStatus::Active);
    assert_eq!(calls, 5);
    assert_eq!(
        active
            .member_snapshots
            .iter()
            .filter(|m| m.skip_reason.is_some())
            .count(),
        5
    );
    let diag = active.source_scan.as_ref().unwrap();
    assert_eq!(
        (
            diag.coverage_elapsed_ms,
            diag.scan_elapsed_ms,
            diag.member_count,
            diag.covered_count
        ),
        (3100, 0, 69, 64)
    );
    assert!(diag.members.iter().all(|m| m.verdict
        == super::super::source_scan::MemberSourceVerdict::NoSource
        && m.visibility_started));
    let (failed, calls) = exercise_coverage_scan(&fixture, &manifest, VisibleCase::Expires);
    assert!(matches!(
        failed,
        Err(AggregateIndexError::Failed {
            code: "aggregate_index_source_scan_incomplete",
            ..
        })
    ));
    assert_eq!(calls, 1);
    let records = fixture.records();
    let attempt = records
        .iter()
        .find(|r| r.status == AggregateIndexStatus::Stale)
        .unwrap();
    assert!(
        attempt
            .member_snapshots
            .iter()
            .all(|m| m.skip_reason.is_none())
    );
    assert!(
        attempt
            .warning
            .as_deref()
            .unwrap()
            .contains("source scan limit reached;")
    );
    let diag = attempt.source_scan.as_ref().unwrap();
    assert_eq!(
        (
            diag.coverage_elapsed_ms,
            diag.scan_elapsed_ms,
            diag.covered_count
        ),
        (3100, 2200, 64)
    );
    assert_eq!(diag.members.len(), 5);
    assert!(
        diag.members
            .iter()
            .all(|m| m.reason == Some(ScanUnknownReason::Deadline))
    );
    assert!(diag.members[0].visibility_started);
    assert_eq!(diag.members[0].remaining_ms_at_entry, Some(2000));
    assert!(
        diag.members[1..]
            .iter()
            .all(|m| !m.visibility_started && m.remaining_ms_at_entry == Some(0))
    );
    let lkg = records
        .iter()
        .find(|r| r.aggregate_index_id == active.aggregate_index_id)
        .unwrap();
    assert_eq!(lkg.status, AggregateIndexStatus::Degraded);
    assert_eq!(
        lkg.source_scan, active.source_scan,
        "a failure must not replace evidence of a different attempt"
    );
}

#[test]
fn mixed_disk_source_and_unknown_keep_evidence_and_do_not_publish() {
    use super::super::source_scan::MemberSourceVerdict;
    let (fixture, manifest) = synthetic_sixty_nine();
    let (result, calls) = exercise_coverage_scan(&fixture, &manifest, VisibleCase::Mixed);
    assert!(matches!(
        result,
        Err(AggregateIndexError::Failed {
            code: "aggregate_index_member_coverage_failed",
            ..
        })
    ));
    assert_eq!(calls, 5);
    let failed = fixture.records().pop().unwrap();
    assert_eq!(failed.status, AggregateIndexStatus::Stale);
    let rows = &failed.source_scan.as_ref().unwrap().members;
    assert_eq!(rows[0].verdict, MemberSourceVerdict::Unknown);
    assert_eq!(rows[1].verdict, MemberSourceVerdict::SourceInNormalPath);
    assert!(
        rows[2..]
            .iter()
            .all(|r| r.verdict == MemberSourceVerdict::NoSource)
    );
    assert!(
        failed
            .member_snapshots
            .iter()
            .all(|m| m.skip_reason.is_none())
    );
}
