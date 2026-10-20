//! 证据根计算与来源根解析(唯一计算点)。
//!
//! 全部生产者(cell.json/汇总/快照/边界/失败落盘)与消费者(承继格
//! `source_evidence_root` 解析、快照续跑定位)共用本文件,防多处各自拼
//! 路径导致 run-id 只落在生产者一侧、carried 引用失效。
//!
//! 抽离理由:`live_matrix` 已按祖父条款登记 2461 行上限,`harness` 已按
//! 祖父条款登记 14966 行上限(棘轮只许删不许涨);类1′ 引入的证据根
//! run-id 计算点(⚠1 来源根一致性)与仓库根相对路径解析整体落到本文件,
//! 纯移动、行为不变。

use std::path::{Path, PathBuf};

use cadence_aria::product::logical_codebase::provider_boundary_probe::provider_family_text;
use cadence_aria::product::models::ProviderName;

// ---------------------------------------------------------------------------
// 证据根计算(唯一计算点):
// ---------------------------------------------------------------------------

/// 冻结报告根(仓库相对;计划 Files)。
pub(crate) const LC_GATEWAY_REPORTS_ROOT: &str = "cadence/reports/lc-gateway-multi-provider";

/// T22 证据根 run-id(缺省空=旧固定路径;设值作 provider 证据根末段)。
///
/// 偏差记录(类1′ 独审 ⚠4):计划 §1.7 ② 字面仅批准 `B45_T23_RUN_ID`/
/// `B45_T24_RUN_ID`,本 env 属范围外新增;行为无破坏(缺省逐字回退旧
/// 固定路径),按 §3.2-2「类1′ 改动清单 + 偏差」如实登记。
pub(crate) const B45_T22_RUN_ID_ENV: &str = "B45_T22_RUN_ID";
/// T23 证据根 run-id(同上;含五阶段/roles 矩阵/boundary/failures/汇总)。
pub(crate) const B45_T23_RUN_ID_ENV: &str = "B45_T23_RUN_ID";
/// T24 证据根 run-id(同上;pi/ 下全部 facet)。
pub(crate) const B45_T24_RUN_ID_ENV: &str = "B45_T24_RUN_ID";

/// run-id 路径段校验:必须是单一路径段(防注入逃出证据根),非空白,
/// 字符集 `[A-Za-z0-9._-]`,且不得为 `.`/`..`。
fn validate_run_id_segment(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("run-id 为空".to_string());
    }
    if matches!(trimmed, "." | "..") {
        return Err(format!("run-id 不得为 {trimmed:?}(非单一路径段)"));
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(format!(
            "run-id {trimmed:?} 含非法字符(只允许 ASCII 字母数字与 . _ -)"
        ));
    }
    Ok(trimmed.to_string())
}

/// 纯计算:provider 证据根 = `<reports>/<provider 族名>[/<run-id>]`。
///
/// - `run_id=None`(env 未设/空白)→ 旧固定路径,向后兼容;
/// - `run_id=Some`(env 设值)→ 追加为**末段**:该任务全部 facet 与兄弟
///   `snapshots/`(capture→resume 同一 run-id 共用)随之按轮隔离。
pub(crate) fn provider_evidence_root_with_run_id(
    provider: &ProviderName,
    run_id: Option<&str>,
) -> Result<PathBuf, String> {
    let mut root = PathBuf::from(LC_GATEWAY_REPORTS_ROOT).join(provider_family_text(provider));
    if let Some(run_id) = run_id {
        root.push(validate_run_id_segment(run_id)?);
    }
    Ok(root)
}

/// run-id env 读取(去空白;缺省/空白=None)。UTF-8 内的合法性由
/// [`provider_evidence_root_with_run_id`] 统一校验。
///
/// F1(类1′ 独审补修):`var_os` 显式区分「未设」与「非 UTF-8」——后者
/// 与非法 run-id 同类,直接 panic(fail-closed),不得静默视同未设而回落
/// 固定根覆盖旧轮证据(与 RunMode/B45_T23_PROVIDERS 非法值即拒一致)。
pub(crate) fn evidence_run_id_from_env(run_id_env: &str) -> Option<String> {
    let raw = std::env::var_os(run_id_env)?;
    let raw = raw.to_str().unwrap_or_else(|| {
        panic!("{run_id_env} 值非 UTF-8:非法 run-id(fail-closed,不得静默落回固定路径)")
    });
    Some(raw.trim().to_string()).filter(|trimmed| !trimmed.is_empty())
}

/// env 版唯一入口:非法 run-id fail-closed(panic,不静默落回固定路径)。
pub(crate) fn provider_evidence_root(provider: &ProviderName, run_id_env: &str) -> PathBuf {
    let run_id = evidence_run_id_from_env(run_id_env);
    provider_evidence_root_with_run_id(provider, run_id.as_deref())
        .unwrap_or_else(|error| panic!("{run_id_env} 非法:{error}"))
}

// ---------------------------------------------------------------------------
// 来源根解析与一致性校验:
// ---------------------------------------------------------------------------

/// 证据根/来源根路径解析:相对路径按仓库根(`CARGO_MANIFEST_DIR`)解析,
/// 绝对路径原样(承继格来源链与仓库根同源,不受测试进程 cwd 影响)。
pub(crate) fn resolve_repo_relative(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
    }
}

/// ⚠1(类1′ 独审补修):resume 轮快照来源根一致性——capture 轮 manifest
/// 记录的 `source_evidence_root` 必须与本次 resume 的证据根同源(capture→
/// resume 是同一逻辑轮,必须同一 run-id/同一根);不一致即 fail loudly,
/// 不得静默读到异根快照(如 capture 设 run-id 而 resume 忘设 env,回落
/// 旧固定根读到异轮 manifest)。相对路径按仓库根解析(承继格来源链同语义)。
pub(crate) fn validate_snapshot_origin(
    source_evidence_root: &str,
    current_evidence_root: &Path,
) -> Result<(), String> {
    if source_evidence_root.trim().is_empty() {
        return Err("快照 manifest 缺来源证据根(source_evidence_root 为空)".to_string());
    }
    let source_root = resolve_repo_relative(Path::new(source_evidence_root));
    let current_root = resolve_repo_relative(current_evidence_root);
    if source_root != current_root {
        return Err(format!(
            "resume 证据根与快照来源根不一致(capture→resume 必须同一 run-id/同一根):\
             当前 {} != manifest 记录 {}",
            current_root.display(),
            source_root.display()
        ));
    }
    Ok(())
}

/// Task 11 现场证据根(计划 Files 冻结):`<provider>/{boundary,failures}/`
/// (设 `B45_T23_RUN_ID` 时 `<provider>/<run-id>/{boundary,failures}/`);
/// provider 族名与证据根计算统一走本文件的单一计算点,避免与 T23 汇总/
/// 矩阵根各自拼路径导致 run-id 只落一处。
pub(crate) fn task11_evidence_roots(provider: &ProviderName) -> (PathBuf, PathBuf) {
    let base = provider_evidence_root(provider, B45_T23_RUN_ID_ENV);
    (base.join("boundary"), base.join("failures"))
}

// ---------------------------------------------------------------------------
// 证据根 run-id 纯逻辑负测(类1′ ②;不需要真实 CLI/LC_GATEWAY_E2E):
// 未设 env 旧行为不变、设值作末段且按轮隔离、注入形态 fail-closed、
// 快照兄弟目录随根同轮隔离(capture→resume 同 run-id 同根)。
// 补修:F1 非 UTF-8 fail-closed;⚠1 capture↔resume 来源根一致性。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod evidence_root_tests {
    use super::*;
    use crate::web_lc_gateway_multi_provider::snapshot::snapshots_root;

    fn legacy(provider: &ProviderName) -> PathBuf {
        PathBuf::from(LC_GATEWAY_REPORTS_ROOT).join(provider_family_text(provider))
    }

    #[test]
    fn evidence_root_without_run_id_keeps_legacy_fixed_path() {
        for provider in [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ] {
            assert_eq!(
                provider_evidence_root_with_run_id(&provider, None).expect("legacy root"),
                legacy(&provider),
                "未设 run-id 必须保持旧固定路径"
            );
        }
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
                .expect("legacy root"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/claude-code"),
            "旧固定路径逐字不变(claude-code)"
        );
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::KimiCode, None).expect("legacy root"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/kimi-code"),
            "旧固定路径逐字不变(kimi-code)"
        );
        // 空白 env 视同未设(读取侧去空白后过滤空串)。
        assert_eq!(
            evidence_run_id_from_env("B45_TEST_RUN_ID_DEFINITELY_UNSET"),
            None,
            "未设 run-id env 必须为 None(向后兼容前提)"
        );
    }

    #[test]
    fn evidence_root_appends_run_id_as_last_segment() {
        let run_root =
            provider_evidence_root_with_run_id(&ProviderName::Pi, Some("20261020T010203Z-capture"))
                .expect("run root");
        assert_eq!(
            run_root,
            legacy(&ProviderName::Pi).join("20261020T010203Z-capture"),
            "run-id 必须是证据根末段"
        );
        // 同家三轮各自留证:不同 run-id 根互不相同,也不覆盖旧固定根。
        let round_two =
            provider_evidence_root_with_run_id(&ProviderName::Pi, Some("20261020T020304Z-capture"))
                .expect("run root 2");
        assert_ne!(run_root, round_two, "不同 run-id 必须各自留证");
        assert_ne!(
            run_root,
            legacy(&ProviderName::Pi),
            "带 run-id 不落旧固定根"
        );
        // 异家并行:同 run-id 下各家族根互不相同。
        let other_family = provider_evidence_root_with_run_id(
            &ProviderName::KimiCode,
            Some("20261020T010203Z-capture"),
        )
        .expect("run root");
        assert_ne!(
            run_root, other_family,
            "同 run-id 异家证据根必须互不相同(不互相覆盖)"
        );
    }

    #[test]
    fn evidence_root_run_id_scopes_snapshot_sibling_for_capture_and_resume() {
        let run_root =
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, Some("round-1"))
                .expect("run root");
        let matrix_root = run_root.join("b45-t23-roles").join("matrix");
        // capture→resume 同根(⚠5 补修):布局逐字钉死——报告根/家族映射/
        // 段序任一漂移即红;不再用纯函数自证幂等(原断言两侧同一计算,恒真)。
        assert_eq!(
            matrix_root,
            PathBuf::from(
                "cadence/reports/lc-gateway-multi-provider/claude-code/round-1/b45-t23-roles/matrix"
            ),
            "capture→resume 证据根布局必须逐字钉死(run-id 段在 b45-t23-roles 之前)"
        );
        // 快照兄弟目录(capture 落盘 / resume 定位)随 run-id 同轮隔离。
        assert_eq!(
            snapshots_root(&matrix_root),
            run_root.join("b45-t23-roles").join("snapshots"),
            "快照根必须是同 run-id 内的兄弟目录"
        );
        assert_eq!(
            snapshots_root(&run_root.join("matrix")),
            run_root.join("snapshots"),
            "五阶段矩阵快照根必须是同 run-id 内的兄弟目录"
        );
    }

    #[test]
    fn evidence_root_rejects_run_id_escaping_provider_directory() {
        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b",
            "..\\escape",
            "a b",
            "a:b",
            "r1/",
        ] {
            assert!(
                provider_evidence_root_with_run_id(&ProviderName::Pi, Some(bad)).is_err(),
                "非法 run-id {bad:?} 必须 fail-closed(不得静默拼进证据根)"
            );
        }
        for good in ["r1", "round-1", "20261020T010203Z", "a_b.c-1"] {
            assert!(
                provider_evidence_root_with_run_id(&ProviderName::Pi, Some(good)).is_ok(),
                "合法 run-id {good:?} 必须接受"
            );
        }
    }

    // F1(类1′ 独审补修):非 UTF-8 run-id 必须 fail-closed(panic),
    // 不得静默视同未设而回落固定路径(否则整轮证据覆盖旧固定根轮次)。
    #[test]
    #[cfg(unix)]
    fn evidence_run_id_non_utf8_fails_closed_not_silent_fallback() {
        use std::os::unix::ffi::OsStringExt;
        let env_key = "B45_TEST_RUN_ID_NON_UTF8";
        unsafe {
            std::env::set_var(
                env_key,
                std::ffi::OsString::from_vec(vec![0xff, 0xfe, 0xfd]),
            );
        }
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panicked = std::panic::catch_unwind(|| evidence_run_id_from_env(env_key)).is_err();
        std::panic::set_hook(previous_hook);
        unsafe {
            std::env::remove_var(env_key);
        }
        assert!(
            panicked,
            "非 UTF-8 run-id 必须 panic(fail-closed),不得静默当未设回落固定路径"
        );
    }

    // ⚠1(类1′ 独审补修):capture↔resume 根一致性——快照 manifest 记录的
    // 来源证据根与 resume 当前根不一致(如 capture 设 run-id 而 resume 忘设
    // env 落回旧固定根读到异根快照)必须 fail loudly,不得静默续跑。
    #[test]
    fn snapshot_origin_mismatch_fails_loudly_on_resume_root_divergence() {
        let captured = "cadence/reports/lc-gateway-multi-provider/claude-code/round-1/matrix";
        let resume_legacy =
            Path::new("cadence/reports/lc-gateway-multi-provider/claude-code/matrix");
        let error = validate_snapshot_origin(captured, resume_legacy)
            .expect_err("异根 resume 必须被拒绝(不得静默读异轮快照)");
        assert!(error.contains("不一致"), "错误必须点明来源根不一致:{error}");
        // 同根放行:manifest 相对 + resume 相对(同仓库根解析)。
        assert_eq!(
            validate_snapshot_origin(captured, Path::new(captured)),
            Ok(()),
            "capture→resume 同根(相对)必须放行"
        );
        // 同根放行:manifest 相对 + resume 绝对(等价解析)。
        let absolute = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(captured);
        assert_eq!(
            validate_snapshot_origin(captured, &absolute),
            Ok(()),
            "capture→resume 同根(manifest 相对、resume 绝对)必须放行"
        );
    }
}
