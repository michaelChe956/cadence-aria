use super::*;

/// 流日志目录必须落在 attempt 根下，并与 `provider-raw` 同父目录，
/// 以保证删除 attempt 目录时流日志一并清理。
#[test]
fn provider_stream_log_root_sits_under_the_attempt_dir_next_to_provider_raw() {
    let (_tmp, store, attempt) = setup();

    let stream_root =
        store.provider_stream_log_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    let raw_root =
        store.provider_raw_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    let attempt_dir = store.attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id);

    assert_eq!(stream_root, attempt_dir.join("provider-streams"));
    assert_eq!(stream_root.parent(), raw_root.parent());
    assert_eq!(stream_root.parent(), Some(attempt_dir.as_path()));
    assert!(
        stream_root.starts_with(&attempt_dir),
        "stream log root must stay inside the attempt dir: {}",
        stream_root.display()
    );
}

/// 不同 attempt 的流日志目录必须彼此隔离。
#[test]
fn provider_stream_log_root_is_scoped_per_attempt() {
    let (_tmp, store, attempt) = setup();

    let stream_root =
        store.provider_stream_log_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    let other_root = store.provider_stream_log_root(
        &attempt.project_id,
        &attempt.issue_id,
        "coding_attempt_other",
    );

    assert_ne!(stream_root, other_root);
}
