// 知识库写入面审计的落盘记录与平台索引运行期隔离（按仓内 `*.inc.rs`
// include 惯例从 `coordinator.rs` 拆出；修复轮 2）。
//
// 两块内容：
// 1. 错误/取消记录映射（`error_record` 家族）：把执行层错误映射为持久化
//    失败记录，越界事实与审计可信度并入其中；
// 2. 平台聚合索引隔离（controller 裁决 (b)）：`AggregateIndexOperation`
//    的 init/sync 全程独占持有 `.aggregate-index.lock`（生产链路经 `for_lc`
//    重定 scope 到 LC 子树，见 `src/web/handlers/aggregate_index.rs`）。
//    知识库 turn 的前后快照期间同样独占持有这把锁，使 turn 窗口内不可能
//    有平台索引写入聚合根：`codegraph.json`、`.codegraph/**` 及其原子写
//    临时文件因此无需按路径豁免——出现在差分里就只能来自 provider，照常
//    判越界。等锁在专用线程上进行，不占用 async executor；guard drop 即释放。

// （本文件经 include! 与 coordinator.rs 处于同一模块作用域，直接使用其
//  use 导入与常量。）
const REASON_WRITE_SCOPE_VIOLATION: &str = "knowledge_base_write_scope_violation";
const REASON_TURN_FAILED: &str = "knowledge_base_provider_turn_failed";
const REASON_INVALID_INPUT: &str = "knowledge_base_invalid_input";
const REASON_INTERNAL: &str = "knowledge_base_internal_error";
const REASON_TRUST_WAITING: &str = "aggregate_initialization_trust_waiting";
const FAILURE_STAGE: &str = "knowledge_base";

// ───────────────────── 错误 / 取消记录映射 ─────────────────────

const VIOLATION_ACTION: &str = "知识库只允许写 cadence/knowledge-base、cadence/project-rules 与根 CLAUDE.md/AGENTS.md；检查越界改动后重新发起";
/// 起草 operation 的越界 remediation：允许面只有 user-input 目录（skill 契约）。
const USER_INPUT_VIOLATION_ACTION: &str =
    "起草只允许写 cadence/knowledge-base/user-input；检查越界改动后重新发起";

/// 越界 remediation 文案按 operation kind 取各自允许面。
fn violation_action(kind: KnowledgeBaseOperationKind) -> &'static str {
    match kind {
        KnowledgeBaseOperationKind::UserInput => USER_INPUT_VIOLATION_ACTION,
        KnowledgeBaseOperationKind::Bootstrap | KnowledgeBaseOperationKind::Update => {
            VIOLATION_ACTION
        }
    }
}

fn describe_violations(paths: &[String]) -> String {
    let mut listed = paths
        .iter()
        .take(VIOLATION_SUMMARY_LIMIT)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > VIOLATION_SUMMARY_LIMIT {
        listed.push_str(&format!(" 等共 {} 处", paths.len()));
    }
    listed
}

/// 把执行层错误映射为持久化失败记录；`violations` 非空时把越界事实并入：
/// 顶层 `reason_code` 取越界码且不可重试（工作树已被污染，不能盲目重试），
/// 原始失败原因保存在 `cause` 与 `stderr_summary`，不被掩盖。
fn error_record(
    error: &KnowledgeBaseError,
    violations: &[String],
    provider: &str,
    kind: KnowledgeBaseOperationKind,
) -> AggregateInitializationErrorRecord {
    let base = base_error_record(error, provider, kind);
    if violations.is_empty() {
        return base;
    }
    let original_cause = base
        .cause
        .as_deref()
        .map(|cause| format!("（{cause}）"))
        .unwrap_or_default();
    let original = format!(
        "原始失败 {}{original_cause}: {}",
        base.reason_code,
        base.stderr_summary.as_deref().unwrap_or_default()
    );
    AggregateInitializationErrorRecord {
        reason_code: REASON_WRITE_SCOPE_VIOLATION.to_string(),
        stderr_summary: Some(format!(
            "越界写入 {}；{original}",
            describe_violations(violations)
        )),
        retryable: false,
        action: violation_action(kind).to_string(),
        cause: Some(base.reason_code),
        ..base
    }
}

fn base_error_record(
    error: &KnowledgeBaseError,
    provider: &str,
    kind: KnowledgeBaseOperationKind,
) -> AggregateInitializationErrorRecord {
    let record = |reason_code: &str,
                  summary: Option<String>,
                  retryable: bool,
                  action: &str,
                  cause: Option<String>| AggregateInitializationErrorRecord {
        stage: FAILURE_STAGE.to_string(),
        reason_code: reason_code.to_string(),
        stderr_summary: summary,
        retryable,
        action: action.to_string(),
        cause,
        provider: Some(provider.to_string()),
    };
    match error {
        KnowledgeBaseError::TrustWaiting(waiting) => AggregateInitializationErrorRecord {
            stage: FAILURE_STAGE.to_string(),
            reason_code: REASON_TRUST_WAITING.to_string(),
            stderr_summary: Some(waiting.message.clone()),
            retryable: true,
            action: waiting.retry_action.clone(),
            cause: Some(waiting.reason_code.clone()),
            provider: Some(provider_wire_label(&waiting.provider).to_string()),
        },
        KnowledgeBaseError::WriteScopeViolation { paths } => record(
            REASON_WRITE_SCOPE_VIOLATION,
            Some(describe_violations(paths)),
            false,
            violation_action(kind),
            None,
        ),
        KnowledgeBaseError::TurnFailed {
            reason_code,
            summary,
            retryable,
        } => record(
            REASON_TURN_FAILED,
            Some(summary.clone()),
            *retryable,
            "查看 provider 输出摘要，处理后重新发起",
            Some(reason_code.clone()),
        ),
        KnowledgeBaseError::InvalidInput(message) => record(
            REASON_INVALID_INPUT,
            Some(message.clone()),
            false,
            "修正输入后重新发起",
            None,
        ),
        KnowledgeBaseError::Cancelled => record(
            REASON_CANCELLED,
            Some("操作已取消".to_string()),
            false,
            "确认工作树状态后重新发起",
            None,
        ),
        other => record(
            REASON_INTERNAL,
            Some(other.to_string()),
            false,
            "检查服务端日志后重新发起",
            None,
        ),
    }
}

// ───────────────────── 平台索引运行期隔离 ─────────────────────

/// 聚合索引锁的独占持有 guard：drop 时通知持锁线程退出临界区并放锁。
struct AggregateIndexIsolation {
    release: Option<std::sync::mpsc::Sender<()>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl AggregateIndexIsolation {
    /// 阻塞获取 `lock_path` 上的排他锁（调用方须在 blocking 线程上执行）。
    /// 持锁线程拿到锁后经 `held` 通道回执；获取失败把错误回传，避免调用方
    /// 永久等待。
    fn acquire(lock_path: PathBuf) -> Result<Self, String> {
        let (held_tx, held_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let join = std::thread::Builder::new()
            .name("kb-aggregate-index-isolation".to_string())
            .spawn(move || {
                let mut held_tx = Some(held_tx);
                let outcome = with_exact_exclusive_lock(&lock_path, || {
                    let Some(held) = held_tx.take() else {
                        return Err(ProductStoreError::Io(
                            "index isolation held channel already consumed".to_string(),
                        ));
                    };
                    if held.send(Ok(())).is_err() {
                        return Err(ProductStoreError::Io(
                            "index isolation holder channel closed".to_string(),
                        ));
                    }
                    // 协调器 drop guard ⇒ 发送端关闭 ⇒ recv 出错 ⇒ 立即放锁。
                    let _ = release_rx.recv();
                    Ok(())
                });
                // 获取锁失败（闭包未运行）：held 通道未消费，把错误回传。
                if let Err(error) = outcome
                    && let Some(held) = held_tx.take()
                {
                    let _ = held.send(Err(format!("{error:?}")));
                }
            })
            .map_err(|error| format!("spawn index isolation holder: {error}"))?;
        match held_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                release: Some(release_tx),
                join: Some(join),
            }),
            Ok(Err(message)) => {
                let _ = join.join();
                Err(message)
            }
            Err(_) => {
                let _ = join.join();
                Err("index isolation holder exited before acquiring".to_string())
            }
        }
    }
}

impl Drop for AggregateIndexIsolation {
    fn drop(&mut self) {
        // 先 drop 发送端：持锁线程的 recv 立即出错、退出临界区释放 flock。
        self.release.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// 平台索引锁路径（与生产 `AggregateIndexOperation::for_lc` 同一 LC 作用域）。
fn aggregate_index_lock_path(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
) -> Result<PathBuf, String> {
    AggregateIndexStore::for_lc(paths.clone(), lc_id.to_string())
        .lock_path(project_id)
        .map_err(|error| format!("aggregate index lock path: {error:?}"))
}
