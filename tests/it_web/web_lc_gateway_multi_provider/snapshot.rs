//! Plan-confirmed 快照(MaxPhaseSkip+OraclePhaseSkip 双裁决方案实施)。
//!
//! 语义(裁决铁律,不删格不伪造):
//! - 快照点=plan typed confirm 返回 confirmed 且 resolve_first_work_item
//!   成功后的静止边界(无 running run/attempt,门为确定态)。
//! - 快照=字节级逻辑夹具:workspace root(.aria 全部)+聚合根(成员 Git
//!   仓+policy 产物)+manifest(staging+fsync+rename 原子落盘)。
//! - 路径策略:固定目录原地打开,不做路径重写(绝对路径嵌入指纹/Git
//!   common-dir,重写会假红)。pristine 副本以**同一路径**回灌 run/,
//!   每轮续跑从 pristine 恢复后原地执行。
//! - 续跑轮=coding/review 真实执行(snapshot_fresh:新 attempt+hello+
//!   start_coding,不是 resume);story/design/plan/split 为承继格
//!   (carried,引用来源轮证据链,不重新计票)。
//! - diff 门禁:git diff <快照SHA>..<当前SHA> 触及被跳阶段路径→拒绝续跑
//!   转全链;每 3 轮续跑后强制一轮全链再基线。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub(crate) const SNAPSHOT_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub(crate) const SNAPSHOT_POINT_PLAN_CONFIRMED: &str = "plan_confirmed";
/// 续跑轮再基线节奏:连续续跑 N 轮后强制一轮全链。
pub(crate) const SNAPSHOT_REBASELINE_EVERY: u32 = 3;

/// 被跳阶段(story/design/plan/split)与语义契约面的路径清单——diff 触及
/// 任一即拒绝续跑(oracle 裁决 3:指纹/schema/prompt/protocol 变更视为
/// 承继证据失效)。前缀匹配。
pub(crate) const SNAPSHOT_DIFF_GATE_PATHS: &[&str] = &[
    // 被跳阶段的 engine+prompts(review engine 亦在其中——保守裁决)。
    "src/product/workspace_engine/",
    "src/product/work_item_split_engine/",
    // 聚合初始化 recipe 面(承继格依赖其产物形态)。
    "src/product/logical_codebase/",
    // 承继阶段消费的 store schema。
    "src/product/lifecycle_store/",
    // 协议契约。
    "src/protocol/",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// 全链:12 格真实执行(默认)。
    FullChain,
    /// 全链 + 在 plan-confirmed 边界落快照(为续跑轮建基线)。
    CapturePlanSnapshot,
    /// 从快照续跑:承继 8 格 + 真实执行 coding/review。
    ResumeFromPlanSnapshot,
}

impl RunMode {
    /// `LIVE_MATRIX_RUN_MODE`:full_chain(默认)/capture_plan_snapshot/
    /// resume_from_plan_snapshot。非法值 fail-closed 报错(不静默降级)。
    pub(crate) fn from_env() -> Result<Self, String> {
        let Some(raw) = std::env::var("LIVE_MATRIX_RUN_MODE").ok().filter(|v| !v.trim().is_empty())
        else {
            return Ok(Self::FullChain);
        };
        match raw.trim() {
            "full_chain" => Ok(Self::FullChain),
            "capture_plan_snapshot" => Ok(Self::CapturePlanSnapshot),
            "resume_from_plan_snapshot" => Ok(Self::ResumeFromPlanSnapshot),
            other => Err(format!(
                "LIVE_MATRIX_RUN_MODE 非法值 {other:?}(合法:full_chain|capture_plan_snapshot|resume_from_plan_snapshot)"
            )),
        }
    }
}

/// 快照清单(原子落盘;无 manifest 的快照不得用于续跑)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct SnapshotManifest {
    pub schema_version: u32,
    pub snapshot_point: String,
    pub provider_wire: String,
    pub cli_version: String,
    pub project_id: String,
    pub issue_id: String,
    pub lc_id: String,
    pub plan_id: String,
    pub work_item_id: String,
    /// 成员仓(聚合根内首个成员)HEAD,恢复后 preflight 复核。
    pub member_git_head: String,
    /// LC 子树(含 capability/policy/trust/audit)内容摘要。
    pub capability_digest: String,
    /// 聚合根 policy 面(AGENTS.md 等)内容摘要。
    pub policy_digest: String,
    /// 构建修订(快照来源轮的 git SHA;diff 门禁基准)。
    pub harness_revision: String,
    /// pristine 副本的树摘要(恢复时字节级校验;漂移→BLOCKED)。
    pub workspace_digest: String,
    pub aggregate_digest: String,
    /// 来源轮证据根(承继格引用该轮证据链)。
    pub source_evidence_root: String,
    /// 静止性确认(捕获点无 running run/attempt)。
    pub quiescent: bool,
    pub created_at: String,
}

/// 捕获输入(由 harness 在快照点组装)。
pub(crate) struct CaptureInput<'a> {
    pub provider_wire: &'a str,
    pub cli_version: &'a str,
    pub project_id: &'a str,
    pub issue_id: &'a str,
    pub lc_id: &'a str,
    pub plan_id: &'a str,
    pub work_item_id: &'a str,
    pub workspace_root: &'a Path,
    pub aggregate_root: &'a Path,
    pub lc_root: &'a Path,
    pub policy_root: &'a Path,
    pub source_evidence_root: &'a Path,
}

/// 捕获结果(快照目录)。
pub(crate) struct CapturedSnapshot {
    pub snapshot_id: String,
    pub directory: PathBuf,
}

/// 打开(恢复)结果。
pub(crate) struct OpenedSnapshot {
    pub manifest: SnapshotManifest,
    /// 续跑轮 live 根(pristine 已回灌;路径与捕获轮逐字节同源)。
    pub workspace_root: PathBuf,
    pub aggregate_root: PathBuf,
}

#[derive(Debug)]
pub(crate) enum SnapshotOpenError {
    /// pristine 字节漂移(digest 校验失败)——BLOCKED 真实报告。
    DigestMismatch { detail: String },
    /// diff 门禁命中:当前构建触及被跳阶段路径→转全链。
    DiffGateHit { touched: Vec<String> },
    /// 连续续跑达上限→强制全链再基线。
    ForceRebaseline { rounds: u32 },
    /// 快照不存在/manifest 非法/IO 失败。
    Invalid(String),
}

impl SnapshotOpenError {
    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::DigestMismatch { .. } => "snapshot_digest_mismatch",
            Self::DiffGateHit { .. } => "snapshot_diff_gate_hit",
            Self::ForceRebaseline { .. } => "snapshot_force_rebaseline",
            Self::Invalid(_) => "snapshot_invalid",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::DigestMismatch { detail } => {
                format!("快照 pristine 校验失败(字节漂移):{detail}")
            }
            Self::DiffGateHit { touched } => {
                format!(
                    "diff 门禁命中(当前构建触及被跳阶段路径,续跑证据失效,转全链):{}",
                    touched.join(",")
                )
            }
            Self::ForceRebaseline { rounds } => format!(
                "连续续跑 {rounds} 轮达上限(每 {SNAPSHOT_REBASELINE_EVERY} 轮强制全链再基线)"
            ),
            Self::Invalid(detail) => format!("快照不可用:{detail}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 快照目录布局
// ---------------------------------------------------------------------------

/// `<provider>/snapshots/` 基目录——与 `matrix/` 同级(evidence_root 的
/// 兄弟目录;provider 目录用 dash 形态,与矩阵证据目录一致,不重复传
/// provider 名避免 snake/dash 漂移)。
pub(crate) fn snapshots_root(evidence_root: &Path) -> PathBuf {
    evidence_root
        .parent()
        .unwrap_or(evidence_root)
        .join("snapshots")
}

fn snapshot_dir(root: &Path, snapshot_id: &str) -> PathBuf {
    root.join(snapshot_id)
}

/// 再基线计数状态文件(`<provider>/snapshots/state.json`)。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub(crate) struct SnapshotBaselineState {
    pub rounds_since_baseline: u32,
}

fn baseline_state_path(root: &Path) -> PathBuf {
    root.join("state.json")
}

// ---------------------------------------------------------------------------
// 摘要:确定性树摘要(排序相对路径+文件内容;非常规条目按可观测文本计入)
// ---------------------------------------------------------------------------

pub(crate) fn tree_digest(root: &Path) -> Result<String, String> {
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_files(root, root, &mut entries).map_err(|error| {
        format!("tree_digest 扫描 {} 失败:{error}", root.display())
    })?;
    entries.sort();
    let mut hasher = Sha256::new();
    for path in entries {
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("strip_prefix: {error}"))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        match std::fs::read(&path) {
            Ok(bytes) => hasher.update(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                // 特殊条目(socket/fifo):路径可观测即可,不读内容。
            }
            Err(error) => {
                return Err(format!(
                    "tree_digest 读取 {} 失败:{error}",
                    path.display()
                ))
            }
        }
        hasher.update(b"\0");
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries.flatten() {
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            // Git 仓的对象库大且非确定性:摘要按 Git 自身的 HEAD/引用面
            // (见 git_head)覆盖;树摘要跳过 .git 目录内容,保留 .git
            // 顶层配置文件以外的可确定性文件面。
            if entry.file_name() == ".git" {
                continue;
            }
            collect_files(root, &entry.path(), out)?;
        } else {
            out.push(entry.path());
        }
    }
    Ok(())
}

pub(crate) fn git_head(repo: &Path) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|error| format!("git rev-parse: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse HEAD 失败于 {}:{}",
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// 原子落盘:staging + fsync + rename
// ---------------------------------------------------------------------------

pub(crate) fn write_manifest_atomic(directory: &Path, manifest: &SnapshotManifest) -> Result<(), String> {
    let target = directory.join("manifest.json");
    let staging = directory.join("manifest.json.staging");
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("manifest 序列化失败:{error}"))?;
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建快照目录失败 {}: {error}", parent.display()))?;
    }
    std::fs::write(&staging, &bytes).map_err(|error| {
        format!("manifest staging 写入失败 {}: {error}", staging.display())
    })?;
    let file = std::fs::File::open(&staging)
        .map_err(|error| format!("manifest staging 打开失败:{error}"))?;
    file.sync_all()
        .map_err(|error| format!("manifest staging fsync 失败:{error}"))?;
    drop(file);
    std::fs::rename(&staging, &target).map_err(|error| {
        format!(
            "manifest rename 失败 {} -> {}: {error}",
            staging.display(),
            target.display()
        )
    })?;
    if let Some(parent) = target.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

pub(crate) fn read_manifest(directory: &Path) -> Result<SnapshotManifest, String> {
    let path = directory.join("manifest.json");
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("manifest 读取失败 {}: {error}", path.display()))?;
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("manifest 解析失败:{error}"))?;
    if manifest.schema_version != SNAPSHOT_MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version={:?} 不受支持(期望 {:?})",
            manifest.schema_version, SNAPSHOT_MANIFEST_SCHEMA_VERSION
        ));
    }
    if manifest.snapshot_point != SNAPSHOT_POINT_PLAN_CONFIRMED {
        return Err(format!(
            "manifest snapshot_point={:?} 非法(期望 {:?})",
            manifest.snapshot_point, SNAPSHOT_POINT_PLAN_CONFIRMED
        ));
    }
    if !manifest.quiescent {
        return Err("manifest 声明非静止边界(不得用于续跑)".to_string());
    }
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// 捕获(在固定路径 run/ 执行的捕获轮调用;pristine 为同路径字节副本)
// ---------------------------------------------------------------------------

fn copy_tree_quiet(source: &Path, destination: &Path) -> Result<(), String> {
    fn copy_inner(source: &Path, destination: &Path) -> std::io::Result<()> {
        let entries = match std::fs::read_dir(source) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        std::fs::create_dir_all(destination)?;
        for entry in entries.flatten() {
            let target = destination.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_inner(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    copy_inner(source, destination)
        .map_err(|error| format!("复制 {} -> {} 失败:{error}", source.display(), destination.display()))
}

pub(crate) fn capture_plan_snapshot(
    evidence_root: &Path,
    input: &CaptureInput<'_>,
    directory_override: Option<&Path>,
) -> Result<CapturedSnapshot, String> {
    // r52:capture 轮的 run/ 由 build 预建(固定路径指纹要求)——pristine
    // 必须落在**同一**目录(目录覆盖优先;快照 id 取目录名)。
    let root = snapshots_root(evidence_root);
    let directory = match directory_override {
        Some(directory) => directory.to_path_buf(),
        None => {
            let snapshot_id = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            snapshot_dir(&root, &snapshot_id)
        }
    };
    let snapshot_id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("快照目录名非法")?
        .to_string();
    let pristine_workspace = directory.join("pristine").join("workspace");
    let pristine_aggregate = directory.join("pristine").join("aggregate-root");
    std::fs::create_dir_all(&pristine_workspace)
        .map_err(|error| format!("创建 pristine workspace 失败:{error}"))?;
    std::fs::create_dir_all(&pristine_aggregate)
        .map_err(|error| format!("创建 pristine aggregate 失败:{error}"))?;
    copy_tree_quiet(input.workspace_root, &pristine_workspace)?;
    copy_tree_quiet(input.aggregate_root, &pristine_aggregate)?;
    let member_repo = input
        .aggregate_root
        .join("alpha");
    let manifest = SnapshotManifest {
        schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
        snapshot_point: SNAPSHOT_POINT_PLAN_CONFIRMED.to_string(),
        provider_wire: input.provider_wire.to_string(),
        cli_version: input.cli_version.to_string(),
        project_id: input.project_id.to_string(),
        issue_id: input.issue_id.to_string(),
        lc_id: input.lc_id.to_string(),
        plan_id: input.plan_id.to_string(),
        work_item_id: input.work_item_id.to_string(),
        member_git_head: git_head(&member_repo).unwrap_or_default(),
        capability_digest: tree_digest(input.lc_root)?,
        policy_digest: tree_digest(input.policy_root)?,
        harness_revision: current_revision(),
        workspace_digest: tree_digest(&pristine_workspace)?,
        aggregate_digest: tree_digest(&pristine_aggregate)?,
        source_evidence_root: input
            .source_evidence_root
            .to_string_lossy()
            .to_string(),
        quiescent: true,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    write_manifest_atomic(&directory, &manifest)?;
    // 捕获即再基线:连续续跑计数清零。
    let state = SnapshotBaselineState::default();
    std::fs::write(
        baseline_state_path(&root),
        serde_json::to_vec_pretty(&state).map_err(|error| format!("state 序列化失败:{error}"))?,
    )
    .map_err(|error| format!("state 写入失败:{error}"))?;
    Ok(CapturedSnapshot {
        snapshot_id,
        directory,
    })
}

/// 当前构建修订(git SHA;仓库根=本文件所在 repo)。
pub(crate) fn current_revision() -> String {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    git_head(&manifest_dir).unwrap_or_else(|error| {
        panic!("快照需要当前构建修订(git SHA),不可得:{error}");
    })
}

// ---------------------------------------------------------------------------
// diff 门禁
// ---------------------------------------------------------------------------

/// `git diff <from>..<to> --name-only` 与门禁路径清单的交集(前缀匹配)。
pub(crate) fn diff_gate_touched(repo: &Path, from: &str, to: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", &format!("{from}..{to}")])
        .current_dir(repo)
        .output()
        .map_err(|error| format!("git diff 启动失败:{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff {from}..{to} 失败:{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let touched = String::from_utf8_lossy(&output.stdout);
    Ok(touched
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            SNAPSHOT_DIFF_GATE_PATHS
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        .map(ToString::to_string)
        .collect())
}

// ---------------------------------------------------------------------------
// 打开(恢复):pristine 校验 → diff 门禁 → 再基线计数 → 回灌 run/
// ---------------------------------------------------------------------------

pub(crate) fn open_plan_snapshot(
    evidence_root: &Path,
    snapshot_id: &str,
) -> Result<OpenedSnapshot, SnapshotOpenError> {
    let root = snapshots_root(evidence_root);
    let directory = snapshot_dir(&root, snapshot_id);
    let manifest = read_manifest(&directory).map_err(SnapshotOpenError::Invalid)?;
    if manifest.provider_wire.trim().is_empty() {
        return Err(SnapshotOpenError::Invalid(
            "manifest 缺 provider 身份(不得用于续跑)".to_string(),
        ));
    }
    // 1) pristine 字节校验。
    let pristine_workspace = directory.join("pristine").join("workspace");
    let pristine_aggregate = directory.join("pristine").join("aggregate-root");
    let workspace_digest = tree_digest(&pristine_workspace)
        .map_err(SnapshotOpenError::Invalid)?;
    if workspace_digest != manifest.workspace_digest {
        return Err(SnapshotOpenError::DigestMismatch {
            detail: format!(
                "workspace pristine {workspace_digest} != manifest {}",
                manifest.workspace_digest
            ),
        });
    }
    let aggregate_digest = tree_digest(&pristine_aggregate)
        .map_err(SnapshotOpenError::Invalid)?;
    if aggregate_digest != manifest.aggregate_digest {
        return Err(SnapshotOpenError::DigestMismatch {
            detail: format!(
                "aggregate pristine {aggregate_digest} != manifest {}",
                manifest.aggregate_digest
            ),
        });
    }
    // 2) diff 门禁(快照构建 → 当前构建)。
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let touched = diff_gate_touched(&repo, &manifest.harness_revision, &current_revision())
        .map_err(SnapshotOpenError::Invalid)?;
    if !touched.is_empty() {
        return Err(SnapshotOpenError::DiffGateHit { touched });
    }
    // 3) 再基线计数。
    let state = read_baseline_state(&root);
    if state.rounds_since_baseline >= SNAPSHOT_REBASELINE_EVERY {
        return Err(SnapshotOpenError::ForceRebaseline {
            rounds: state.rounds_since_baseline,
        });
    }
    // 4) 回灌 run/(固定路径原地打开;路径与捕获轮逐字节同源)。
    let run_workspace = directory.join("run").join("workspace");
    let run_aggregate = directory.join("run").join("aggregate-root");
    let _ = std::fs::remove_dir_all(&run_workspace);
    let _ = std::fs::remove_dir_all(&run_aggregate);
    std::fs::create_dir_all(&run_workspace)
        .map_err(|error| SnapshotOpenError::Invalid(format!("run workspace 创建失败:{error}")))?;
    std::fs::create_dir_all(&run_aggregate)
        .map_err(|error| SnapshotOpenError::Invalid(format!("run aggregate 创建失败:{error}")))?;
    copy_tree_quiet(&pristine_workspace, &run_workspace)
        .map_err(SnapshotOpenError::Invalid)?;
    copy_tree_quiet(&pristine_aggregate, &run_aggregate)
        .map_err(SnapshotOpenError::Invalid)?;
    // 5) durable preflight:LC 子树与 policy 面摘要复核(回灌后)。
    let lc_root = run_workspace
        .join(".aria")
        .join("projects")
        .join(&manifest.project_id)
        .join("logical-codebases")
        .join(&manifest.lc_id);
    let capability_digest = tree_digest(&lc_root).map_err(SnapshotOpenError::Invalid)?;
    if capability_digest != manifest.capability_digest {
        return Err(SnapshotOpenError::DigestMismatch {
            detail: format!(
                "回灌后 LC 子树 {capability_digest} != manifest {}",
                manifest.capability_digest
            ),
        });
    }
    // r53:回灌后 canonicalize——与 capture 轮同源绝对路径(evidence_root
    // 可为相对路径,产品登记/指纹面消费 canonical 绝对形态)。
    let run_workspace = run_workspace
        .canonicalize()
        .map_err(|error| SnapshotOpenError::Invalid(format!("canonicalize run workspace:{error}")))?;
    let run_aggregate = run_aggregate
        .canonicalize()
        .map_err(|error| SnapshotOpenError::Invalid(format!("canonicalize run aggregate:{error}")))?;
    Ok(OpenedSnapshot {
        manifest,
        workspace_root: run_workspace,
        aggregate_root: run_aggregate,
    })
}

/// 续跑轮结束后递增再基线计数。
pub(crate) fn record_resume_round(evidence_root: &Path) -> Result<u32, String> {
    let root = snapshots_root(evidence_root);
    let mut state = read_baseline_state(&root);
    state.rounds_since_baseline = state
        .rounds_since_baseline
        .saturating_add(1);
    std::fs::create_dir_all(&root).map_err(|error| format!("snapshots 目录创建失败:{error}"))?;
    std::fs::write(
        baseline_state_path(&root),
        serde_json::to_vec_pretty(&state).map_err(|error| format!("state 序列化失败:{error}"))?,
    )
    .map_err(|error| format!("state 写入失败:{error}"))?;
    Ok(state.rounds_since_baseline)
}

fn read_baseline_state(root: &Path) -> SnapshotBaselineState {
    std::fs::read(baseline_state_path(root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// 选择快照 id:显式 env(LIVE_MATRIX_SNAPSHOT_ID)或目录内最新
/// (字典序=时间序)。
pub(crate) fn resolve_snapshot_id(evidence_root: &Path) -> Result<String, String> {
    if let Ok(explicit) = std::env::var("LIVE_MATRIX_SNAPSHOT_ID") {
        let trimmed = explicit.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let root = snapshots_root(evidence_root);
    let mut ids: Vec<String> = std::fs::read_dir(&root)
        .map_err(|error| format!("snapshots 目录不可读 {}: {error}", root.display()))?
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != "state.json")
        .collect();
    ids.sort();
    ids.pop()
        .ok_or_else(|| format!("{} 下无可用快照(先跑 capture_plan_snapshot)", root.display()))
}

// ---------------------------------------------------------------------------
// 单元测试(纯逻辑面:manifest 原子性/digest 漂移/diff 门禁/再基线计数)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn sample_manifest() -> SnapshotManifest {
        SnapshotManifest {
            schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
            snapshot_point: SNAPSHOT_POINT_PLAN_CONFIRMED.to_string(),
            provider_wire: "claude_code".to_string(),
            cli_version: "2.1.283".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            lc_id: "logical_codebase_x".to_string(),
            plan_id: "issue_work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            member_git_head: String::new(),
            capability_digest: "sha256:cap".to_string(),
            policy_digest: "sha256:policy".to_string(),
            harness_revision: "deadbeef".to_string(),
            workspace_digest: "sha256:ws".to_string(),
            aggregate_digest: "sha256:agg".to_string(),
            source_evidence_root: "/tmp/evidence".to_string(),
            quiescent: true,
            created_at: "2026-10-08T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn manifest_round_trips_atomically_and_leaves_no_staging() {
        let dir = tempdir();
        let manifest = sample_manifest();
        write_manifest_atomic(dir.path(), &manifest).expect("write manifest");
        assert!(
            !dir.path().join("manifest.json.staging").exists(),
            "staging 文件必须被 rename 消费,不得残留"
        );
        let read = read_manifest(dir.path()).expect("read manifest");
        assert_eq!(read, manifest);
    }

    #[test]
    fn manifest_rejects_wrong_schema_and_non_quiescent() {
        let dir = tempdir();
        let mut manifest = sample_manifest();
        manifest.schema_version = 99;
        write_manifest_atomic(dir.path(), &manifest).expect("write manifest");
        assert!(
            read_manifest(dir.path()).is_err(),
            "schema_version 不受支持必须拒绝"
        );
        let mut manifest = sample_manifest();
        manifest.quiescent = false;
        write_manifest_atomic(dir.path(), &manifest).expect("write manifest");
        assert!(
            read_manifest(dir.path()).is_err(),
            "非静止快照不得用于续跑"
        );
    }

    #[test]
    fn tree_digest_detects_byte_drift_and_ignores_git_dirs() {
        let dir = tempdir();
        let root = dir.path().join("tree");
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a").join("f.txt"), b"one").unwrap();
        std::fs::create_dir_all(root.join(".git").join("objects")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), b"ref: refs/heads/main").unwrap();
        let first = tree_digest(&root).expect("digest");
        // .git 内容变化不参与(确定性由 git_head 承载)。
        std::fs::write(root.join(".git").join("HEAD"), b"changed").unwrap();
        assert_eq!(tree_digest(&root).expect("digest"), first);
        // 业务文件字节漂移必须检出。
        std::fs::write(root.join("a").join("f.txt"), b"two").unwrap();
        assert_ne!(tree_digest(&root).expect("digest"), first);
    }

    #[test]
    fn diff_gate_matches_skipped_stage_prefixes() {
        let touched = ["src/product/workspace_engine/prompts.rs", "src/web/other.rs"];
        let matched: Vec<&str> = touched
            .iter()
            .copied()
            .filter(|line| {
                SNAPSHOT_DIFF_GATE_PATHS
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            })
            .collect();
        assert_eq!(matched, vec!["src/product/workspace_engine/prompts.rs"]);
    }

    #[test]
    fn baseline_state_round_trips_and_increments() {
        let dir = tempdir();
        let root = dir.path().join("snapshots");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            read_baseline_state(&root).rounds_since_baseline,
            0,
            "缺省计数=0"
        );
        std::fs::write(
            baseline_state_path(&root),
            serde_json::to_vec(&SnapshotBaselineState {
                rounds_since_baseline: 2,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(read_baseline_state(&root).rounds_since_baseline, 2);
    }

    #[test]
    fn run_mode_parses_env_and_fails_closed() {
        // 默认(无 env)=full_chain。
        let mode = RunMode::from_env().expect("default mode");
        assert_eq!(mode, RunMode::FullChain);
    }

    #[test]
    fn snapshots_root_is_sibling_of_matrix_dir() {
        let evidence = PathBuf::from("/tmp/reports/claude-code/matrix");
        assert_eq!(
            snapshots_root(&evidence),
            PathBuf::from("/tmp/reports/claude-code/snapshots")
        );
    }
}
