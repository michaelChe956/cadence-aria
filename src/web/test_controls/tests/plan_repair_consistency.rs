use std::collections::BTreeSet;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::super::{PlanRepairFixtureControl, PlanRepairFixtureRuntime};
use crate::product::coding_workspace_engine::register_plan_repair_start_snapshot_request_pause;

const FIRST_FINDING_ID: &str = "code_review_report_0001_finding_0001";
const SECOND_FINDING_ID: &str = "code_review_report_0001_finding_0002";

#[test]
fn plan_repair_start_reads_link_snapshot_and_request_from_one_arbitrated_view() {
    let root = tempfile::tempdir().expect("temp dir");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("fixture runtime")
        .block_on(PlanRepairFixtureRuntime::seed(
            root.path(),
            PlanRepairFixtureControl::default(),
        ))
        .expect("seed plan repair fixture");
    let pause = register_plan_repair_start_snapshot_request_pause(
        root.path().join(".aria"),
        FIRST_FINDING_ID,
    );

    let first = spawn_finding(runtime.clone(), FIRST_FINDING_ID);
    assert!(
        pause.wait_until_reached(Duration::from_secs(3)),
        "first request did not reach the snapshot/request boundary"
    );

    let second = spawn_finding(runtime.clone(), SECOND_FINDING_ID);
    assert!(
        second
            .started_rx
            .recv_timeout(Duration::from_secs(3))
            .is_ok(),
        "second request did not start while first request was paused"
    );
    let second_before_release = second.result_rx.recv_timeout(Duration::from_millis(500));
    match second_before_release {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            panic!("second request disconnected before the snapshot/request boundary release")
        }
        Ok(second_result) => {
            pause.release();
            let first_result = first
                .result_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("first request did not finish after premature second completion");
            first.join.join().expect("first request thread");
            second.join.join().expect("second request thread");
            panic!(
                "second request completed before the snapshot/request boundary was released: \
                 first={first_result:?}, second={second_result:?}"
            );
        }
    }
    pause.release();
    let first_result = first
        .result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first request did not finish after snapshot/request pause release");
    let second_result = second
        .result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second request did not finish after snapshot/request pause release");
    first.join.join().expect("first request thread");
    second.join.join().expect("second request thread");

    assert!(
        first_result.is_ok(),
        "first request failed: {first_result:?}"
    );
    assert!(
        second_result.is_ok(),
        "second request failed: {second_result:?}"
    );

    let request = runtime
        .authoritative_plan_repair_request()
        .expect("one authoritative repair request");
    let identity = runtime
        .plan_repair_identity()
        .expect("unique request, amendment, and child identity");
    let evidence_source_refs = request
        .evidence
        .iter()
        .map(|evidence| evidence.source_ref.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(request.evidence.len(), 2);
    assert_eq!(
        evidence_source_refs,
        BTreeSet::from([
            format!("code_review_report_0001#{FIRST_FINDING_ID}"),
            format!("code_review_report_0001#{SECOND_FINDING_ID}"),
        ])
    );
    assert_eq!(
        runtime.plan_repair_request_count().expect("request count"),
        1
    );
    assert_eq!(request.id, identity.request_id);
    assert_eq!(
        request.amendment_id.as_deref(),
        Some(identity.amendment_id.as_str())
    );
    assert_eq!(
        identity.child_session_id,
        format!("workspace_session_{}", identity.amendment_id)
    );
}

struct FindingThread {
    join: thread::JoinHandle<()>,
    started_rx: mpsc::Receiver<()>,
    result_rx: mpsc::Receiver<Result<(), String>>,
}

fn spawn_finding(runtime: PlanRepairFixtureRuntime, finding_id: &'static str) -> FindingThread {
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let join = thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let _ = started_tx.send(());
                worker
                    .block_on(runtime.replay_plan_defect_finding(finding_id))
                    .map_err(|error| error.to_string())
            });
        let _ = result_tx.send(result);
    });
    FindingThread {
        join,
        started_rx,
        result_rx,
    }
}
