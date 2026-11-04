//! 票3「一键核验全部」编排层单测（T6.1~T6.8）。
//!
//! 覆盖：立即返回 job_id（零探针等待）/ 四家并发 / 单家失败不阻断 /
//! 全行已验证秒回 / CLI 缺失 skipped+稳定原因 / 终态未完成标超时 /
//! durable 跨读取 / 绝不自动触发。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::*;
use crate::product::logical_codebase::SessionPolicyAction;

const PROJECT_ID: &str = "project_0001";
/// 作业存盘的路径 scope：用项目默认作用域的既有 helper（`lc_scope_root` 判定
/// 落回项目级根的那一支），不手写字面量、不涉 manifest/selection 解析。
fn lc_id() -> String {
    crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID)
}

fn paths_fixture() -> (ProductAppPaths, TempDir) {
    let root = tempfile::tempdir().expect("temp root");
    (ProductAppPaths::new(root.path().join(".aria")), root)
}

fn revalidated() -> Result<RevalidateOutcome, RevalidateError> {
    Ok(RevalidateOutcome::Revalidated {
        version: "1.0.0".to_string(),
        imported_actions: vec![SessionPolicyAction::PlanningReadOnly],
    })
}

async fn wait_terminal(
    service: &CapabilityRevalidateBatchService,
    job_id: &str,
) -> CapabilityRevalidateBatchJob {
    for _ in 0..400 {
        let job = service
            .get(PROJECT_ID, &lc_id(), job_id)
            .expect("job 必须 durable 可读");
        if job.is_terminal() {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("作业未在 4s 内到终态：{job_id}");
}

// T6.1
#[tokio::test]
async fn start_returns_job_id_without_waiting_for_probe() {
    let (paths, _root) = paths_fixture();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let gate_for_call = gate.clone();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, _provider| {
        let gate = gate_for_call.clone();
        Box::pin(async move {
            let _permit = gate.acquire().await.expect("gate never closed");
            Ok(RevalidateOutcome::AlreadyConfirmed {
                version: "1.0.0".to_string(),
            })
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths).with_revalidate_call(call);

    let started = Instant::now();
    let job = service.start(PROJECT_ID, &lc_id()).expect("start");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "发起必须立即返回（零探针等待），实际耗时 {elapsed:?}"
    );
    assert_eq!(job.status, BatchJobStatus::Running);
    assert!(
        job.job_id.starts_with("capability_batch_"),
        "job_id: {}",
        job.job_id
    );
    assert_eq!(job.results.len(), 4, "四家逐项结果");
    for item in &job.results {
        assert_eq!(item.status, BatchItemStatus::Running, "{item:?}");
        assert!(item.reason.is_none());
    }

    // 探针仍被闸门挡住 ⇒ 作业仍在跑（证明发起未等探针）。
    let observed = service.get(PROJECT_ID, &lc_id(), &job.job_id).expect("job");
    assert_eq!(observed.status, BatchJobStatus::Running);

    gate.add_permits(8);
    let done = wait_terminal(&service, &job.job_id).await;
    assert_eq!(done.status, BatchJobStatus::Completed);
}

// T6.2
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_providers_run_concurrently() {
    let (paths, _root) = paths_fixture();
    let spans: Arc<Mutex<Vec<(ProviderName, Instant, Instant)>>> = Arc::new(Mutex::new(Vec::new()));
    let spans_for_call = spans.clone();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, provider| {
        let spans = spans_for_call.clone();
        Box::pin(async move {
            let start = Instant::now();
            tokio::time::sleep(Duration::from_millis(150)).await;
            let end = Instant::now();
            spans.lock().expect("spans").push((provider, start, end));
            revalidated()
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths).with_revalidate_call(call);

    let job = service.start(PROJECT_ID, &lc_id()).expect("start");
    let done = wait_terminal(&service, &job.job_id).await;
    assert_eq!(done.status, BatchJobStatus::Completed);

    let spans = spans.lock().expect("spans").clone();
    assert_eq!(spans.len(), 4, "四家各跑一次");
    let latest_start = spans
        .iter()
        .map(|(_, start, _)| *start)
        .max()
        .expect("latest start");
    let earliest_end = spans
        .iter()
        .map(|(_, _, end)| *end)
        .min()
        .expect("earliest end");
    assert!(
        latest_start < earliest_end,
        "四家探针窗口必须重叠（并发，非串行）；spans: {spans:?}"
    );
}

// T6.3
#[tokio::test]
async fn single_provider_failure_does_not_block_others() {
    let (paths, _root) = paths_fixture();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, provider| {
        Box::pin(async move {
            if provider == ProviderName::Codex {
                Err(RevalidateError::ProbeFailed {
                    action: Some(SessionPolicyAction::PlanningReadOnly),
                    detail: "boom: probe exploded".to_string(),
                })
            } else {
                revalidated()
            }
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths).with_revalidate_call(call);

    let job = service.start(PROJECT_ID, &lc_id()).expect("start");
    let done = wait_terminal(&service, &job.job_id).await;

    assert_eq!(
        done.status,
        BatchJobStatus::Completed,
        "单家失败不得让整批失败"
    );
    let codex = done.item(ProviderName::Codex).expect("codex 项");
    assert_eq!(codex.status, BatchItemStatus::Failed);
    assert_eq!(codex.reason.as_deref(), Some(BATCH_REASON_PROBE_FAILED));
    assert!(
        codex.detail.as_deref().unwrap_or_default().contains("boom"),
        "detail 须携带原始错误：{codex:?}"
    );
    for other in [
        ProviderName::ClaudeCode,
        ProviderName::Pi,
        ProviderName::KimiCode,
    ] {
        let item = done.item(other.clone()).expect("item");
        assert_eq!(
            item.status,
            BatchItemStatus::Confirmed,
            "{other:?}: {item:?}"
        );
        assert_eq!(item.reason.as_deref(), Some(BATCH_REASON_REVALIDATED));
    }
}

// T6.4
#[tokio::test]
async fn already_confirmed_provider_returns_immediately() {
    let (paths, _root) = paths_fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_call = calls.clone();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, _provider| {
        let calls = calls_for_call.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(RevalidateOutcome::AlreadyConfirmed {
                version: "9.9.9".to_string(),
            })
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths).with_revalidate_call(call);

    let job = service.start(PROJECT_ID, &lc_id()).expect("start");
    let done = wait_terminal(&service, &job.job_id).await;

    assert_eq!(done.status, BatchJobStatus::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 4, "每家各调用一次单家入口");
    for item in &done.results {
        assert_eq!(item.status, BatchItemStatus::Confirmed, "{item:?}");
        assert_eq!(
            item.reason.as_deref(),
            Some(BATCH_REASON_ALREADY_CONFIRMED),
            "秒回不重跑"
        );
        assert_eq!(item.version.as_deref(), Some("9.9.9"));
    }
}

// T6.5
#[tokio::test]
async fn missing_cli_is_skipped_with_stable_reason() {
    let (paths, _root) = paths_fixture();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, provider| {
        Box::pin(async move {
            if provider == ProviderName::Pi {
                Err(RevalidateError::ProbeFailed {
                    action: None,
                    detail: "boundary_unavailable: cannot execute `pi --version`: \
                             No such file or directory (os error 2)"
                        .to_string(),
                })
            } else {
                revalidated()
            }
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths).with_revalidate_call(call);

    let job = service.start(PROJECT_ID, &lc_id()).expect("start");
    let done = wait_terminal(&service, &job.job_id).await;

    assert_eq!(done.status, BatchJobStatus::Completed, "不整体失败");
    let pi = done.item(ProviderName::Pi).expect("pi 项");
    assert_eq!(pi.status, BatchItemStatus::Skipped, "不得挂起");
    assert_eq!(
        pi.reason.as_deref(),
        Some(BATCH_REASON_CLI_NOT_INSTALLED),
        "稳定原因码"
    );
    assert!(
        pi.detail
            .as_deref()
            .unwrap_or_default()
            .contains("pi --version"),
        "detail: {pi:?}"
    );
    for other in [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::KimiCode,
    ] {
        assert_eq!(
            done.item(other.clone()).expect("item").status,
            BatchItemStatus::Confirmed,
            "{other:?}"
        );
    }
}

// T6.6
#[tokio::test]
async fn unfinished_provider_at_terminal_is_marked_timeout() {
    let (paths, _root) = paths_fixture();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, provider| {
        Box::pin(async move {
            if provider == ProviderName::KimiCode {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            revalidated()
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths)
        .with_revalidate_call(call)
        .with_job_deadline(Duration::from_millis(800));

    let job = service.start(PROJECT_ID, &lc_id()).expect("start");
    let done = wait_terminal(&service, &job.job_id).await;

    assert_eq!(done.status, BatchJobStatus::Completed);
    let kimi = done.item(ProviderName::KimiCode).expect("kimi 项");
    assert_eq!(kimi.status, BatchItemStatus::Timeout, "未完成项标超时");
    assert_eq!(kimi.reason.as_deref(), Some(BATCH_REASON_DEADLINE_EXCEEDED));
    assert!(kimi.version.is_none(), "不得谎报已确认");
    for other in [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::Pi,
    ] {
        assert_eq!(
            done.item(other.clone()).expect("item").status,
            BatchItemStatus::Confirmed,
            "{other:?} 的已得结果不得被超时覆盖"
        );
    }
}

// T6.7
#[tokio::test]
async fn job_state_is_durable_across_reads() {
    let (paths, _root) = paths_fixture();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let gate_for_call = gate.clone();
    let call: BatchRevalidateCall = Arc::new(move |_project, _lc, _provider| {
        let gate = gate_for_call.clone();
        Box::pin(async move {
            let _permit = gate.acquire().await.expect("gate never closed");
            revalidated()
        })
    });
    let service = CapabilityRevalidateBatchService::new(paths.clone()).with_revalidate_call(call);
    let job = service.start(PROJECT_ID, &lc_id()).expect("start");

    // 另一实例按 job_id 读：进度不丢（刷新即续）。
    let reader = CapabilityRevalidateBatchService::new(paths.clone());
    let mid = reader
        .get(PROJECT_ID, &lc_id(), &job.job_id)
        .expect("mid read");
    assert_eq!(mid.job_id, job.job_id);
    assert_eq!(mid.status, BatchJobStatus::Running);
    assert_eq!(mid.results.len(), 4);

    gate.add_permits(8);
    let done = wait_terminal(&reader, &job.job_id).await;
    assert_eq!(done.status, BatchJobStatus::Completed);

    // 终态再读：不得返回「作业不存在」。
    let terminal = reader
        .get(PROJECT_ID, &lc_id(), &job.job_id)
        .expect("终态仍可读");
    assert_eq!(terminal.status, BatchJobStatus::Completed);
    assert!(terminal.finished_at.is_some());

    // 未知 job_id：明确的 NotFound（非静默空作业）。
    match reader.get(PROJECT_ID, &lc_id(), "capability_batch_missing") {
        Err(ProductStoreError::NotFound { .. }) => {}
        other => panic!("期望 NotFound，实际 {other:?}"),
    }
}

// T6.8
#[test]
fn batch_is_never_auto_triggered() {
    // ① 构造服务不产生任何作业（只有显式 start 才创建）。
    let (paths, _root) = paths_fixture();
    let _service = CapabilityRevalidateBatchService::new(paths.clone());
    let jobs_dir = batch_jobs_dir(&paths, PROJECT_ID, &lc_id()).expect("jobs dir");
    assert!(
        !jobs_dir.exists(),
        "无显式点击不得创建作业目录：{}",
        jobs_dir.display()
    );

    // ② 聚合初始化 / 冷启动链不得引用批量核验（源码级回归锁）。
    for source in [
        include_str!("aggregate_initialization.rs"),
        include_str!("aggregate_initialization_coordinator.rs"),
        include_str!("bootstrap.rs"),
        include_str!("provider_capability_revalidate.rs"),
    ] {
        assert!(
            !source.contains("capability_revalidate_batch"),
            "REQ-PCR-03：批量核验只由显式点击发起，不得被自动触发"
        );
    }
}
