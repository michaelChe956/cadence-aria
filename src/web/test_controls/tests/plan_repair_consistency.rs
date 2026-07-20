use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::super::{PlanRepairFixtureControl, PlanRepairFixtureRuntime};
use crate::product::coding_workspace_engine::register_plan_repair_start_consistency_pause;

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
    let pause =
        register_plan_repair_start_consistency_pause(root.path().join(".aria"), FIRST_FINDING_ID);

    let first = spawn_finding(runtime.clone(), FIRST_FINDING_ID);
    assert!(
        pause.wait_until_reached(Duration::from_secs(3)),
        "first request did not reach the consistency read boundary"
    );

    let second = spawn_finding(runtime, SECOND_FINDING_ID);
    let second_result = second
        .result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second request did not finish while first request was paused");
    pause.release();
    let first_result = first
        .result_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first request did not finish after consistency pause release");
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
}

struct FindingThread {
    join: thread::JoinHandle<()>,
    result_rx: mpsc::Receiver<Result<(), String>>,
}

fn spawn_finding(runtime: PlanRepairFixtureRuntime, finding_id: &'static str) -> FindingThread {
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let join = thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                worker
                    .block_on(runtime.replay_plan_defect_finding(finding_id))
                    .map_err(|error| error.to_string())
            });
        let _ = result_tx.send(result);
    });
    FindingThread { join, result_rx }
}
