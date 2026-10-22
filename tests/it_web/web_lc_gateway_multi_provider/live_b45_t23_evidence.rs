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
use super::snapshot::RunMode;

/// 矩阵证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/
/// <provider>/b45-t23-roles/<facet>/;设 `B45_T23_RUN_ID` 时
/// `<provider>/<run-id>/b45-t23-roles/<facet>/`,按轮隔离)。
pub(crate) fn b45_t23_evidence_root(provider: &ProviderName, facet: &str) -> PathBuf {
    provider_evidence_root(provider, B45_T23_RUN_ID_ENV)
        .join("b45-t23-roles")
        .join(facet)
}

/// F1(轮次合同修复):腿标签(capture→resume 是同一逻辑轮的两条腿,run-id
/// 共用;腿内证据必须互不覆盖)。
pub(crate) fn b45_t23_leg_label(run_mode: RunMode) -> &'static str {
    match run_mode {
        RunMode::FullChain => "full_chain",
        RunMode::CapturePlanSnapshot => "capture",
        RunMode::ResumeFromPlanSnapshot => "resume",
    }
}

/// F1:按腿分面的矩阵证据根(`matrix-<leg>`)。
///
/// run-id 不变、facet 只加腿后缀——`snapshots_root()` 取 facet 的父目录
/// (`b45-t23-roles/snapshots`),因此快照位置不随腿漂移,resume 腿仍能定位
/// capture 腿落下的快照;而 cell.json 与汇总按腿分家,resume 腿不再覆写
/// capture 腿的 fresh 格(§3.0 的 triage 误读根因)。
pub(crate) fn b45_t23_matrix_evidence_root(provider: &ProviderName, run_mode: RunMode) -> PathBuf {
    b45_t23_evidence_root(provider, &format!("matrix-{}", b45_t23_leg_label(run_mode)))
}

/// 单家汇总文件名(类1′ ①:per-provider 分文件,异家并行/多轮互不覆盖;
/// F1:再按腿分列,capture 汇总不被 resume 汇总覆盖)。
pub(crate) fn b45_t23_summary_filename(provider: &ProviderName, leg: &str) -> String {
    format!(
        "b45-t23-roles-matrix-{}-{leg}.json",
        provider_family_text(provider)
    )
}

/// 落盘单家汇总到给定证据根(纯 IO;单元测试直接调用 tempdir 根)。
///
/// 只写本家文件,不生成跨家合并视图:异家并行/同家多轮不会互相覆盖,
/// 判定只认各 provider 的 cell.json 与本家汇总(controller 需要合并视图
/// 时自行合并)。F1:文件名带腿后缀——capture 与 resume 共用 run-id 时
/// 两份汇总各自留证。
fn b45_t23_write_provider_summary_at(
    evidence_root: &Path,
    provider: &ProviderName,
    leg: &str,
    run_id: Option<&str>,
    summary: &Value,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(evidence_root)
        .map_err(|error| format!("创建汇总目录 {}: {error}", evidence_root.display()))?;
    let path = evidence_root.join(b45_t23_summary_filename(provider, leg));
    let payload = json!({
        "provider": provider_family_text(provider),
        "leg": leg,
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
pub(crate) fn b45_t23_write_provider_summary(
    provider: &ProviderName,
    leg: &str,
    summary: &Value,
) -> PathBuf {
    let run_id = evidence_run_id_from_env(B45_T23_RUN_ID_ENV);
    let root = provider_evidence_root_with_run_id(provider, run_id.as_deref())
        .unwrap_or_else(|error| panic!("{B45_T23_RUN_ID_ENV} 非法:{error}"));
    b45_t23_write_provider_summary_at(&root, provider, leg, run_id.as_deref(), summary)
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
        // 汇总文件名按家分列,异家并行不互相覆盖;F1:再按腿分列。
        assert_eq!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode, "capture"),
            "b45-t23-roles-matrix-claude-code-capture.json"
        );
        assert_ne!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode, "capture"),
            b45_t23_summary_filename(&ProviderName::Codex, "capture")
        );
        assert_ne!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode, "capture"),
            b45_t23_summary_filename(&ProviderName::ClaudeCode, "resume"),
            "同轮两腿汇总不得同名(否则 resume 覆写 capture)"
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
            "capture",
            Some("round-1"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write claude summary");
        let codex_path = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::Codex,
            "capture",
            Some("round-1"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write codex summary");
        assert_ne!(claude_path, codex_path, "异家汇总必须分文件");
        assert!(
            claude_path.ends_with("b45-t23-roles-matrix-claude-code-capture.json"),
            "汇总文件名必须带 provider 与腿:{}",
            claude_path.display()
        );

        // 同家不同 run-id:各自留证,先落的不被覆盖。
        let claude_round_two = b45_t23_write_provider_summary_at(
            &run_two,
            &ProviderName::ClaudeCode,
            "capture",
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
            "capture",
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

        // 文件内容只含本家(可逐格复算),run-id/腿如实记录。
        let written: Value =
            serde_json::from_slice(&std::fs::read(&claude_path).expect("read claude summary"))
                .expect("parse claude summary");
        assert_eq!(written["provider"], "claude-code");
        assert_eq!(written["leg"], "capture");
        assert_eq!(written["run_id"], "round-1");
        assert!(written["providers"].get("ClaudeCode").is_some());
        assert!(
            written["providers"].get("Codex").is_none(),
            "单家汇总不得混入异家条目"
        );
    }

    /// F1 负测(轮次合同修复):同一 run-id 下 capture 与 resume 两腿证据
    /// **互不覆盖**——格按腿分面、汇总按腿分文件,两腿事后都可复核(旧形态
    /// 下 resume 腿把 capture 腿 fresh 格与汇总一起写掉,是 triage 误读
    /// 「现场 cell」的根因)。
    #[test]
    fn t23_two_legs_same_run_id_do_not_overwrite_each_other() {
        let temp = tempfile::tempdir().expect("tempdir");
        let run_id = "20261021T220734Z-claude-r3-capture-resume";
        let run_root = temp.path().join("reports/claude-code").join(run_id);
        let roles = run_root.join("b45-t23-roles");
        let capture_root = roles.join("matrix-capture");
        let resume_root = roles.join("matrix-resume");
        assert_eq!(
            b45_t23_leg_label(RunMode::CapturePlanSnapshot),
            capture_root
                .file_name()
                .expect("capture facet")
                .to_str()
                .expect("utf8")
                .trim_start_matches("matrix-"),
            "腿标签必须与证据面后缀同源"
        );

        let write_leg_cell = |root: &Path, state: &str| {
            let cell_dir = root.join("story/workspace_streaming_plan-split/fresh");
            std::fs::create_dir_all(&cell_dir).expect("cell dir");
            std::fs::write(
                cell_dir.join("cell.json"),
                serde_json::to_vec_pretty(&json!({
                    "stage": "story",
                    "fresh_or_resume": "fresh",
                    "capability_state": state,
                }))
                .expect("cell json"),
            )
            .expect("write cell");
            cell_dir.join("cell.json")
        };
        let capture_cell = write_leg_cell(&capture_root, "confirmed");
        let capture_summary = b45_t23_write_provider_summary_at(
            &run_root,
            &ProviderName::ClaudeCode,
            "capture",
            Some(run_id),
            &json!({"matrix_cells": ["capture-leg"]}),
        )
        .expect("capture summary");

        // resume 腿写同名 stage/入口/相位(carried):同 run-id 下旧形态会
        // 逐字覆写上面那份格。
        let resume_cell = write_leg_cell(&resume_root, "carried");
        let resume_summary = b45_t23_write_provider_summary_at(
            &run_root,
            &ProviderName::ClaudeCode,
            "resume",
            Some(run_id),
            &json!({"matrix_cells": ["resume-leg"]}),
        )
        .expect("resume summary");

        assert_ne!(capture_cell, resume_cell, "两腿格路径必须分面");
        assert_ne!(capture_summary, resume_summary, "两腿汇总必须分文件");
        let read_state = |path: &Path| -> String {
            let doc: Value = serde_json::from_slice(&std::fs::read(path).expect("read cell"))
                .expect("parse cell");
            doc["capability_state"].as_str().unwrap_or("-").to_string()
        };
        assert_eq!(
            read_state(&capture_cell),
            "confirmed",
            "resume 腿不得覆写 capture 腿的格"
        );
        assert_eq!(read_state(&resume_cell), "carried");
        let read_cells = |path: &Path| -> Value {
            let doc: Value = serde_json::from_slice(&std::fs::read(path).expect("read summary"))
                .expect("parse summary");
            doc["providers"]["ClaudeCode"]["matrix_cells"].clone()
        };
        assert_eq!(read_cells(&capture_summary), json!(["capture-leg"]));
        assert_eq!(read_cells(&resume_summary), json!(["resume-leg"]));
        // 分面只加腿后缀:快照根不随腿漂移,resume 腿仍能定位 capture 腿的快照。
        assert_eq!(snapshots_root(&capture_root), roles.join("snapshots"));
        assert_eq!(snapshots_root(&capture_root), snapshots_root(&resume_root));
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
