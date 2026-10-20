//! T23 证据根与单家汇总(类1′ 证据完整性:per-provider 分文件 + 可选
//! `B45_T23_RUN_ID` 轮隔离)。
//!
//! 抽离理由:`live_b45_t23` 已按祖父条款登记 1651 行上限(棘轮只许删不许
//! 涨),类1′ 引入的根计算点、汇总写入与其负测整体落到本文件;纯移动、
//! 行为不变。

use std::path::{Path, PathBuf};

use cadence_aria::product::logical_codebase::provider_boundary_probe::provider_family_text;
use cadence_aria::product::models::ProviderName;
use serde_json::{Value, json};

use super::evidence_root::{
    B45_T23_RUN_ID_ENV, evidence_run_id_from_env, provider_evidence_root,
    provider_evidence_root_with_run_id,
};

/// 矩阵证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/
/// <provider>/b45-t23-roles/<facet>/;设 `B45_T23_RUN_ID` 时
/// `<provider>/<run-id>/b45-t23-roles/<facet>/`,按轮隔离)。
pub(crate) fn b45_t23_evidence_root(provider: &ProviderName, facet: &str) -> PathBuf {
    provider_evidence_root(provider, B45_T23_RUN_ID_ENV)
        .join("b45-t23-roles")
        .join(facet)
}

/// 单家汇总文件名(类1′ ①:per-provider 分文件,异家并行/多轮互不覆盖)。
pub(crate) fn b45_t23_summary_filename(provider: &ProviderName) -> String {
    format!(
        "b45-t23-roles-matrix-{}.json",
        provider_family_text(provider)
    )
}

/// 落盘单家汇总到给定证据根(纯 IO;单元测试直接调用 tempdir 根)。
///
/// 只写本家文件,不生成跨家合并视图:异家并行/同家多轮不会互相覆盖,
/// 判定只认各 provider 的 cell.json 与本家汇总(controller 需要合并视图
/// 时自行合并)。
fn b45_t23_write_provider_summary_at(
    evidence_root: &Path,
    provider: &ProviderName,
    run_id: Option<&str>,
    summary: &Value,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(evidence_root)
        .map_err(|error| format!("创建汇总目录 {}: {error}", evidence_root.display()))?;
    let path = evidence_root.join(b45_t23_summary_filename(provider));
    let payload = json!({
        "provider": provider_family_text(provider),
        "run_id": run_id,
        "providers": { format!("{provider:?}"): summary },
        "m6_codex_danger": "b45_live_codex_danger_apply_patch",
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| format!("汇总序列化失败:{error}"))?,
    )
    .map_err(|error| format!("{} 落盘失败:{error}", path.display()))?;
    Ok(path)
}

/// 每家跑完立即落本家汇总(不跨家合并、不等进程末尾):后一家失败不会
/// 抹掉前一家已落证据;证据根含可选 `B45_T23_RUN_ID` 段。
pub(crate) fn b45_t23_write_provider_summary(provider: &ProviderName, summary: &Value) -> PathBuf {
    let run_id = evidence_run_id_from_env(B45_T23_RUN_ID_ENV);
    let root = provider_evidence_root_with_run_id(provider, run_id.as_deref())
        .unwrap_or_else(|error| panic!("{B45_T23_RUN_ID_ENV} 非法:{error}"));
    b45_t23_write_provider_summary_at(&root, provider, run_id.as_deref(), summary)
        .unwrap_or_else(|error| panic!("roles-matrix.json 落盘失败:{error}"))
}

// ---------------------------------------------------------------------------
// 类1′(证据完整性)负测:per-provider 汇总 + run-id 证据根
// (纯逻辑面;不需要真实 CLI/LC_GATEWAY_E2E,合成夹具钉死拒绝形态)。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod evidence_tests {
    use super::*;
    use crate::web_lc_gateway_multi_provider::harness::ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT;
    use crate::web_lc_gateway_multi_provider::live_b45_t23::t23_assertion_tests::{
        carried_test_cell, test_manifest,
    };
    use crate::web_lc_gateway_multi_provider::snapshot::snapshots_root;

    #[test]
    fn t23_summary_paths_are_per_provider_and_run_id_scoped() {
        // 旧固定根(未设 run-id)逐字不变。
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
                .expect("legacy root"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/claude-code")
        );
        // run-id 作末段:同一 provider 不同 run-id 各自留证。
        let legacy = provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
            .expect("legacy root");
        let round_one =
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, Some("round-1"))
                .expect("run root");
        assert_eq!(round_one, legacy.join("round-1"));
        // 汇总文件名按家分列,异家并行不互相覆盖。
        assert_eq!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode),
            "b45-t23-roles-matrix-claude-code.json"
        );
        assert_ne!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode),
            b45_t23_summary_filename(&ProviderName::Codex)
        );
        // 矩阵根布局逐字钉死(⚠5 补修:原断言两侧同一实现恒真):不带与带
        // run-id 的 b45-t23-roles/<facet> 都挂在 provider 证据根下,run-id 段
        // 插在 b45-t23-roles 之前(facet 结构不因 run-id 漂移)。
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::Pi, None)
                .expect("pi legacy root")
                .join("b45-t23-roles")
                .join("matrix"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/pi/b45-t23-roles/matrix"),
            "T23 矩阵根必须钉死:facet 挂在 provider 证据根下"
        );
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::Pi, Some("r1"))
                .expect("pi run root")
                .join("b45-t23-roles")
                .join("matrix"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/pi/r1/b45-t23-roles/matrix"),
            "run-id 段必须插在 b45-t23-roles 之前(不因 run-id 漂移 facet 结构)"
        );
        assert_ne!(
            b45_t23_evidence_root(&ProviderName::Codex, "matrix"),
            b45_t23_evidence_root(&ProviderName::Pi, "matrix"),
            "异家矩阵根必须互不相同"
        );
    }

    #[test]
    fn t23_summary_write_does_not_overwrite_other_provider_or_other_run() {
        let temp = tempfile::tempdir().expect("tempdir");
        let run_one = temp.path().join("reports/claude-code/round-1");
        let run_two = temp.path().join("reports/claude-code/round-2");
        let claude_path = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::ClaudeCode,
            Some("round-1"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write claude summary");
        let codex_path = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::Codex,
            Some("round-1"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write codex summary");
        assert_ne!(claude_path, codex_path, "异家汇总必须分文件");
        assert!(
            claude_path.ends_with("b45-t23-roles-matrix-claude-code.json"),
            "汇总文件名必须带 provider:{}",
            claude_path.display()
        );

        // 同家不同 run-id:各自留证,先落的不被覆盖。
        let claude_round_two = b45_t23_write_provider_summary_at(
            &run_two,
            &ProviderName::ClaudeCode,
            Some("round-2"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write claude round two");
        assert_ne!(claude_path, claude_round_two);
        assert!(claude_path.exists() && claude_round_two.exists());

        // 重写同家同 run-id:只覆盖本家文件,不删异家、不生成跨家合并文件。
        let rewritten = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::ClaudeCode,
            Some("round-1"),
            &json!({"duties": {"m2_coder": []}}),
        )
        .expect("rewrite claude summary");
        assert_eq!(rewritten, claude_path);
        assert!(codex_path.exists(), "重写单家不得动异家文件");
        assert!(
            !run_one.join("b45-t23-roles-matrix.json").exists(),
            "不得生成跨家共享汇总文件(会互相覆盖)"
        );

        // 文件内容只含本家(可逐格复算),run-id 如实记录。
        let written: Value =
            serde_json::from_slice(&std::fs::read(&claude_path).expect("read claude summary"))
                .expect("parse claude summary");
        assert_eq!(written["provider"], "claude-code");
        assert_eq!(written["run_id"], "round-1");
        assert!(written["providers"].get("ClaudeCode").is_some());
        assert!(
            written["providers"].get("Codex").is_none(),
            "单家汇总不得混入异家条目"
        );
    }

    #[test]
    fn t23_carried_source_chain_resolves_under_run_id_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        // 带 run-id 的真实布局:<族>/<run-id>/b45-t23-roles/{matrix,snapshots}。
        let run_root = temp
            .path()
            .join("reports")
            .join("claude-code")
            .join("20261020T010203Z-capture");
        let matrix_root = run_root.join("b45-t23-roles").join("matrix");
        std::fs::create_dir_all(&matrix_root).expect("matrix root");
        let snapshot_directory = snapshots_root(&matrix_root).join("snap-1");
        assert_eq!(
            snapshots_root(&matrix_root),
            run_root.join("b45-t23-roles").join("snapshots"),
            "快照根必须是同 run-id 内的矩阵兄弟目录"
        );
        std::fs::create_dir_all(&snapshot_directory).expect("snapshot dir");
        let manifest = test_manifest(&matrix_root.to_string_lossy());
        std::fs::write(
            snapshot_directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");

        // 承继格按来源证据根(带 run-id)复核仍通过。
        let cell = carried_test_cell(
            "story",
            ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT,
            &manifest,
        );
        let canonical = PathBuf::from("/tmp/t23-root");
        let member = canonical.join("alpha");
        assert!(
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_ok(),
            "带 run-id 的来源链必须可复核: {:?}",
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
        );

        // resume 轮消费者必须用同一 run-id 计算根:facet 结构一致时落在
        // 同一相对布局(claude-code/<run-id>/b45-t23-roles/matrix)。
        let resumed_root = provider_evidence_root_with_run_id(
            &ProviderName::ClaudeCode,
            Some("20261020T010203Z-capture"),
        )
        .expect("resume root")
        .join("b45-t23-roles")
        .join("matrix");
        assert!(
            resumed_root.ends_with(Path::new(
                "claude-code/20261020T010203Z-capture/b45-t23-roles/matrix"
            )),
            "resume 轮证据根布局必须与 capture 轮同源:{}",
            resumed_root.display()
        );
        // 不接 run-id 的旧根不会命中该快照(缺同一 run-id = 证据链断裂,
        // 因此 capture→resume 必须复用同一 run-id)。
        let legacy_matrix = provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
            .expect("legacy root")
            .join("b45-t23-roles")
            .join("matrix");
        assert_ne!(
            snapshots_root(&legacy_matrix),
            snapshots_root(&matrix_root),
            "旧固定根不得装作同轮快照根"
        );
    }
}
