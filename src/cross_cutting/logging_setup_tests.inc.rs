#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// 独立临时状态根（测试产物只落 /tmp，见 test-content-tmp-only）。
    fn temp_state_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "cadence-aria-logging-setup-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp state root");
        root
    }

    /// 默认口径：`info` 的两条诊断模块事件必须落进指定 writer 指向的文件；
    /// 未打开的级别/目标不得落盘（过滤口径与「不打印无关内容」一并锁定）。
    #[test]
    fn default_filter_writes_diagnostic_events_to_log_file() {
        let state_root = temp_state_root("file-sink");
        let log_file = log_file_path(&state_root);
        assert_eq!(log_file, state_root.join(LOG_DIR).join(LOG_FILE_NAME));

        let (dispatch, sink) = build_dispatch(&state_root, build_filter(None));
        assert!(
            matches!(&sink, LogSink::FileAndStderr { .. }),
            "日志文件可写时必须双写：{sink:?}"
        );

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(
                target: "cadence_aria::product::provider_installation",
                chosen_prefix = "probe-prefix-logging-setup",
                "provider install: effective prefix decision"
            );
            tracing::info!(
                target: "cadence_aria::product::logical_codebase::aggregate_index",
                payload_bytes = 424242_u64,
                "codegraph aggregate coverage decision"
            );
            // 默认口径下全局只有 warn：相关模块之外的 info 不得落盘。
            tracing::info!(
                target: "cadence_aria::cross_cutting::logging_setup_probe",
                unrelated_probe = "must-not-appear-unrelated-logging-setup",
                "unrelated info probe"
            );
            // 相关模块默认只开到 info：debug 不得落盘。
            tracing::debug!(
                target: "cadence_aria::product::provider_installation",
                debug_probe = "must-not-appear-debug-logging-setup",
                "debug probe"
            );
        });

        let text = std::fs::read_to_string(&log_file).expect("log file must exist");
        assert!(
            text.contains("chosen_prefix=") && text.contains("probe-prefix-logging-setup"),
            "安装前缀决策链必须落盘：{text}"
        );
        assert!(
            text.contains("payload_bytes=424242"),
            "聚合索引覆盖判定必须落盘：{text}"
        );
        assert!(
            !text.contains("must-not-appear-unrelated-logging-setup"),
            "相关模块之外的 info 不得落盘：{text}"
        );
        assert!(
            !text.contains("must-not-appear-debug-logging-setup"),
            "相关模块默认只开到 info：{text}"
        );

        let _ = std::fs::remove_dir_all(&state_root);
    }

    /// `RUST_LOG` 存在且非空 ⇒ 完全取代默认口径（不是叠加）。
    #[test]
    fn rust_log_override_replaces_default_directives() {
        let state_root = temp_state_root("rust-log-override");
        let log_file = log_file_path(&state_root);
        let (dispatch, sink) = build_dispatch(&state_root, build_filter(Some("error")));
        assert!(
            matches!(&sink, LogSink::FileAndStderr { .. }),
            "日志文件可写时必须双写：{sink:?}"
        );

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(
                target: "cadence_aria::product::provider_installation",
                override_probe = "must-not-appear-rust-log-error",
                "info probe under RUST_LOG=error"
            );
        });

        let text = std::fs::read_to_string(&log_file).unwrap_or_default();
        assert!(
            !text.contains("must-not-appear-rust-log-error"),
            "RUST_LOG=error 必须压掉默认打开的 info：{text}"
        );

        let _ = std::fs::remove_dir_all(&state_root);
    }

    /// 日志文件打不开 ⇒ 降级为只写 stderr（并带一行告警），绝不失败。
    #[test]
    fn unwritable_log_file_degrades_to_stderr_only() {
        let state_root = temp_state_root("degrade");
        // `logs` 被普通文件占住 ⇒ `create_dir_all` 必失败。
        std::fs::write(state_root.join(LOG_DIR), "blocked").expect("blocker file");

        let (_dispatch, sink) = build_dispatch(&state_root, build_filter(None));
        match sink {
            LogSink::StderrOnly { log_file, reason } => {
                assert_eq!(log_file, log_file_path(&state_root));
                assert!(!reason.is_empty(), "降级必须留下原因");
            }
            other => panic!("日志文件不可写时必须降级为只写 stderr：{other:?}"),
        }

        let _ = std::fs::remove_dir_all(&state_root);
    }

    /// 幂等：同一闸门重复初始化只构建/安装一次，后续调用直接返回
    /// `AlreadyInstalled`（不重复挂 subscriber，也不重开文件）。
    #[test]
    fn init_is_idempotent_under_one_gate() {
        let state_root = temp_state_root("idempotent");
        let gate = InstallGate::new();
        let installs = AtomicUsize::new(0);

        let first = init_with(&state_root, &gate, |_dispatch| {
            installs.fetch_add(1, Ordering::SeqCst);
            true
        });
        assert!(
            matches!(first, TracingInit::Installed { .. }),
            "首次调用必须安装：{first:?}"
        );

        let second = init_with(&state_root, &gate, |_dispatch| {
            installs.fetch_add(1, Ordering::SeqCst);
            true
        });
        assert_eq!(
            second,
            TracingInit::AlreadyInstalled,
            "重复调用不得再次安装"
        );
        assert_eq!(installs.load(Ordering::SeqCst), 1, "安装必须只发生一次");

        let _ = std::fs::remove_dir_all(&state_root);
    }

    /// 全局 subscriber 槽位已被他人占用（例如测试捕获 fixture）时，不得
    /// 谎报安装成功：返回 `AlreadyInstalled`，且不影响进程后续行为。
    #[test]
    fn install_conflict_reports_already_installed() {
        let state_root = temp_state_root("install-conflict");
        let gate = InstallGate::new();

        let result = init_with(&state_root, &gate, |_dispatch| false);
        assert_eq!(result, TracingInit::AlreadyInstalled);

        let _ = std::fs::remove_dir_all(&state_root);
    }

    /// 默认口径与文档承诺的两个模块名必须一致（文档 grep 指引据此写）。
    #[test]
    fn default_directives_cover_the_two_diagnostic_modules() {
        assert!(DEFAULT_FILTER_DIRECTIVES.starts_with("warn"));
        assert!(
            DEFAULT_FILTER_DIRECTIVES.contains("cadence_aria::product::provider_installation=info")
        );
        assert!(DEFAULT_FILTER_DIRECTIVES
            .contains("cadence_aria::product::logical_codebase::aggregate_index=info"));
    }
}
