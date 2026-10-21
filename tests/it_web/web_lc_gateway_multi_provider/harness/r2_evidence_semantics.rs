//! 简化夹具批次 r2 的三个红因(A1/A2/B1,另含 C1 夹具一致性的钉针)。
//!
//! 诊断:`/tmp/aria-bug45/t25/diag-simplified-r2.md`。三处红**都不是产品
//! 行为错误**,而是测试侧证据口径:
//! - A1:coding resume 格 attach 到已停人工终门的 attempt 时,产品按设计
//!   **不重启 runner**(`coding_ws_handler/socket/resumption.rs`
//!   `ensure_runner_for_resumed_attempt` 只对半启动 Running attempt 重启),
//!   零 spawn 是正确语义。该格必须显式标 `not_applicable`/
//!   `no_pending_work`,不得判成「执行 resume 未起进程」。
//! - A2:`requested_resume_id == native_confirmed_id` 单独是自证(两侧读同
//!   一条既有审计),必须另有本格**新增 ProviderStart** 或真实在途重挂,
//!   才能把「空转」与「真实恢复」区分开。
//! - B1:`scenario_fingerprint_drift` 的旧突变(commit 前移 HEAD)不改产品
//!   指纹维度(canonical target git identity=`.git` 路径),spawn 前复验必然
//!   放行;改为把 `.git` 重指到**可解析的替代 gitdir**,使复验真触发
//!   `PolicyDrift{resume_fingerprint}`。
//!
//! 产品代码零改动:本文件只承载测试侧判据/夹具突变,harness 只留薄接线。

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    ENTRYPOINT_SPLIT_SYNC, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, EvidenceCell,
    MatrixEnvironment, ProviderName, RESUME, STAGE_ORDER, ScenarioOutcome, StageObservation,
    Task11MutationGuard,
};

/// A1 格的执行来源标注:resume attach 已无可恢复工作(产品按设计不重启)。
pub(crate) const NO_PENDING_WORK_ORIGIN: &str = "no_pending_work";

/// B1:期望由 spawn 前复验给出的漂移维度(产品
/// `ProviderGatewayError::PolicyDrift` 的 Display 前缀 + dimension)。
const RESUME_FINGERPRINT_DRIFT: &str = "provider_gateway_policy_drift: resume_fingerprint";

/// A1 + A2:resume 格结论(替换 harness `build_cell` 里的 resume 分支)。
pub(super) fn resume_cell_verdict(cell: &mut EvidenceCell, observation: &StageObservation) {
    if observation.observed_spawn_count == 0
        && !observation.reattached_inflight_run
        && observation.no_pending_work_reason.is_some()
    {
        cell.capability_state = "not_applicable".to_string();
        cell.denied_reason = observation.no_pending_work_reason.clone();
        cell.execution_origin = NO_PENDING_WORK_ORIGIN.to_string();
        return;
    }
    if super::coding_resume_native_confirmed(
        &observation.requested_resume_id,
        &observation.native_confirmed_id,
        observation.reattached_inflight_run,
        observation.observed_spawn_count > 0,
    ) {
        cell.capability_state = "confirmed".to_string();
    } else {
        cell.capability_state = "unknown".to_string();
        cell.denied_reason = Some(format!(
            "原生恢复未确认:请求 {:?} 实测 {:?}(在途重挂={},本格新增 ProviderStart={})",
            observation.requested_resume_id,
            observation.native_confirmed_id,
            observation.reattached_inflight_run,
            observation.observed_spawn_count > 0
        ));
    }
}

/// A1:resume 泵收口后的「无待恢复工作」判据(产品语义见模块注释)。
///
/// 三条件缺一不成立:泵确认已达终门/终态、本格零新增 spawn、既无在途重挂
/// 也无运行失败——任一不满足都仍按「执行 resume」判(native id + 新增
/// ProviderStart),不借 not_applicable 逃避判等。
pub(super) fn no_pending_work_reason(
    observation: &StageObservation,
    artifact_confirmed: bool,
) -> Option<String> {
    if !artifact_confirmed
        || observation.run_failure.is_some()
        || observation.observed_spawn_count > 0
        || observation.reattached_inflight_run
    {
        return None;
    }
    let (status, stage) = terminal_state_without_work(observation)?;
    Some(format!(
        "无待恢复工作:resume attach 时 attempt 已停在 status={status}/stage={stage},\
         产品按设计不重启 runner(ensure_runner_for_resumed_attempt 仅对半启动 \
         Running attempt 重启);本格零新增 ProviderStart 审计,不计作执行 resume"
    ))
}

/// 泵内最后一帧 coding_session_state 是否为「已停终门/终态」。
fn terminal_state_without_work(observation: &StageObservation) -> Option<(String, String)> {
    let frame = observation.events.iter().rev().find_map(|event| {
        let payload = event.get("event")?;
        (payload.get("type").and_then(Value::as_str) == Some("coding_session_state"))
            .then_some(payload)
    })?;
    let status = frame.get("status").and_then(Value::as_str)?;
    let stage = frame
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match status {
        // 终门判据与产品确认口径同源(中途门/rework 上限/共享 worktree 脏
        // 同样落 waiting_for_human,那些形态仍有待恢复工作)。
        "waiting_for_human" if super::coding_waiting_human_confirms(frame) => {
            Some((status.to_string(), stage))
        }
        "completed" | "confirmed" => Some((status.to_string(), stage)),
        _ => None,
    }
}

/// A1:not_applicable 格的自证(结构面断言 + 断言侧共用)。
///
/// 该状态只能由「终门空转」产生,故必须同时满足:标注来源、零新增 spawn、
/// 原生会话 id 前后一致、非空原因、以及 zero_spawn_observation 的审计前后
/// 相等(可证伪的零新增 ProviderStart)。任一缺失即红——状态名本身不构成
/// 证据,不得用它逃避 resume 判等。
pub(crate) fn not_applicable_evidence_error(cell: &EvidenceCell) -> Option<String> {
    if cell.execution_origin != NO_PENDING_WORK_ORIGIN {
        return Some(format!(
            "not_applicable 格 origin 必须是 {NO_PENDING_WORK_ORIGIN},实际 {:?}",
            cell.execution_origin
        ));
    }
    if cell.fresh_or_resume != RESUME {
        return Some(format!(
            "not_applicable 只适用于 resume 格,实际 {}",
            cell.fresh_or_resume
        ));
    }
    if cell.provider_spawn_count != 0 {
        return Some(format!(
            "not_applicable 格必须零新增 spawn,实测 {}",
            cell.provider_spawn_count
        ));
    }
    match (&cell.requested_resume_id, &cell.native_resume_confirmed_id) {
        (Some(requested), Some(confirmed))
            if !requested.trim().is_empty() && requested == confirmed => {}
        _ => {
            return Some(format!(
                "not_applicable 格缺原生会话一致证据:请求 {:?} 确认 {:?}",
                cell.requested_resume_id, cell.native_resume_confirmed_id
            ));
        }
    }
    if cell
        .denied_reason
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Some("not_applicable 格缺非空原因(无待恢复工作的现场说明)".to_string());
    }
    let observation = cell.provider_events.iter().find_map(|event| {
        let payload = event.get("event")?;
        (payload.get("type").and_then(Value::as_str) == Some("zero_spawn_observation"))
            .then_some(payload)
    });
    let Some(observation) = observation else {
        return Some("not_applicable 格缺 zero_spawn_observation(零新增不可证)".to_string());
    };
    let before = observation
        .get("provider_start_audits_before")
        .and_then(Value::as_u64);
    let after = observation
        .get("provider_start_audits_after")
        .and_then(Value::as_u64);
    match (before, after) {
        (Some(before), Some(after)) if before == after => None,
        _ => Some(format!(
            "not_applicable 格审计前后不相等或缺记(before={before:?} after={after:?};零新增不可证)"
        )),
    }
}

/// A1:`validate_against` 的 not_applicable 分支(结构面).
pub(super) fn validate_not_applicable(
    cell: &EvidenceCell,
    expected_provider: &ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
) -> Result<(), String> {
    if cell.provider != *expected_provider {
        return Err(format!(
            "provider 不一致:格 {:?} != 所选 {expected_provider:?}",
            cell.provider
        ));
    }
    if !STAGE_ORDER.contains(&cell.stage.as_str()) {
        return Err(format!("stage 不在五阶段枚举内:{}", cell.stage));
    }
    if cell.entrypoint != ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT
        && cell.entrypoint != ENTRYPOINT_SPLIT_SYNC
    {
        return Err(format!("entrypoint 不在两入口枚举内:{}", cell.entrypoint));
    }
    if cell.process_cwd != canonical_root {
        return Err(format!(
            "进程 cwd 不是 canonical root:{:?} != {:?}",
            cell.process_cwd, canonical_root
        ));
    }
    if cell.target != member_worktree {
        return Err(format!(
            "target 不是成员 worktree:{:?} != {:?}",
            cell.target, member_worktree
        ));
    }
    not_applicable_evidence_error(cell).map_or(Ok(()), Err)
}

/// A1:断言侧 not_applicable 格检查(带 run_ref,定位现场)。
pub(crate) fn verify_not_applicable_resume_cell(cell: &EvidenceCell) -> Result<(), String> {
    not_applicable_evidence_error(cell).map_or(Ok(()), |error| {
        Err(format!(
            "not_applicable 格 {} 自证不足:{error}",
            cell.run_ref
        ))
    })
}

/// A1:resume 轮的 coding/review resume 格状态判据(confirmed 或自证齐备的
/// not_applicable;其余一律非法,不借状态名逃避 resume 判等)。
pub(crate) fn verify_resume_phase_cell(stage: &str, cell: &EvidenceCell) -> Result<(), String> {
    match cell.capability_state.as_str() {
        "not_applicable" => not_applicable_evidence_error(cell).map_or(Ok(()), |error| {
            Err(format!("resume 轮 {stage} 空转格自证不足:{error}"))
        }),
        "confirmed" => Ok(()),
        other => Err(format!(
            "resume 轮 {stage} 的 resume 格状态 {other} 非法\
             (只允许 confirmed 或自证齐备的 not_applicable)"
        )),
    }
}

/// B3:失败格计数口径——格内 provider_spawn_count 必须等于自身
/// zero_spawn_observation 的审计 delta(不再恒 0),且 delta>0 与
/// 「denied/unknown 零 spawn 拒绝格」互斥(fail-closed)。
pub(crate) fn verify_failure_audit_delta(
    scenario: &str,
    cell: &EvidenceCell,
    observation: &Value,
) -> Result<(), String> {
    let (Some(before), Some(after)) = (
        observation["provider_start_audits_before"].as_u64(),
        observation["provider_start_audits_after"].as_u64(),
    ) else {
        return Err(format!(
            "失败场景 {scenario}:zero_spawn_observation 缺审计前后计数(零 spawn 不可证)"
        ));
    };
    let delta = after.saturating_sub(before);
    if cell.provider_spawn_count != delta {
        return Err(format!(
            "失败场景 {scenario}:provider_spawn_count={} 与审计 delta={delta} 不自洽",
            cell.provider_spawn_count
        ));
    }
    if delta > 0 {
        return Err(format!(
            "失败场景 {scenario}:拒绝格真起了 {delta} 次 provider 启动(零 spawn 负向证明不成立)"
        ));
    }
    Ok(())
}

/// 场景 15:resume_fingerprint 漂移(spawn 前复验真拒)。
///
/// 旧突变(真实 commit 前移 HEAD)落在产品的**非**指纹维度:产品指纹的
/// target git identity 是 `canonical_target_git_identity(target.worktree)`
/// (`provider_gateway.rs:640-657`,取 canonical `.git` 路径或其 gitdir
/// 目标),HEAD/refs 前移不改该路径 ⇒ 复验放行(该场景实测真起进程后被
/// cancel)。正常 coding 自提交必须不毁 resume,产品语义正确,缺陷在场景。
///
/// 本场景改为把成员 worktree 的 `.git` 重指到一个**可解析但不同**的替代
/// gitdir:冻结指纹取原 canonical `.git`,复验重解析得到替代路径 ⇒
/// `PolicyDrift{dimension:"resume_fingerprint"}`,零 spawn(refused=true)。
/// 与场景 16(resolver 重解析失败:指针不可解析)语义不重复。
pub(super) async fn scenario_fingerprint_drift(
    env: &MatrixEnvironment,
    sid: &str,
) -> ScenarioOutcome {
    let mut drift: Option<GitdirDrift> = None;
    let outcome = super::task11_revalidate_drift(
        env,
        sid,
        "resume_fingerprint(target git identity)",
        |env: &MatrixEnvironment| {
            drift = Some(GitdirDrift::apply(&env.member_worktree)?);
            Ok(Task11MutationGuard::None)
        },
    )
    .await;
    if let Some(drift) = drift {
        drift.restore();
    }
    enforce_refusal_dimension(outcome)
}

/// 拒绝维度必须落在 `resume_fingerprint` 上(否则本场景不再构成该维度的
/// 负向证明)——被拒但维度不符时如实落红,不以「拒了就算过」冒充。
fn enforce_refusal_dimension(outcome: ScenarioOutcome) -> ScenarioOutcome {
    if !outcome.refused || outcome.refusal_text.contains(RESUME_FINGERPRINT_DRIFT) {
        return outcome;
    }
    ScenarioOutcome {
        refused: false,
        refusal_gate: "spawn 前复验(resume_fingerprint)".to_string(),
        refusal_text: format!(
            "拒绝维度不符:期望 {RESUME_FINGERPRINT_DRIFT},实测 {}",
            outcome.refusal_text
        ),
        state: "denied",
        detail: Value::Null,
        extra_events: Vec::new(),
    }
}

/// B1 受控突变:把 `worktree/.git` 目录换成指向可解析替代 gitdir 的指针
/// 文件(`restore` 复位;持有到 spawn 前复验完成)。
#[derive(Debug)]
struct GitdirDrift {
    worktree: PathBuf,
    git_entry: PathBuf,
    stashed_git: PathBuf,
    alt_gitdir: PathBuf,
}

impl GitdirDrift {
    fn apply(worktree: &Path) -> Result<Self, String> {
        let git_entry = worktree.join(".git");
        if !git_entry.is_dir() {
            return Err(format!(
                "成员 worktree 的 .git 不是目录形态({}),本场景需要目录形态",
                git_entry.display()
            ));
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let alt_gitdir =
            std::env::temp_dir().join(format!("t11-alt-gitdir-{}-{nanos}", std::process::id()));
        let status = std::process::Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&alt_gitdir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => return Err(format!("git init --bare 替代 gitdir 失败({status})")),
            Err(error) => return Err(format!("git init --bare 替代 gitdir 失败:{error}")),
        }
        let alt_canonical = match alt_gitdir.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&alt_gitdir);
                return Err(format!("替代 gitdir 不可解析:{error}"));
            }
        };
        let stashed_git = worktree.join(".git.t11-drift-real");
        if let Err(error) = std::fs::rename(&git_entry, &stashed_git) {
            let _ = std::fs::remove_dir_all(&alt_gitdir);
            return Err(format!("移开 .git 目录失败:{error}"));
        }
        if let Err(error) =
            std::fs::write(&git_entry, format!("gitdir: {}\n", alt_canonical.display()))
        {
            let _ = std::fs::rename(&stashed_git, &git_entry);
            let _ = std::fs::remove_dir_all(&alt_gitdir);
            return Err(format!("写 .git 指针失败:{error}"));
        }
        super::fp(
            "t11_scenario_git_mutation",
            format_args!(
                "target .git 重指到可解析替代 gitdir({});原目录暂存 {};场景后复位",
                alt_canonical.display(),
                stashed_git.display()
            ),
        );
        Ok(Self {
            worktree: worktree.to_path_buf(),
            git_entry,
            stashed_git,
            alt_gitdir,
        })
    }

    fn restore(&self) {
        if self.stashed_git.is_dir() {
            let _ = std::fs::remove_file(&self.git_entry);
            if std::fs::rename(&self.stashed_git, &self.git_entry).is_ok() {
                super::fp(
                    "t11_git_restored",
                    format_args!(
                        "{} 恢复为原 .git 目录(fingerprint 漂移复位)",
                        self.worktree.display()
                    ),
                );
            }
        }
        let _ = std::fs::remove_dir_all(&self.alt_gitdir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_git_identity(worktree: &Path) -> String {
        let git_entry = worktree.join(".git");
        if git_entry.is_dir() {
            return git_entry
                .canonicalize()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        let pointer = std::fs::read_to_string(&git_entry).unwrap_or_default();
        pointer
            .lines()
            .find_map(|line| line.trim().strip_prefix("gitdir:"))
            .and_then(|gitdir| Path::new(gitdir.trim()).canonicalize().ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn fixture_repo(root: &Path) -> PathBuf {
        let repo = root.join("alpha");
        std::fs::create_dir_all(&repo).expect("create repo dir");
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q"])
            .status()
            .expect("git init");
        assert!(status.success(), "fixture git init 必须成功");
        repo
    }

    /// C1(夹具自相矛盾):三处矩阵请求体必须一致启用 Superpowers。
    #[test]
    fn c1_matrix_fixture_bodies_enable_superpowers() {
        let story = super::super::pinned_story_generate_body("t", "alpha", "claude_code");
        let design = super::super::pinned_design_generate_body(
            "t",
            &["story_1".to_string()],
            "alpha",
            "claude_code",
        );
        let plan =
            super::super::plan_prepare_body(&["story_1".to_string()], "design_1", "claude_code");
        for (label, body) in [("story", story), ("design", design), ("plan", plan)] {
            assert_eq!(
                body["superpowers_enabled"].as_bool(),
                Some(true),
                "{label} 请求体必须启用 Superpowers(否则产品提示词自相矛盾:必调 Skill + 未启用必须停止报告)"
            );
        }
    }

    /// B1:突变必须把 `.git` 解析结果改成一个**可解析但不同**的路径。
    #[test]
    fn b1_gitdir_drift_repoints_to_resolvable_different_gitdir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = fixture_repo(temp.path());
        let baseline = resolve_git_identity(&repo);
        assert!(!baseline.is_empty(), "基线 .git 必须可解析");

        let drift = GitdirDrift::apply(&repo).expect("受控 gitdir 突变");
        let pointer = std::fs::read_to_string(repo.join(".git")).expect("指针文件");
        assert!(
            pointer.starts_with("gitdir:"),
            ".git 必须变成 gitdir 指针形态: {pointer:?}"
        );
        let drifted = resolve_git_identity(&repo);
        assert!(!drifted.is_empty(), "替代 gitdir 必须可解析");
        assert_ne!(
            drifted, baseline,
            "突变后的 git identity 必须与冻结基准不同(否则复验不可能拒)"
        );

        drift.restore();
        assert!(repo.join(".git").is_dir(), "复位后 .git 必须回到目录形态");
        assert_eq!(
            resolve_git_identity(&repo),
            baseline,
            "复位必须回到冻结基准 identity"
        );
        assert!(
            !repo.join(".git.t11-drift-real").exists(),
            "暂存目录必须清掉"
        );
        assert!(!drift.alt_gitdir.exists(), "替代 gitdir 必须清掉");
    }

    /// B1:只有 `resume_fingerprint` 维度的拒绝才算本场景的负向证明。
    #[test]
    fn b1_fingerprint_drift_requires_resume_fingerprint_dimension() {
        let matched = super::super::scenario_refused(
            "spawn 前复验(resume_fingerprint(target git identity))",
            "provider_gateway_policy_drift: resume_fingerprint".to_string(),
        );
        let kept = enforce_refusal_dimension(matched);
        assert!(kept.refused, "正确维度必须保持 refused=true");

        let wrong = super::super::scenario_refused(
            "spawn 前复验(其它维度)",
            "provider_gateway_policy_drift: config_digest".to_string(),
        );
        let red = enforce_refusal_dimension(wrong);
        assert!(
            !red.refused,
            "维度不符不得记作 refused(不得以任意拒绝冒充该维度负向证明)"
        );
        assert!(
            red.refusal_text.contains("拒绝维度不符"),
            "{:?}",
            red.refusal_text
        );

        let spawned = super::super::scenario_not_refused(
            "spawn 前复验(resume_fingerprint(target git identity))",
            "维度漂移未被复验拒绝".to_string(),
        );
        assert!(
            !enforce_refusal_dimension(spawned).refused,
            "未拒绝形态原样透传(仍由断言判红)"
        );
    }

    /// B1:非目录形态(linked worktree 指针)不做突变,如实不可观察。
    #[test]
    fn b1_gitdir_drift_refuses_non_directory_git_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("alpha");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::fs::write(repo.join(".git"), "gitdir: /tmp/elsewhere\n").expect("pointer");
        let error = GitdirDrift::apply(&repo).expect_err("非目录形态必须拒绝突变");
        assert!(error.contains("不是目录形态"), "{error}");
    }

    /// A1:终门空转判据——已停 final_confirm、零 spawn、无失败 ⇒ 无待恢复工作。
    #[test]
    fn a1_terminal_gate_idle_is_no_pending_work() {
        let mut observation = StageObservation::new("coding", &ProviderName::ClaudeCode);
        observation.observed_spawn_count = 0;
        observation.events.push(serde_json::json!({
            "event": {
                "type": "coding_session_state",
                "status": "waiting_for_human",
                "stage": "final_confirm",
            }
        }));
        let reason = no_pending_work_reason(&observation, true).expect("终门空转必须识别");
        assert!(reason.contains("不重启 runner"), "{reason}");
    }

    /// A1:任一条件不满足都不得标 not_applicable(不得借状态逃判)。
    #[test]
    fn a1_no_pending_work_requires_zero_spawn_and_terminal_gate() {
        let mut observation = StageObservation::new("coding", &ProviderName::ClaudeCode);
        observation.events.push(serde_json::json!({
            "event": {
                "type": "coding_session_state",
                "status": "waiting_for_human",
                "stage": "final_confirm",
            }
        }));
        // 未确认达终门。
        assert!(no_pending_work_reason(&observation, false).is_none());
        // 本格真起了进程。
        observation.observed_spawn_count = 1;
        assert!(no_pending_work_reason(&observation, true).is_none());
        // 中途门(非 final_confirm)不构成终门空转。
        observation.observed_spawn_count = 0;
        observation.events.clear();
        observation.events.push(serde_json::json!({
            "event": {
                "type": "coding_session_state",
                "status": "waiting_for_human",
                "stage": "code_review",
            }
        }));
        assert!(no_pending_work_reason(&observation, true).is_none());
        // 运行失败不得洗成空转。
        observation.events.clear();
        observation.events.push(serde_json::json!({
            "event": {
                "type": "coding_session_state",
                "status": "waiting_for_human",
                "stage": "final_confirm",
            }
        }));
        observation.run_failure = Some("coding attempt 终态 failed".to_string());
        assert!(no_pending_work_reason(&observation, true).is_none());
    }

    fn idle_cell() -> EvidenceCell {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut cell = EvidenceCell {
            provider: ProviderName::ClaudeCode,
            exact_version: "claude 2.1.283".to_string(),
            stage: "coding".to_string(),
            entrypoint: ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string(),
            fresh_or_resume: RESUME.to_string(),
            process_cwd: temp.path().to_path_buf(),
            target: temp.path().join("alpha"),
            audit_projection_digest: String::new(),
            frozen_projection_digest: String::new(),
            native_resume_confirmed_id: Some("native-7".to_string()),
            native_resume_reattached: false,
            requested_resume_id: Some("native-7".to_string()),
            argv_or_wire_capture_exists: false,
            approval_and_tool_events_exist: false,
            completed_product_artifact_exists: true,
            run_ref: "run-idle".to_string(),
            run_ref_is_unique_within_entrypoint: true,
            action: "coding_workspace_write".to_string(),
            role: "executor".to_string(),
            gateway_dialect: String::new(),
            wire_dialect: String::new(),
            native_session_id: "native-7".to_string(),
            workspace_session_id: "attempt-1".to_string(),
            argv: Vec::new(),
            capability_state: "not_applicable".to_string(),
            denied_reason: Some("无待恢复工作".to_string()),
            provider_pid: None,
            pid_unavailable_reason: Some("零新增 spawn".to_string()),
            provider_spawn_count: 0,
            session_projection_digest: String::new(),
            provider_events: Vec::new(),
            execution_origin: NO_PENDING_WORK_ORIGIN.to_string(),
            carried_source_evidence_root: None,
            carried_workspace_digest: String::new(),
            carried_aggregate_digest: String::new(),
            carried_capability_digest: String::new(),
            carried_policy_digest: String::new(),
        };
        cell.provider_events.push(serde_json::json!({
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": 4,
                "provider_start_audits_after": 4,
            }
        }));
        cell
    }

    /// A1:自证齐备的空转格通过;缺任一维即红。
    #[test]
    fn a1_not_applicable_evidence_is_falsifiable() {
        let cell = idle_cell();
        assert_eq!(not_applicable_evidence_error(&cell), None);

        let mut wrong_origin = cell.clone();
        wrong_origin.execution_origin = "full_chain".to_string();
        assert!(not_applicable_evidence_error(&wrong_origin).is_some());

        let mut spawned = cell.clone();
        spawned.provider_spawn_count = 1;
        assert!(not_applicable_evidence_error(&spawned).is_some());

        let mut id_mismatch = cell.clone();
        id_mismatch.native_resume_confirmed_id = Some("native-other".to_string());
        assert!(not_applicable_evidence_error(&id_mismatch).is_some());

        let mut no_reason = cell.clone();
        no_reason.denied_reason = Some("  ".to_string());
        assert!(not_applicable_evidence_error(&no_reason).is_some());

        let mut unproven = cell.clone();
        unproven.provider_events.clear();
        assert!(
            not_applicable_evidence_error(&unproven).is_some(),
            "无 zero_spawn_observation 不得放行"
        );

        let mut drifted = cell;
        drifted.provider_events = vec![serde_json::json!({
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": 4,
                "provider_start_audits_after": 5,
            }
        })];
        assert!(
            not_applicable_evidence_error(&drifted).is_some(),
            "审计前后不等=真 spawn,不得标 not_applicable"
        );
    }
}
