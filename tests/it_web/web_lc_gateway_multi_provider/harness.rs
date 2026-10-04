//! Task 10a:四家 LC 五阶段真实矩阵 harness(test-only 基建)。
//!
//! 冻结契约(计划 Task 10 Interfaces,454 行):
//! - `LiveLcGatewayHarness::run_provider_matrix(provider: ProviderName,
//!   evidence_root: &Path) -> Result<LiveMatrixEvidence, LiveMatrixFailure>`;
//! - `LiveMatrixEvidence`(Task 10 唯一定义)与 `EvidenceCell`,格键 =
//!   `provider/exact_version/stage/entrypoint/fresh_or_resume`;
//! - 两入口分列:`workspace_streaming_plan/split`(WS streaming 栈主入口,
//!   Story/Design/Plan/Coding/Review 的会话驱动均属该栈;Plan 的
//!   `start_work_item_plan_author` caller 链为其代表)与 `split_sync`
//!   (`WorkItemSplitEngine::generate/generate_revision` 所代表的 gateway
//!   sync bridge 对照栈)。
//!
//! 阶段顺序固定(Step 3):产品创建/真实登记/trust → 固定 Claude recipe
//! (真实聚合初始化)→ #8 最终 policy artifact/receipt → 真实索引 ready →
//! Story、Design、Plan、Coding、Review,各阶段 fresh/resume 分格。
//! 产品确认一律走真实 HTTP/WS gate;#13 Fake 默认只经既有 provider_select,
//! 本 harness 不直写状态或能力。复用 `web_lc_operations_api` 的 HTTP 形态
//! 与真实 git fixture 建法,不复用 Noop/Fake 驱动当真实证据。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ProviderStartAudit};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::logical_codebase::policy::{
    AggregatePolicyArtifactStore, PolicyTarget, SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::provider_gateway::{
    ProviderLaunchAuditContext, ProviderRef, SessionLaunchRequest,
};
use cadence_aria::product::logical_codebase::store::LogicalCodebaseStore;
use cadence_aria::product::logical_codebase::types::{
    CodebaseMemberRecord, RepositoryCheckoutRecord,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tower::ServiceExt;

/// 真实现场开关:缺失时 `#[ignore]` 测试必须失败而非静默 return 成功。
pub(crate) const LC_GATEWAY_E2E_SWITCH: &str = "LC_GATEWAY_E2E";

/// Plan/split 的 WS streaming 主入口(`start_work_item_plan_author` caller 链)。
pub(crate) const ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT: &str = "workspace_streaming_plan/split";
/// Plan/split 的 sync 对照入口(`WorkItemSplitEngine::generate/generate_revision`)。
pub(crate) const ENTRYPOINT_SPLIT_SYNC: &str = "split_sync";

/// 五阶段顺序固定:Story、Design、Plan、Coding、Review。
pub(crate) const STAGE_ORDER: [&str; 5] = ["story", "design", "plan", "coding", "review"];

/// fresh/resume 两格(键的最后一维)。
pub(crate) const FRESH: &str = "fresh";
pub(crate) const RESUME: &str = "resume";

/// 会话级真实阶段超时(真实现场 CLI 慢;可用环境变量放宽)。
const STAGE_TIMEOUT_ENV: &str = "LC_GATEWAY_E2E_STAGE_TIMEOUT_SECS";
const DEFAULT_STAGE_TIMEOUT_SECS: u64 = 1800;
/// 聚合初始化(root recipe 五步四命令)整体超时。
const INIT_TIMEOUT_ENV: &str = "LC_GATEWAY_E2E_INIT_TIMEOUT_SECS";
const DEFAULT_INIT_TIMEOUT_SECS: u64 = 7200;
/// provider health 就绪等待(真实 CLI 探测刷新)。
const HEALTH_TIMEOUT_ENV: &str = "LC_GATEWAY_E2E_HEALTH_TIMEOUT_SECS";
const DEFAULT_HEALTH_TIMEOUT_SECS: u64 = 300;

const PROJECT_ID: &str = "project_0001";

/// 单格证据(键 + 断言组字段 + 每格证据形态要素)。
///
/// 字段名与计划 Step 1 断言组(459-468 行)逐字对应;`cell.json` 落盘时
/// 敏感 token/API key/home 无关 trust 条目不落盘。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EvidenceCell {
    // ---- 键(provider/exact_version/stage/entrypoint/fresh_or_resume) ----
    pub provider: ProviderName,
    pub exact_version: String,
    pub stage: String,
    pub entrypoint: String,
    pub fresh_or_resume: String,
    // ---- 断言组(459-468)字段 ----
    pub process_cwd: PathBuf,
    pub target: PathBuf,
    pub audit_projection_digest: String,
    pub frozen_projection_digest: String,
    pub native_resume_confirmed_id: Option<String>,
    pub requested_resume_id: Option<String>,
    pub argv_or_wire_capture_exists: bool,
    pub approval_and_tool_events_exist: bool,
    pub completed_product_artifact_exists: bool,
    pub run_ref: String,
    pub run_ref_is_unique_within_entrypoint: bool,
    // ---- 每格证据形态(cell.json 全要素) ----
    pub action: String,
    pub role: String,
    pub gateway_dialect: String,
    pub wire_dialect: String,
    pub native_session_id: String,
    pub workspace_session_id: String,
    pub argv: Vec<String>,
    pub capability_state: String,
    pub denied_reason: Option<String>,
    // ---- F3/F5:PID/时间线/spawn 计数(485 行可追溯要素) ----
    /// provider 子进程 PID(取自 stream log 文件名 `{program}-{pid}-{stream}.log`)。
    /// 入口未提供 stream log 目录时为 `None`,且必须携带
    /// `pid_unavailable_reason`(如实标注不可达,不伪造)。
    pub provider_pid: Option<String>,
    pub pid_unavailable_reason: Option<String>,
    /// 本格观测到的 provider spawn 计数(split_sync resume 恒 0)。
    pub provider_spawn_count: u64,
    /// 带 ts 封包的事件载荷(provider-events.jsonl 全量来源)。
    pub provider_events: Vec<Value>,
}

impl EvidenceCell {
    /// 证据结构校验:缺字段/不一致记录必须拒绝(fail-closed,不以删格缩小验收)。
    ///
    /// 校验语义与 Step 1 断言组同源:exact version 非空、argv/wire 捕获存在
    /// (空 argv 占位不算真实 wire)、approval/tool 事件存在、完成产物存在、
    /// audit 摘要==冻结摘要、原生恢复确认==请求 id、cwd==canonical root、
    /// target==成员 worktree、entrypoint 枚举合法、run_ref 在 entrypoint 内
    /// 唯一、provider 与所选一致、stage 在五阶段枚举内。
    pub(crate) fn validate_against(
        &self,
        expected_provider: &ProviderName,
        canonical_root: &Path,
        member_worktree: &Path,
    ) -> Result<(), String> {
        if self.provider != *expected_provider {
            return Err(format!(
                "provider 不一致:格 {:?} != 所选 {expected_provider:?}",
                self.provider
            ));
        }
        if !STAGE_ORDER.contains(&self.stage.as_str()) {
            return Err(format!("stage 不在五阶段枚举内:{}", self.stage));
        }
        if self.entrypoint != ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT
            && self.entrypoint != ENTRYPOINT_SPLIT_SYNC
        {
            return Err(format!("entrypoint 不在两入口枚举内:{}", self.entrypoint));
        }
        if self.fresh_or_resume != FRESH && self.fresh_or_resume != RESUME {
            return Err(format!("fresh_or_resume 非法:{}", self.fresh_or_resume));
        }
        if self.process_cwd != canonical_root {
            return Err(format!(
                "进程 cwd 不是 canonical root:{:?} != {:?}",
                self.process_cwd, canonical_root
            ));
        }
        if self.target != member_worktree {
            return Err(format!(
                "target 不是成员 worktree:{:?} != {:?}",
                self.target, member_worktree
            ));
        }
        if self.exact_version.trim().is_empty() {
            return Err("exact_version 为空(缺 exact CLI 版本证据)".to_string());
        }
        if self.audit_projection_digest.trim().is_empty() {
            return Err("audit 投影摘要为空(缺 lc_projection 启动审计)".to_string());
        }
        if self.frozen_projection_digest.trim().is_empty() {
            return Err("冻结投影摘要为空(缺 capability projection)".to_string());
        }
        if self.audit_projection_digest != self.frozen_projection_digest {
            return Err(format!(
                "audit 投影摘要 != 冻结投影摘要:{} != {}",
                self.audit_projection_digest, self.frozen_projection_digest
            ));
        }
        let native_pair = (
            self.fresh_or_resume.as_str(),
            self.requested_resume_id.as_deref(),
            self.native_resume_confirmed_id.as_deref(),
        );
        match native_pair {
            (fresh, None, None) if fresh == FRESH => {}
            (resume, Some(requested), Some(confirmed))
                if resume == RESUME && requested == confirmed => {}
            _ => {
                return Err(format!(
                    "原生恢复确认 != 请求:{} 格请求 {:?} 确认 {:?}",
                    self.fresh_or_resume, self.requested_resume_id, self.native_resume_confirmed_id
                ));
            }
        }
        if !self.argv_or_wire_capture_exists {
            return Err("argv/wire 捕获缺失".to_string());
        }
        if self.argv.is_empty() {
            return Err("argv 载荷为空(空 argv 占位不算真实 wire)".to_string());
        }
        if !self.approval_and_tool_events_exist {
            return Err("approval/tool 事件缺失".to_string());
        }
        if !self.completed_product_artifact_exists {
            return Err("完成产物缺失".to_string());
        }
        if self.run_ref.trim().is_empty() {
            return Err("run_ref 为空".to_string());
        }
        if !self.run_ref_is_unique_within_entrypoint {
            return Err("run_ref 在 entrypoint 内重复".to_string());
        }
        // F3:PID 可追溯——要么有真实 PID,要么有明确的不可达说明;两者皆缺
        // 即结构不完整(不以缺记录放行)。
        if self.provider_pid.is_none()
            && self
                .pid_unavailable_reason
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err("缺 provider PID 且无不可达说明(时间线不可追溯)".to_string());
        }
        // F1:事件载荷必须落盘可核对(approval_and_tool_events 的磁盘证据)。
        if self.provider_events.is_empty() {
            return Err("缺事件载荷(provider-events.jsonl 无可核对事件)".to_string());
        }
        // F5:split_sync 无 native session contract → resume 恒零 spawn;
        // 计数>0 即语义漂移,拒绝。
        if self.entrypoint == ENTRYPOINT_SPLIT_SYNC
            && self.fresh_or_resume == RESUME
            && self.provider_spawn_count > 0
        {
            return Err(format!(
                "split_sync resume 必须零 spawn,实测 {}",
                self.provider_spawn_count
            ));
        }
        Ok(())
    }

    /// `cell.json` 形态(键 + 断言组字段 + 全要素;敏感项不落盘)。
    pub(crate) fn to_cell_json(&self) -> Value {
        json!({
            "provider": self.provider,
            "exact_version": self.exact_version,
            "stage": self.stage,
            "entrypoint": self.entrypoint,
            "fresh_or_resume": self.fresh_or_resume,
            "process_cwd": self.process_cwd,
            "target": self.target,
            "audit_projection_digest": self.audit_projection_digest,
            "frozen_projection_digest": self.frozen_projection_digest,
            "native_resume_confirmed_id": self.native_resume_confirmed_id,
            "requested_resume_id": self.requested_resume_id,
            "argv_or_wire_capture_exists": self.argv_or_wire_capture_exists,
            "approval_and_tool_events_exist": self.approval_and_tool_events_exist,
            "completed_product_artifact_exists": self.completed_product_artifact_exists,
            "run_ref": self.run_ref,
            "run_ref_is_unique_within_entrypoint": self.run_ref_is_unique_within_entrypoint,
            "action": self.action,
            "role": self.role,
            "gateway_dialect": self.gateway_dialect,
            "wire_dialect": self.wire_dialect,
            "native_session_id": self.native_session_id,
            "workspace_session_id": self.workspace_session_id,
            "argv": self.argv,
            "capability_state": self.capability_state,
            "denied_reason": self.denied_reason,
            "provider_pid": self.provider_pid,
            "pid_unavailable_reason": self.pid_unavailable_reason,
            "provider_spawn_count": self.provider_spawn_count,
            "timeline": self.timeline_summary(),
        })
    }

    /// F3:时间线摘要(首/末事件 ts;provider-events.jsonl 为全量)。
    fn timeline_summary(&self) -> Value {
        let first = self
            .provider_events
            .first()
            .and_then(|event| event.get("ts").and_then(Value::as_str).map(str::to_string));
        let last = self
            .provider_events
            .last()
            .and_then(|event| event.get("ts").and_then(Value::as_str).map(str::to_string));
        json!({
            "first_event_ts": first,
            "last_event_ts": last,
            "event_count": self.provider_events.len(),
        })
    }
}

/// 一格被证据结构校验拒绝时的稳定记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceCellRejection {
    pub stage: String,
    pub entrypoint: String,
    pub fresh_or_resume: String,
    pub reason: String,
}

/// 四家五阶段真实矩阵证据(Task 10 唯一定义)。
#[derive(Debug, Clone)]
pub(crate) struct LiveMatrixEvidence {
    pub(crate) provider: ProviderName,
    pub(crate) canonical_root: PathBuf,
    pub(crate) member_worktree: PathBuf,
    pub(crate) cells: Vec<EvidenceCell>,
    pub(crate) rejections: Vec<EvidenceCellRejection>,
}

impl LiveMatrixEvidence {
    pub(crate) fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub(crate) fn member_worktree(&self) -> &Path {
        &self.member_worktree
    }

    /// 全部格子(Confirmed 与未 Confirmed 分列,不合并隐藏)。
    pub(crate) fn cells(&self) -> &[EvidenceCell] {
        &self.cells
    }

    /// 成功支持格:具备全部断言要素的 Confirmed 格。
    pub(crate) fn confirmed_cells(&self) -> Vec<&EvidenceCell> {
        self.cells
            .iter()
            .filter(|cell| cell.capability_state == "confirmed")
            .collect()
    }

    /// 缺证据格:保持 Unknown/Denied + reason,不以删格缩小验收范围。
    pub(crate) fn unconfirmed_cells(&self) -> Vec<&EvidenceCell> {
        self.cells
            .iter()
            .filter(|cell| cell.capability_state != "confirmed")
            .collect()
    }
}

/// harness 自身无法尝试矩阵(环境/前置失败)时的失败记录;
/// 阶段内失败不进入此类型,一律落格为 Unknown/Denied + reason。
#[derive(Debug, Clone)]
pub(crate) struct LiveMatrixFailure {
    pub(crate) reason_code: String,
    pub(crate) message: String,
    pub(crate) stage: Option<String>,
}

fn matrix_failure(reason_code: &str, message: String, stage: Option<&str>) -> LiveMatrixFailure {
    LiveMatrixFailure {
        reason_code: reason_code.to_string(),
        message,
        stage: stage.map(str::to_string),
    }
}

/// 四家五阶段真实矩阵 harness。
pub(crate) struct LiveLcGatewayHarness;

impl LiveLcGatewayHarness {
    pub(crate) fn new() -> Self {
        Self
    }

    /// 对单个 provider 执行五阶段(Story→Design→Plan→Coding→Review)×
    /// fresh/resume 真实矩阵,逐格落证据到 `evidence_root`。
    ///
    /// 阶段顺序与两入口分列冻结(计划 Step 3):Plan 的
    /// `workspace_streaming_plan/split`(WS begin handle→bind sink→start→
    /// parse/complete)与 `split_sync`(sync bridge;无 native session
    /// contract 时 resume 记 Unknown + 零 spawn)各自独立 run_ref 与证据。
    /// 阶段内失败不中止矩阵:落格 Unknown/Denied + reason,不删格。
    pub(crate) async fn run_provider_matrix(
        &self,
        provider: ProviderName,
        evidence_root: &Path,
    ) -> Result<LiveMatrixEvidence, LiveMatrixFailure> {
        let mut env = MatrixEnvironment::build(provider.clone(), evidence_root).await?;

        let mut cells = Vec::new();
        // 五阶段顺序固定:Story、Design、Plan、Coding、Review。
        cells.extend(
            env.run_workspace_entity_stage(
                "story",
                &format!(
                    "/api/projects/{PROJECT_ID}/issues/{}/story-specs:generate",
                    env.issue_id
                ),
                json!({
                    "title": "矩阵 story:会话过期提示",
                    "author_provider": env.provider_wire,
                    "reviewer_provider": env.provider_wire,
                    "review_rounds": 0,
                    "superpowers_enabled": false,
                    "openspec_enabled": true
                }),
                "story_specs",
            )
            .await,
        );
        cells.extend(
            env.run_workspace_entity_stage(
                "design",
                &format!(
                    "/api/projects/{PROJECT_ID}/issues/{}/design-specs:generate",
                    env.issue_id
                ),
                json!({
                    "title": "矩阵 design:会话过期后端设计",
                    "story_spec_ids": env.story_spec_ids(),
                    "author_provider": env.provider_wire,
                    "reviewer_provider": env.provider_wire,
                    "review_rounds": 0,
                    "superpowers_enabled": false,
                    "openspec_enabled": true
                }),
                "design_specs",
            )
            .await,
        );
        // Plan:workspace streaming 主入口(start_work_item_plan_author caller 链)。
        cells.extend(env.run_plan_streaming_stage().await);
        // Plan:split_sync 对照(gateway sync bridge)。
        cells.extend(env.run_split_sync_stage().await);
        // Coding(依赖 Plan 确认后的 work item;失败落格)。
        cells.extend(env.run_coding_stage().await);
        // Review:reviewer 角色经 streaming 栈的真实评审会话。
        cells.extend(env.run_review_stage().await);

        // run_ref 在 entrypoint 内唯一:跨格回填(空 run_ref 不参与判重)。
        let mut seen_per_entrypoint: BTreeMap<(String, String), usize> = BTreeMap::new();
        for cell in &cells {
            if cell.run_ref.trim().is_empty() {
                continue;
            }
            *seen_per_entrypoint
                .entry((cell.entrypoint.clone(), cell.run_ref.clone()))
                .or_default() += 1;
        }
        for cell in &mut cells {
            cell.run_ref_is_unique_within_entrypoint = cell.run_ref.trim().is_empty()
                || seen_per_entrypoint
                    .get(&(cell.entrypoint.clone(), cell.run_ref.clone()))
                    .is_none_or(|count| *count == 1);
        }

        // 逐格结构校验 + 证据落盘(形态全要素,485 行)。
        std::fs::create_dir_all(&env.evidence_root).map_err(|error| {
            matrix_failure(
                "evidence_root_unwritable",
                format!(
                    "创建证据根目录失败 {}/: {error}",
                    env.evidence_root.display()
                ),
                None,
            )
        })?;
        let mut rejections = Vec::new();
        for index in 0..cells.len() {
            let verdict = cells[index].validate_against(
                &env.provider,
                &env.canonical_root,
                &env.member_worktree,
            );
            if let Err(reason) = verdict {
                rejections.push(EvidenceCellRejection {
                    stage: cells[index].stage.clone(),
                    entrypoint: cells[index].entrypoint.clone(),
                    fresh_or_resume: cells[index].fresh_or_resume.clone(),
                    reason: reason.clone(),
                });
                // 结构不完整的 Confirmed 格降级为 denied(不以删格缩小验收)。
                cells[index].capability_state = "denied".to_string();
                cells[index].denied_reason =
                    cells[index].denied_reason.take().or_else(|| Some(reason));
            }
            // F7:证据落盘失败不静默——该格降级 denied(reason=证据落盘失败),
            // 不出现绿格+空证据目录。
            if let Err(io_reason) = env.write_cell_evidence(&cells[index]) {
                rejections.push(EvidenceCellRejection {
                    stage: cells[index].stage.clone(),
                    entrypoint: cells[index].entrypoint.clone(),
                    fresh_or_resume: cells[index].fresh_or_resume.clone(),
                    reason: io_reason.clone(),
                });
                cells[index].capability_state = "denied".to_string();
                cells[index].denied_reason = Some(io_reason);
            }
        }

        Ok(LiveMatrixEvidence {
            provider,
            canonical_root: env.canonical_root.clone(),
            member_worktree: env.member_worktree.clone(),
            cells,
            rejections,
        })
    }
}

impl Default for LiveLcGatewayHarness {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 环境搭建:真实 git fixture + 真实 runtime app + HTTP/WS 驱动。
// ---------------------------------------------------------------------------

struct MatrixEnvironment {
    provider: ProviderName,
    provider_wire: String,
    /// 非 Git 隔离 root(store/服务器工作区;未经审计不覆盖用户文件)。
    /// `Option`:失败诊断时 `keep()` 保留 tempdir,成功路径随 drop 清理。
    _root: Option<TempDir>,
    /// 真实聚合根(canonical root;两成员 git 仓)。
    _aggregate_root: Option<TempDir>,
    app: axum::Router,
    _server: tokio::task::JoinHandle<()>,
    ws_addr: SocketAddr,
    app_paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    gateway_factory: Arc<LogicalCodebaseGatewayFactory>,
    lc_id: String,
    issue_id: String,
    canonical_root: PathBuf,
    member_worktree: PathBuf,
    member_physical_repo_id: String,
    member_logical_id: String,
    member_checkout_id: String,
    story_spec_id: Option<String>,
    design_spec_id: Option<String>,
    work_item_id: Option<String>,
    prior_entity_session_id: Option<String>,
    prior_plan_session_id: Option<String>,
    prior_coding_session_id: Option<String>,
    prior_review_session_id: Option<String>,
    evidence_root: PathBuf,
    stage_timeout: Duration,
}

impl MatrixEnvironment {
    async fn build(
        provider: ProviderName,
        evidence_root: &Path,
    ) -> Result<Self, LiveMatrixFailure> {
        // 1) 真实模式门:fake registry/test provider 下不允许冒充真实现场。
        let root = TempDir::new().expect("workspace root");
        let runtime = WebRuntime::new_real(root.path().to_path_buf()).map_err(|error| {
            matrix_failure("real_runtime_unavailable", format!("{error:?}"), None)
        })?;
        let state = WebAppState::with_events(root.path().to_path_buf(), runtime, EventHub::new());
        if state.test_provider_enabled {
            return Err(matrix_failure(
                "real_mode_required",
                "ARIA_PROVIDER_MODE=fake 或 test provider 生效:真实矩阵要求真实 provider registry"
                    .to_string(),
                None,
            ));
        }
        // 生产装配的 gateway factory(与 app 同一 registry/gate/sync bridge)。
        let gateway_factory = state
            .logical_gateway_factory
            .clone()
            .expect("生产 logical gateway factory");

        // 2) 真实 git fixture(复用 web_lc_operations_api 建法:两成员仓+提交)。
        let aggregate_root = TempDir::new().expect("aggregate root");
        let member_a = aggregate_root.path().join("alpha");
        let member_b = aggregate_root.path().join("beta");
        git_repo_at(&member_a);
        git_repo_at(&member_b);
        // 真实产品语义:成员 checkout 缺 `.claude/rules/language.md` 时
        // missing_member_rules 任何 phase 阻断(Task 3 收紧)。与 lc-root
        // 旧 E2E 的真实成员仓形态对齐,git fixture 播种成员规则材料
        //(环境材料补齐,非产品绕过;root 侧规则由 #8 发布链产出)。
        seed_member_rule_material(&member_a, "alpha");
        seed_member_rule_material(&member_b, "beta");
        commit(
            &member_a,
            "pub fn cross_repo_greeting() -> &'static str { \"alpha\" }",
        );
        commit(
            &member_b,
            "pub fn cross_repo_greeting() -> &'static str { \"beta\" }",
        );

        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let app = build_web_router(state);
        // serve 消费一份 clone(共享同一 WebAppState);HTTP 走 oneshot。
        let http_app = app.clone();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ws listener");
        let ws_addr = listener.local_addr().expect("ws addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve matrix app");
        });

        let provider_wire = serde_json::to_value(&provider)
            .expect("provider serde")
            .as_str()
            .expect("provider wire name")
            .to_string();

        let mut env = Self {
            provider,
            provider_wire,
            _root: Some(root),
            _aggregate_root: Some(aggregate_root),
            app: http_app,
            _server: server,
            ws_addr,
            app_paths,
            lifecycle,
            gateway_factory,
            lc_id: String::new(),
            issue_id: String::new(),
            canonical_root: PathBuf::new(),
            member_worktree: PathBuf::new(),
            member_physical_repo_id: String::new(),
            member_logical_id: String::new(),
            member_checkout_id: String::new(),
            story_spec_id: None,
            design_spec_id: None,
            work_item_id: None,
            prior_entity_session_id: None,
            prior_plan_session_id: None,
            prior_coding_session_id: None,
            prior_review_session_id: None,
            evidence_root: evidence_root.to_path_buf(),
            stage_timeout: Duration::from_secs(env_timeout_secs(
                STAGE_TIMEOUT_ENV,
                DEFAULT_STAGE_TIMEOUT_SECS,
            )),
        };

        // 2.5) provider health 就绪等待:new_real 的 ProviderHealthService
        // 需完成探测刷新,否则 spawn 复验报 provider_gateway_unavailable
        //(health state degraded)。有界超时,超时=BLOCKED 真实报告。
        env.wait_for_provider_health().await?;
        // 3) 产品创建 + 真实登记(与 LcOperationsFixture 同一 HTTP 形态)。
        env.create_project_and_lc().await?;
        env.register_members().await?;

        // 4) 固定 Claude recipe:真实聚合初始化(root recipe 五步四命令,
        //    trust/#8 policy artifact/receipt/预算门由生产链自持)。
        env.run_real_aggregate_initialization().await?;
        env.wait_for_index_ready().await?;

        // 5) canonical root/target 定位 + 成员 issue。
        env.resolve_member_target().await?;
        env.create_issue().await?;
        Ok(env)
    }

    fn workspace_root_path(&self) -> &Path {
        self._root.as_ref().expect("workspace root").path()
    }

    fn aggregate_root_path(&self) -> &Path {
        self._aggregate_root
            .as_ref()
            .expect("aggregate root")
            .path()
    }

    /// fix 轮 3:失败诊断抄录 + tempdir 保留。把该 LC 的 root recipe
    /// receipts(逐命令 observed_changes/verdict)与 bootstrap GET 投影
    /// 抄录到 `evidence_root/diagnostics/`,并 `keep()` 两个 tempdir
    /// (成功路径照旧随 drop 清理);保留路径写入 failure.message。
    async fn fail_with_diagnostics(
        &mut self,
        mut failure: LiveMatrixFailure,
        operation_id: Option<&str>,
    ) -> LiveMatrixFailure {
        let diagnostics_dir = self.evidence_root.join("diagnostics");
        let _ = std::fs::create_dir_all(&diagnostics_dir);
        // 1) root recipe receipts 目录整体抄录(拒绝也 durable 保留证据)。
        let receipts_src = self
            .app_paths
            .logical_codebases_root(PROJECT_ID)
            .join(&self.lc_id)
            .join("aggregate-recipe-receipts");
        let receipts_dst = diagnostics_dir.join("aggregate-recipe-receipts");
        let _ = copy_tree(&receipts_src, &receipts_dst);
        // 2) bootstrap GET 投影快照(材料/planning 状态)。
        let bootstrap_uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/bootstrap",
            self.lc_id
        );
        let (bootstrap_status, bootstrap_body) =
            request_json(&self.app, Method::GET, &bootstrap_uri, json!({})).await;
        let _ = std::fs::write(
            diagnostics_dir.join("bootstrap.json"),
            serde_json::to_vec_pretty(&json!({
                "http_status": bootstrap_status.as_u16(),
                "body": bootstrap_body,
            }))
            .unwrap_or_default(),
        );
        // 3) tempdir 保留(keep 后不再随 drop 删除)。
        let workspace_before = self.workspace_root_path().display().to_string();
        let workspace_kept = self
            ._root
            .take()
            .map(|temp| temp.keep())
            .map(|path| path.display().to_string());
        let aggregate_kept = self
            ._aggregate_root
            .take()
            .map(|temp| temp.keep())
            .map(|path| path.display().to_string());
        let _ = std::fs::write(
            diagnostics_dir.join("paths.json"),
            serde_json::to_vec_pretty(&json!({
                "operation_id": operation_id,
                "reason_code": failure.reason_code,
                "workspace_root_before_keep": workspace_before,
                "workspace_root_kept": workspace_kept,
                "aggregate_root_kept": aggregate_kept,
            }))
            .unwrap_or_default(),
        );
        failure.message = format!(
            "{} [诊断已保留:{},workspace/aggregate_root 见 paths.json]",
            failure.message,
            diagnostics_dir.display()
        );
        failure
    }

    /// 轮询真实 provider health 直到所选 provider available(先触发一次
    /// 主动 recheck 刷新,再读 status;有界超时,超时按 BLOCKED 报告)。
    async fn wait_for_provider_health(&mut self) -> Result<(), LiveMatrixFailure> {
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(env_timeout_secs(
                HEALTH_TIMEOUT_ENV,
                DEFAULT_HEALTH_TIMEOUT_SECS,
            ));
        loop {
            // 主动刷新一次(探测真实 CLI --version),随后读状态。
            let _ =
                request_json(&self.app, Method::POST, "/api/providers/recheck", json!({})).await;
            let (status, body) =
                request_json(&self.app, Method::GET, "/api/providers/status", json!({})).await;
            let provider_ready = status.is_success()
                && body["state_status"] == "ready"
                && body["providers"].as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry["provider"] == self.provider_wire && entry["available"] == true
                    })
                });
            if provider_ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let failure = matrix_failure(
                    "provider_health_not_ready",
                    format!(
                        "provider health 未就绪({}):{} (环境不可运行须报告 BLOCKED)",
                        self.provider_wire, body
                    ),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn create_project_and_lc(&mut self) -> Result<(), LiveMatrixFailure> {
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            "/api/projects",
            json!({"name":"LC gateway matrix","description":null}),
        )
        .await;
        if let Err(failure) = expect_ok(status, &body, "创建 project", None) {
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/logical-codebases"),
            json!({"name":"Matrix","aggregate_root": self.aggregate_root_path()}),
        )
        .await;
        if let Err(failure) = expect_ok(status, &body, "创建逻辑代码库", None) {
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        let Some(lc_id) = body["id"].as_str() else {
            let failure = matrix_failure("lc_id_missing", format!("{body}"), None);
            return Err(self.fail_with_diagnostics(failure, None).await);
        };
        self.lc_id = lc_id.to_string();
        Ok(())
    }

    async fn register_members(&mut self) -> Result<(), LiveMatrixFailure> {
        let aggregate_root = self.aggregate_root_path().to_path_buf();
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/registrations/preflight",
                self.lc_id
            ),
            json!({"aggregate_root": aggregate_root, "candidate_paths": [], "auto_discover": true}),
        )
        .await;
        if let Err(failure) = expect_ok(status, &body, "登记 preflight", None) {
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        let preflight_id = body["preflight_id"].as_str().expect("preflight id");
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/registrations",
                self.lc_id
            ),
            json!({
                "preflight_id": preflight_id,
                "aggregate_root": aggregate_root,
                "confirmed_paths": [
                    aggregate_root.join("alpha").display().to_string(),
                    aggregate_root.join("beta").display().to_string(),
                ],
            }),
        )
        .await;
        if let Err(failure) = expect_ok(status, &body, "真实登记成员", None) {
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        if body["status"] != "completed" {
            let failure = matrix_failure(
                "registration_not_completed",
                format!("登记未完成:{body}"),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        Ok(())
    }

    /// 固定 Claude recipe 的真实聚合初始化(产品确认走真实 HTTP gate)。
    async fn run_real_aggregate_initialization(&mut self) -> Result<(), LiveMatrixFailure> {
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations",
                self.lc_id
            ),
            json!({"idempotency_key": format!("lcg-matrix-{}", self.lc_id)}),
        )
        .await;
        if status != StatusCode::ACCEPTED {
            let failure = matrix_failure(
                "initialization_rejected",
                format!("初始化启动被拒({status}):{body}"),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        let operation_id = body["operation_id"]
            .as_str()
            .expect("operation id")
            .to_string();
        let init_timeout = Duration::from_secs(env_timeout_secs(
            INIT_TIMEOUT_ENV,
            DEFAULT_INIT_TIMEOUT_SECS,
        ));
        let uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations/{operation_id}",
            self.lc_id
        );
        let deadline = tokio::time::Instant::now() + init_timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                let failure = matrix_failure(
                    "initialization_timeout",
                    format!("聚合初始化超时({init_timeout:?});环境不可运行须报告 BLOCKED"),
                    None,
                );
                return Err(self
                    .fail_with_diagnostics(failure, Some(&operation_id))
                    .await);
            }
            let (status, snapshot) = request_json(&self.app, Method::GET, &uri, json!({})).await;
            if let Err(failure) = expect_ok(status, &snapshot, "轮询初始化", None) {
                return Err(self
                    .fail_with_diagnostics(failure, Some(&operation_id))
                    .await);
            }
            match snapshot["status"].as_str() {
                Some("completed") => return Ok(()),
                Some("failed") | Some("cancelled") => {
                    let failure = matrix_failure(
                        "initialization_failed",
                        format!("聚合初始化 {snapshot}"),
                        None,
                    );
                    return Err(self
                        .fail_with_diagnostics(failure, Some(&operation_id))
                        .await);
                }
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    /// 真实索引 ready(初始化 D5 后 active index 应可用;缺失则 rebuild)。
    async fn wait_for_index_ready(&mut self) -> Result<(), LiveMatrixFailure> {
        let active_uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes/active",
            self.lc_id
        );
        let rebuild_uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes:rebuild",
            self.lc_id
        );
        let deadline = tokio::time::Instant::now() + self.stage_timeout;
        loop {
            let (status, body) = request_json(&self.app, Method::GET, &active_uri, json!({})).await;
            if let Err(failure) = expect_ok(status, &body, "读取 active index", None) {
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
            match body["state"].as_str() {
                // 终态:active 簇 → 通过。
                Some("ready") | Some("active") | Some("completed") => return Ok(()),
                // 终态:failed/error → 失败(带 warning 原文)。
                Some("failed") | Some("error") => {
                    let failure =
                        matrix_failure("index_failed", format!("聚合索引终态失败:{body}"), None);
                    return Err(self.fail_with_diagnostics(failure, None).await);
                }
                // 中间态(building/rebuilding/未知)→ 继续轮询到终态,
                // 有界超时兜底;不把中间态当终态判失败,也不重复 POST
                // rebuild(已有任务在跑会 409)。
                // 仅缺失(missing/None)时触发一次 rebuild。
                missing @ (Some("missing") | None) => {
                    let _ = missing;
                    let (rebuild_status, rebuild_body) =
                        request_json(&self.app, Method::POST, &rebuild_uri, json!({})).await;
                    if rebuild_status != StatusCode::ACCEPTED && rebuild_status != StatusCode::OK {
                        let failure = matrix_failure(
                            "index_rebuild_rejected",
                            format!("索引 rebuild 被拒({rebuild_status}):{rebuild_body}"),
                            None,
                        );
                        return Err(self.fail_with_diagnostics(failure, None).await);
                    }
                }
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                let failure = matrix_failure(
                    "index_timeout",
                    "索引未 ready(超时);环境不可运行须报告 BLOCKED".to_string(),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// canonical root + 成员 target 定位(fix 轮 5:对照 #8 E2E 同链路——
    /// 经 LC store 的成员记录(logical_repository_id/physical_repository_id/
    /// checkout_ids)与 checkout 记录(canonical_path)解析,不再查物理
    /// repository store 的 logical_repository_id 字段)。
    async fn resolve_member_target(&mut self) -> Result<(), LiveMatrixFailure> {
        self.canonical_root = self
            .aggregate_root_path()
            .to_path_buf()
            .canonicalize()
            .expect("canonical aggregate root");
        let alpha_source = self
            .canonical_root
            .join("alpha")
            .canonicalize()
            .expect("canonical alpha source");
        // fix 轮 6:store 必须 `for_lc` 作用域——checkout 记录随 per-LC
        // 子树落盘,`new()`(legacy project root)读不到;成员记录按
        // member.checkout_ids 逐个 `load_checkout`(#8 同链路),不做
        // 全项目 list 过滤。
        let store = LogicalCodebaseStore::for_lc(self.app_paths.clone(), self.lc_id.clone());
        let members = match store.list_lc_members(PROJECT_ID, &self.lc_id) {
            Ok(members) => members,
            Err(error) => {
                self.dump_members_diagnostics().await;
                let failure = matrix_failure(
                    "lc_member_store_error",
                    format!("读取 LC 成员记录失败:{error}"),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
        };
        if members.is_empty() {
            self.dump_members_diagnostics().await;
            let failure = matrix_failure(
                "member_repository_missing",
                "LC 成员记录为空(登记未产出成员)".to_string(),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        // 逐成员逐 checkout_id 读取记录;成员选择=checkout canonical_path
        // 以 alpha 源路径开头者优先,否则第一个读到 checkout 的成员。
        let mut resolved: Option<(&CodebaseMemberRecord, RepositoryCheckoutRecord)> = None;
        let mut alpha_matched: Option<(&CodebaseMemberRecord, RepositoryCheckoutRecord)> = None;
        for member in &members {
            for checkout_id in &member.checkout_ids {
                match store.load_checkout(PROJECT_ID, *checkout_id) {
                    Ok(Some(checkout)) => {
                        if checkout.canonical_path.starts_with(&alpha_source)
                            && alpha_matched.is_none()
                        {
                            alpha_matched = Some((member, checkout));
                        } else if resolved.is_none() {
                            resolved = Some((member, checkout));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.dump_members_diagnostics().await;
                        let failure = matrix_failure(
                            "lc_checkout_store_error",
                            format!("读取 checkout 记录失败:{error}"),
                            None,
                        );
                        return Err(self.fail_with_diagnostics(failure, None).await);
                    }
                }
            }
        }
        let Some((member, checkout)) = alpha_matched.or(resolved) else {
            self.dump_members_diagnostics().await;
            let failure = matrix_failure(
                "member_checkout_missing",
                format!(
                    "成员 checkout 记录缺失(成员 {members:?};store 作用域 lc_id={})",
                    self.lc_id
                ),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        };
        self.member_logical_id = member.logical_repository_id.0.to_string();
        self.member_checkout_id = checkout.checkout_id.0.to_string();
        self.member_physical_repo_id = member.physical_repository_id.clone();
        self.member_worktree = checkout
            .canonical_path
            .canonicalize()
            .expect("member checkout canonical");
        Ok(())
    }

    /// 成员解析失败时抄录现场到 diagnostics:GET members 实际 HTTP 响应、
    /// manifest 摘要、成员记录(含 checkout_ids)与逐 checkout 读取结果。
    async fn dump_members_diagnostics(&self) {
        let diagnostics_dir = self.evidence_root.join("diagnostics");
        let _ = std::fs::create_dir_all(&diagnostics_dir);
        let members_uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/members",
            self.lc_id
        );
        let (status, body) = request_json(&self.app, Method::GET, &members_uri, json!({})).await;
        let _ = std::fs::write(
            diagnostics_dir.join("members-endpoint.json"),
            serde_json::to_vec_pretty(&json!({
                "uri": members_uri,
                "http_status": status.as_u16(),
                "body": body,
            }))
            .unwrap_or_default(),
        );
        let store = LogicalCodebaseStore::for_lc(self.app_paths.clone(), self.lc_id.clone());
        let manifest = store
            .load_lc_manifest(PROJECT_ID, &self.lc_id)
            .ok()
            .flatten();
        let members = store.list_lc_members(PROJECT_ID, &self.lc_id).ok();
        let mut checkout_probes = Vec::new();
        if let Some(members) = &members {
            for member in members {
                for checkout_id in &member.checkout_ids {
                    let probe = match store.load_checkout(PROJECT_ID, *checkout_id) {
                        Ok(Some(checkout)) => json!({
                            "checkout_id": checkout_id.0.to_string(),
                            "found": true,
                            "canonical_path": checkout.canonical_path,
                            "kind": checkout.kind,
                        }),
                        Ok(None) => json!({
                            "checkout_id": checkout_id.0.to_string(),
                            "found": false,
                        }),
                        Err(error) => json!({
                            "checkout_id": checkout_id.0.to_string(),
                            "error": error.to_string(),
                        }),
                    };
                    checkout_probes.push(probe);
                }
            }
        }
        let _ = std::fs::write(
            diagnostics_dir.join("members-store.json"),
            serde_json::to_vec_pretty(&json!({
                "lc_id": self.lc_id,
                "member_count": manifest.as_ref().map(|value| value.member_ids.len()),
                "membership_revision": manifest.as_ref().map(|value| value.membership_revision),
                "members": members,
                "checkout_probes": checkout_probes,
            }))
            .unwrap_or_default(),
        );
    }

    async fn create_issue(&mut self) -> Result<(), LiveMatrixFailure> {
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues"),
            json!({
                "title": "矩阵 issue:跨仓会话过期修复",
                "description": "四家五阶段真实矩阵驱动 issue",
                "repository_id": self.member_physical_repo_id,
                "logical_codebase_id": self.lc_id,
            }),
        )
        .await;
        if let Err(failure) = expect_ok(status, &body, "创建逻辑 issue", None) {
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        self.issue_id = body["id"].as_str().expect("issue id").to_string();
        Ok(())
    }

    fn story_spec_ids(&self) -> Vec<String> {
        self.story_spec_id
            .as_ref()
            .map(|id| vec![id.clone()])
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // 阶段驱动:workspace 实体会话(story/design)。
    // -----------------------------------------------------------------------

    /// 驱动一个 workspace 实体会话阶段(story/design),fresh + resume 两格。
    async fn run_workspace_entity_stage(
        &mut self,
        stage: &'static str,
        generate_uri: &str,
        generate_body: Value,
        response_spec_field: &str,
    ) -> Vec<EvidenceCell> {
        let mut cells = Vec::new();
        // 本阶段专属会话锚:防 fresh 失败后 resume 误用上一阶段会话。
        self.prior_entity_session_id = None;
        // ---- fresh:生成实体 → WS streaming 驱动 → 真实 HTTP confirm ----
        let fresh = self
            .drive_entity_session_fresh(stage, generate_uri, &generate_body, response_spec_field)
            .await;
        cells.push(fresh);
        // ---- resume:同一会话显式 revision 重驱,原生恢复确认 ----
        let resume = self.drive_entity_session_resume(stage).await;
        cells.push(resume);
        cells
    }

    async fn drive_entity_session_fresh(
        &mut self,
        stage: &'static str,
        generate_uri: &str,
        generate_body: &Value,
        response_spec_field: &str,
    ) -> EvidenceCell {
        let mut observation = StageObservation::new(stage, &self.provider);
        let (status, body) =
            request_json(&self.app, Method::POST, generate_uri, generate_body.clone()).await;
        if !status.is_success() {
            return observation.denied_cell(format!("生成 {stage} 实体失败({status}):{body}"));
        }
        let session_id = body["workspace_session"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            return observation
                .denied_cell(format!("生成 {stage} 响应缺 workspace_session:{body}"));
        }
        if let Some(spec_id) = body[response_spec_field][0]["id"].as_str() {
            match stage {
                "story" => self.story_spec_id = Some(spec_id.to_string()),
                "design" => self.design_spec_id = Some(spec_id.to_string()),
                _ => {}
            }
        }
        observation.workspace_session_id = session_id.clone();
        self.prior_entity_session_id = Some(session_id.clone());

        // WS streaming 驱动:hello → start_generation → pump(真实 author
        // caller 链在服务端执行)。
        let drive = self
            .drive_workspace_session_ws(&session_id, &mut observation, 3)
            .await;
        observation.completed_product_artifact_exists = drive.artifact_confirmed;
        observation.build_cell(self)
    }

    /// 同一会话的显式 revision 重驱:gateway resume 路径的原生恢复确认。
    async fn drive_entity_session_resume(&mut self, stage: &'static str) -> EvidenceCell {
        let mut observation = StageObservation::new(stage, &self.provider);
        let Some(session_id) = self.prior_entity_session_id.clone() else {
            return observation.denied_cell("无可恢复的本阶段 fresh 会话".to_string());
        };
        observation.workspace_session_id = session_id.clone();
        observation.force_resume = true;
        // 请求恢复的 native id = fresh 轮审计里的 provider_session_id
        //(必须在 revision 重驱前捕获)。
        observation.requested_resume_id =
            self.latest_audit_native_id(&session_id, &self.provider, None);
        let revision = self
            .drive_revision_resume(&session_id, &mut observation, "矩阵 resume:显式修订重驱")
            .await;
        observation.completed_product_artifact_exists = revision.artifact_confirmed;
        // 原生恢复确认:revision 轮审计的 provider_session_id == 请求 id。
        observation.native_confirmed_id =
            self.latest_audit_native_id(&session_id, &self.provider, None);
        observation.build_cell(self)
    }

    /// revision 重驱公共路径:连接会话 WS → request_revision → pump。
    async fn drive_revision_resume(
        &self,
        session_id: &str,
        observation: &mut StageObservation,
        description: &str,
    ) -> DriveOutcome {
        let mut ws = match self.connect_session_ws(session_id).await {
            Ok(ws) => ws,
            Err(error) => {
                observation.push_event(json!({"type": "matrix_error", "message": error}));
                observation.run_failure = Some(format!("resume WS 连接失败:{error}"));
                return DriveOutcome::default();
            }
        };
        let revision = json!({
            "type": "request_revision",
            "feedback": {
                "feedback_types": ["scope"],
                "description": description,
                "target_artifact_version": null
            }
        });
        if let Err(error) = ws.send_json(&revision).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("request_revision 发送失败:{error}"));
            return DriveOutcome::default();
        }
        self.pump_workspace_session(&mut ws, observation, 4).await
    }

    /// 连接 workspace 会话 WS 并完成 hello。
    async fn connect_session_ws(&self, session_id: &str) -> Result<LiveWs, String> {
        let url = format!(
            "ws://{}/api/workspace-sessions/{session_id}/ws",
            self.ws_addr
        );
        let mut ws = connect_live_ws(&url).await?;
        ws.send_json(&json!({
            "type": "hello",
            "session_id": session_id,
            "last_seen_node_id": null,
        }))
        .await?;
        Ok(ws)
    }

    /// fresh 轮:连接 WS、start_generation(provider 配置真实选型)、pump。
    async fn drive_workspace_session_ws(
        &self,
        session_id: &str,
        observation: &mut StageObservation,
        confirm_rounds: usize,
    ) -> DriveOutcome {
        let mut ws = match self.connect_session_ws(session_id).await {
            Ok(ws) => ws,
            Err(error) => {
                observation.push_event(json!({"type": "matrix_error", "message": error}));
                observation.run_failure = Some(format!("workspace 会话 WS 连接失败:{error}"));
                return DriveOutcome::default();
            }
        };
        // 产品确认走真实 WS gate:StartGeneration 携带所选 provider 配置。
        // #13 Fake 默认只经 provider_select——本 harness 直接以所选真实
        // provider 发起,不写状态或能力。
        let start = json!({
            "type": "start_generation",
            "provider_config": {
                "author": self.provider,
                "reviewer": null,
                "review_rounds": 0
            },
            "reviewer_enabled": false
        });
        if let Err(error) = ws.send_json(&start).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("start_generation 发送失败:{error}"));
            return DriveOutcome::default();
        }
        self.pump_workspace_session(&mut ws, observation, confirm_rounds)
            .await
    }

    /// 泵送会话事件直至终态;waiting_for_human 时走真实 HTTP confirm 门。
    async fn pump_workspace_session(
        &self,
        ws: &mut LiveWs,
        observation: &mut StageObservation,
        confirm_rounds: usize,
    ) -> DriveOutcome {
        let mut outcome = DriveOutcome::default();
        let deadline = tokio::time::Instant::now() + self.stage_timeout;
        let mut confirms_left = confirm_rounds;
        loop {
            if tokio::time::Instant::now() >= deadline {
                observation.push_event(json!({"type": "matrix_stage_timeout"}));
                return outcome;
            }
            let message = tokio::time::timeout_at(deadline, ws.recv_json()).await;
            let message = match message {
                Ok(Ok(Some(value))) => value,
                Ok(Ok(None)) => {
                    observation.push_event(json!({"type": "matrix_ws_closed"}));
                    // F5:驱动失败原因必须落在格上,不得被 resume 兜底文案覆盖。
                    observation.run_failure = Some("workspace 会话 WS 在终态前关闭".to_string());
                    return outcome;
                }
                Ok(Err(error)) => {
                    observation.push_event(json!({"type": "matrix_ws_error", "message": error}));
                    observation.run_failure = Some(format!("workspace 会话 WS 错误:{error}"));
                    return outcome;
                }
                Err(_) => {
                    observation.push_event(json!({"type": "matrix_stage_timeout"}));
                    observation.run_failure = Some("workspace 会话阶段超时未达终态".to_string());
                    return outcome;
                }
            };
            let kind = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            observation.push_event(message.clone());
            // F6:tool 事件结构化计数(type/字段精确匹配,非全文子串)。
            if is_tool_event(&message) {
                observation.tool_events += 1;
            }
            match kind.as_str() {
                "session_state" => {
                    if let Some(status) = message.get("status").and_then(Value::as_str) {
                        observation.record_status(status);
                        match status {
                            "waiting_for_human" if confirms_left > 0 => {
                                confirms_left -= 1;
                                let uri = format!(
                                    "/api/workspace-sessions/{}/confirm",
                                    observation.workspace_session_id
                                );
                                let (status, body) = request_json(
                                    &self.app,
                                    Method::POST,
                                    &uri,
                                    json!({"confirmed_by": "lcg-matrix"}),
                                )
                                .await;
                                if status.is_success() {
                                    outcome.artifact_confirmed = true;
                                } else {
                                    observation.push_event(json!({
                                        "type": "matrix_confirm_rejected",
                                        "status": status.as_u16(),
                                        "body": body
                                    }));
                                }
                            }
                            "confirmed" => {
                                outcome.artifact_confirmed = true;
                            }
                            "failed"
                            | "blocked_provider_unavailable"
                            | "terminated"
                            | "stopped_needs_human" => {
                                observation.terminal_status = Some(status.to_string());
                                return outcome;
                            }
                            _ => {}
                        }
                    }
                }
                "choice_request" => {
                    // 真实 choice 门:按首个选项应答,不旁路产品决策面。
                    if let Some(choice_id) = message.get("id").and_then(Value::as_str) {
                        let first_option = message
                            .pointer("/options/0/id")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let _ = ws
                            .send_json(&json!({
                                "type": "choice_response",
                                "id": choice_id,
                                "selected_option_ids": [first_option],
                                "free_text": null
                            }))
                            .await;
                    }
                }
                "permission_request" => {
                    // 审批事件留证;默认不自动放行(策略由生产侧裁决)。
                    observation.permission_events += 1;
                }
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Plan:workspace streaming 主入口。
    // -----------------------------------------------------------------------

    async fn run_plan_streaming_stage(&mut self) -> Vec<EvidenceCell> {
        let mut cells = Vec::new();
        if self.story_spec_id.is_none() || self.design_spec_id.is_none() {
            cells.push(
                StageObservation::new("plan", &self.provider)
                    .denied_cell("前置 story/design 未确认,plan 无法启动(不删格)".to_string()),
            );
            cells.push(
                StageObservation::new("plan", &self.provider)
                    .denied_cell("前置 story/design 未确认,plan resume 无会话可恢复".to_string()),
            );
            return cells;
        }
        // ---- fresh:prepare plan 会话(真实 start_work_item_plan_author
        // caller 链在 WS author 驱动里执行:begin handle→bind sink→start→
        // parse/complete)----
        let mut observation = StageObservation::new("plan", &self.provider);
        observation.role = "work_item_splitter".to_string();
        let design_spec_id = self.design_spec_id.clone().expect("design spec");
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{}/work-item-plans:prepare",
                self.issue_id
            ),
            json!({
                "title": "矩阵 plan:跨仓修复拆分",
                "story_spec_ids": self.story_spec_ids(),
                "design_spec_ids": [design_spec_id],
                "author_provider": self.provider_wire,
                "reviewer_provider": self.provider_wire,
                "review_rounds": 0,
                "superpowers_enabled": false,
                "openspec_enabled": true
            }),
        )
        .await;
        if !status.is_success() {
            cells.push(observation.denied_cell(format!("plan prepare 失败({status}):{body}")));
            cells.push(
                StageObservation::new("plan", &self.provider)
                    .denied_cell("plan fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        }
        let session_id = body["workspace_session"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            cells.push(observation.denied_cell(format!("plan prepare 缺会话:{body}")));
            cells.push(
                StageObservation::new("plan", &self.provider)
                    .denied_cell("plan fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        }
        observation.workspace_session_id = session_id.clone();
        self.prior_plan_session_id = Some(session_id.clone());
        let drive = self
            .drive_workspace_session_ws(&session_id, &mut observation, 4)
            .await;
        observation.completed_product_artifact_exists = drive.artifact_confirmed;
        cells.push(observation.build_cell(self));
        // Plan confirmed 后解析 work item(coding 前置)。
        self.resolve_first_work_item().await;

        // ---- resume:plan 会话 revision 重驱 ----
        let mut resume = StageObservation::new("plan", &self.provider);
        resume.role = "work_item_splitter".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = session_id.clone();
        resume.requested_resume_id = self.latest_audit_native_id(&session_id, &self.provider, None);
        let drive = self
            .drive_revision_resume(
                &session_id,
                &mut resume,
                "矩阵 plan resume:显式修订重驱,须原生恢复 native 会话",
            )
            .await;
        resume.completed_product_artifact_exists = drive.artifact_confirmed;
        resume.native_confirmed_id = self.latest_audit_native_id(&session_id, &self.provider, None);
        cells.push(resume.build_cell(self));
        cells
    }

    async fn resolve_first_work_item(&mut self) {
        let (status, body) = request_json(
            &self.app,
            Method::GET,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{}/work-items",
                self.issue_id
            ),
            json!({}),
        )
        .await;
        if status.is_success() {
            if let Some(id) = body
                .pointer("/work_items/0/id")
                .or_else(|| body.pointer("/0/id"))
                .and_then(Value::as_str)
            {
                self.work_item_id = Some(id.to_string());
                return;
            }
        }
        // 列表端点形态变化时留 None,coding 阶段落格记录原因。
        self.work_item_id = None;
    }

    // -----------------------------------------------------------------------
    // Plan:split_sync 对照(gateway sync bridge 真实栈)。
    // -----------------------------------------------------------------------

    async fn run_split_sync_stage(&mut self) -> Vec<EvidenceCell> {
        let mut cells = Vec::new();
        let stage: &'static str = "plan";
        let entrypoint = ENTRYPOINT_SPLIT_SYNC;

        // fresh:begin handle → prepare_sync_launch(绑定 run-bound audit
        // sink)→ run_sync(真实 CLI)→ complete(handle 收口)。
        let mut fresh = StageObservation::new(stage, &self.provider);
        fresh.entrypoint = entrypoint.to_string();
        fresh.role = "work_item_splitter".to_string();
        fresh.action = "planning_read_only".to_string();
        let workspace_session_id = self
            .prior_plan_session_id
            .clone()
            .unwrap_or_else(|| format!("lcg-split-sync-{}", self.lc_id));
        fresh.workspace_session_id = workspace_session_id.clone();

        let before_starts = self.count_session_provider_starts(&workspace_session_id);
        let gateway = match self
            .gateway_factory
            .build_for_lc(PROJECT_ID, Some(&self.lc_id))
        {
            Ok(gateway) => gateway,
            Err(error) => {
                cells.push(fresh.denied_cell(format!("split_sync gateway 组装失败:{error}")));
                cells.push(
                    StageObservation::new(stage, &self.provider)
                        .denied_cell("split_sync fresh 未启动,resume 无对照".to_string()),
                );
                return cells;
            }
        };
        let handle = match self.lifecycle.begin_work_item_split_provider_run(
            PROJECT_ID,
            &self.issue_id,
            &self.provider,
            &workspace_session_id,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                cells.push(fresh.denied_cell(format!("split_sync begin handle 失败:{error}")));
                cells.push(
                    StageObservation::new(stage, &self.provider)
                        .denied_cell("split_sync fresh 未启动,resume 无对照".to_string()),
                );
                return cells;
            }
        };
        fresh.run_ref = handle.run_ref.clone();
        fresh.role_run_seq = Some(handle.role_run_seq);

        let provider_ref =
            match ProviderRef::from_provider_name(&self.provider, "cap_managed_snapshot") {
                Ok(provider_ref) => provider_ref,
                Err(error) => {
                    let _ = self
                        .lifecycle
                        .fail_work_item_split_provider_run(&handle, &error.to_string());
                    cells.push(fresh.denied_cell(format!("split_sync provider 映射失败:{error}")));
                    cells.push(
                        StageObservation::new(stage, &self.provider)
                            .denied_cell("split_sync fresh 未启动,resume 无对照".to_string()),
                    );
                    return cells;
                }
            };
        // F3:split_sync 自备绝对 stream log 目录——真实子进程 PID 从
        // `{program}-{pid}-{stream}.log` 文件名解析(时间线可追溯)。
        let split_log_dir = self
            .evidence_root
            .join(stage)
            .join(entrypoint.replace('/', "-"))
            .join(FRESH)
            .join("stream-logs");
        let _ = std::fs::create_dir_all(&split_log_dir);
        let split_log_dir = split_log_dir.canonicalize().unwrap_or(split_log_dir);
        let adapter_input = AdapterInput {
            provider_type: provider_type_for(&self.provider),
            role: AdapterRole::WorkItemSplitter,
            // cwd=canonical root(gateway 冻结);worktree_path=target 成员路径。
            working_directory: Some(gateway.authority_root().to_path_buf()),
            worktree_path: Some(self.member_worktree.to_string_lossy().into_owned()),
            provider_stream_log_dir: Some(split_log_dir.to_string_lossy().into_owned()),
            prompt: split_sync_prompt(),
            context_files: Vec::new(),
            output_schema: work_item_split_output_schema(),
            timeout: self.stage_timeout.as_secs(),
            max_retries: 1,
        };
        let request = SessionLaunchRequest {
            project_id: PROJECT_ID.to_string(),
            provider: provider_ref,
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::checkout(
                self.member_logical_id.clone(),
                self.member_checkout_id.clone(),
                self.member_worktree.clone(),
            ),
            working_directory: gateway.authority_root().to_path_buf(),
            readable_roots: vec![gateway.authority_root().to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        let context = ProviderLaunchAuditContext {
            workspace_session_id: workspace_session_id.clone(),
            role_run_seq: handle.role_run_seq,
            audit_sink: Arc::new(self.lifecycle.clone()),
        };
        let launch = gateway
            .prepare_sync_launch(adapter_input, request, context)
            .map_err(|error| error.to_string());
        match launch {
            Ok(launch) => match gateway.run_sync(launch) {
                Ok(output) => {
                    let structured = output.structured_output.clone();
                    let complete = structured.as_ref().map(|value| {
                        self.lifecycle
                            .complete_work_item_split_provider_run(
                                &handle,
                                &split_sync_prompt(),
                                value,
                            )
                            .map(|_| ())
                    });
                    fresh.completed_product_artifact_exists =
                        complete.is_some_and(|result| result.is_ok());
                    if !fresh.completed_product_artifact_exists {
                        let reason = format!("split_sync 结构化产物缺失/收口失败:{output:?}");
                        let _ = self
                            .lifecycle
                            .fail_work_item_split_provider_run(&handle, &reason);
                        fresh.run_failure = Some(reason);
                    }
                }
                Err(error) => {
                    let _ = self
                        .lifecycle
                        .fail_work_item_split_provider_run(&handle, &error.to_string());
                    fresh.run_failure = Some(format!("split_sync run_sync 失败:{error}"));
                }
            },
            Err(reason) => {
                let _ = self
                    .lifecycle
                    .fail_work_item_split_provider_run(&handle, &reason);
                fresh.run_failure = Some(format!("split_sync prepare 失败:{reason}"));
            }
        }
        let after_starts = self.count_session_provider_starts(&workspace_session_id);
        fresh.observed_spawn_count = after_starts.saturating_sub(before_starts);
        // F3:split_sync PID 来自自备 stream log 目录的真实文件名。
        fresh.observed_pid = scan_stream_log_pid(&split_log_dir);
        cells.push(fresh.build_cell(self));

        // resume:sync split 无 native session contract → resume=Unknown、
        // 零 spawn(不借 streaming fresh 推导支持),证据如实记录。
        let mut resume = StageObservation::new(stage, &self.provider);
        resume.entrypoint = entrypoint.to_string();
        resume.role = "work_item_splitter".to_string();
        resume.action = "planning_read_only".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = workspace_session_id;
        resume.observed_spawn_count = 0;
        resume.run_failure = Some(
            "sync split 无 native session contract:resume=Unknown,零 spawn(新增 provider_start=0)"
                .to_string(),
        );
        cells.push(resume.build_cell(self));
        cells
    }

    // -----------------------------------------------------------------------
    // Coding 与 Review。
    // -----------------------------------------------------------------------

    async fn run_coding_stage(&mut self) -> Vec<EvidenceCell> {
        let mut cells = Vec::new();
        let Some(work_item_id) = self.work_item_id.clone() else {
            cells.push(
                StageObservation::new("coding", &self.provider).denied_cell(
                    "无已确认 work item(前置 plan 未产出),coding 无法启动".to_string(),
                ),
            );
            cells.push(
                StageObservation::new("coding", &self.provider)
                    .denied_cell("coding fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        };
        // fresh:真实 coding attempt 创建(创建沿 plan 会话的 provider 配置)
        // → coding WS 驱动(StartCoding/阶段门)→ FinalConfirm。
        let mut observation = StageObservation::new("coding", &self.provider);
        observation.role = "executor".to_string();
        observation.action = "coding_workspace_write".to_string();
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{}/work-items/{work_item_id}/coding-attempts",
                self.issue_id
            ),
            json!({}),
        )
        .await;
        if !status.is_success() {
            cells
                .push(observation.denied_cell(format!("coding attempt 创建失败({status}):{body}")));
            cells.push(
                StageObservation::new("coding", &self.provider)
                    .denied_cell("coding fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        }
        let attempt_id = body["id"]
            .as_str()
            .or_else(|| body["attempt"]["id"].as_str())
            .unwrap_or_default()
            .to_string();
        if attempt_id.is_empty() {
            cells.push(observation.denied_cell(format!("coding attempt 响应缺 id:{body}")));
            cells.push(
                StageObservation::new("coding", &self.provider)
                    .denied_cell("coding fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        }
        observation.workspace_session_id = attempt_id.clone();
        self.prior_coding_session_id = Some(attempt_id.clone());
        let drive = self
            .drive_coding_attempt_ws(&attempt_id, &mut observation)
            .await;
        observation.completed_product_artifact_exists = drive.artifact_confirmed;
        // F3:PID 取自该 attempt 的 provider stream log 目录(生产路径)。
        observation.observed_pid = self.scan_attempt_stream_log_pid(&attempt_id);
        cells.push(observation.build_cell(self));

        // resume:重连 coding WS 再次驱动(原生恢复由生产 resume 语义裁决,
        // 审计 provider_session_id 与 fresh 轮比对)。
        let mut resume = StageObservation::new("coding", &self.provider);
        resume.role = "executor".to_string();
        resume.action = "coding_workspace_write".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = attempt_id.clone();
        resume.requested_resume_id = self.latest_audit_native_id(&attempt_id, &self.provider, None);
        let drive = self.drive_coding_attempt_ws(&attempt_id, &mut resume).await;
        resume.completed_product_artifact_exists = drive.artifact_confirmed;
        resume.observed_pid = self.scan_attempt_stream_log_pid(&attempt_id);
        resume.native_confirmed_id = self.latest_audit_native_id(&attempt_id, &self.provider, None);
        cells.push(resume.build_cell(self));
        cells
    }

    async fn drive_coding_attempt_ws(
        &self,
        attempt_id: &str,
        observation: &mut StageObservation,
    ) -> DriveOutcome {
        let url = format!("ws://{}/ws/coding-attempts/{attempt_id}", self.ws_addr);
        let mut ws = match connect_live_ws(&url).await {
            Ok(ws) => ws,
            Err(error) => {
                observation.push_event(json!({"type": "matrix_error", "message": error}));
                return DriveOutcome::default();
            }
        };
        if let Err(error) = ws
            .send_json(&json!({
                "type": "coding_hello",
                "attempt_id": attempt_id,
                "last_seen_node_id": null
            }))
            .await
        {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("coding_hello 发送失败:{error}"));
            return DriveOutcome::default();
        }
        if let Err(error) = ws.send_json(&json!({"type": "start_coding"})).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("start_coding 发送失败:{error}"));
            return DriveOutcome::default();
        }
        let mut outcome = DriveOutcome::default();
        let deadline = tokio::time::Instant::now() + self.stage_timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                observation.push_event(json!({"type": "matrix_stage_timeout"}));
                return outcome;
            }
            let message = tokio::time::timeout_at(deadline, ws.recv_json()).await;
            let message = match message {
                Ok(Ok(Some(value))) => value,
                Ok(Ok(None)) | Ok(Err(_)) => {
                    observation.run_failure = Some("coding 会话 WS 在终态前关闭/错误".to_string());
                    return outcome;
                }
                Err(_) => {
                    observation.push_event(json!({"type": "matrix_stage_timeout"}));
                    observation.run_failure = Some("coding 阶段超时未达终态".to_string());
                    return outcome;
                }
            };
            let kind = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            observation.push_event(message.clone());
            // F6:tool 事件结构化计数。
            if is_tool_event(&message) {
                observation.tool_events += 1;
            }
            match kind.as_str() {
                "coding_gate_required" => {
                    // 真实阶段门:以首个可用动作应答,不旁路产品决策面。
                    let gate_id = message
                        .pointer("/gate/id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let action_id = message
                        .pointer("/gate/available_actions/0/action_id")
                        .or_else(|| message.pointer("/gate/available_actions/0/id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !gate_id.is_empty() && !action_id.is_empty() {
                        let _ = ws
                            .send_json(&json!({
                                "type": "gate_response",
                                "gate_id": gate_id,
                                "action_id": action_id,
                                "extra_context": null
                            }))
                            .await;
                    }
                }
                "coding_session_state" => {
                    if let Some(status) = message.get("status").and_then(Value::as_str) {
                        observation.record_status(status);
                        match status {
                            // F2:waiting_for_human 即阶段门就绪——确认成功并
                            // 立即返回,不再烧满 stage_timeout。
                            "waiting_for_human" => {
                                outcome.artifact_confirmed = true;
                                return outcome;
                            }
                            "completed" | "confirmed" => {
                                outcome.artifact_confirmed = true;
                                return outcome;
                            }
                            "failed" | "aborted" => {
                                observation.terminal_status = Some(status.to_string());
                                observation.run_failure =
                                    Some(format!("coding attempt 终态 {status}"));
                                return outcome;
                            }
                            _ => {}
                        }
                    }
                }
                "coding_permission_request" | "permission_request" => {
                    observation.permission_events += 1;
                }
                _ => {}
            }
        }
    }

    /// Review 阶段:reviewer 角色的真实评审会话(streaming 栈)。
    async fn run_review_stage(&mut self) -> Vec<EvidenceCell> {
        let mut cells = Vec::new();
        // reviewer 评审走 design 实体 review_rounds=1(reviewer=所选 provider)。
        let mut observation = StageObservation::new("review", &self.provider);
        observation.role = "reviewer".to_string();
        observation.action = "review_read_only".to_string();
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{}/design-specs:generate",
                self.issue_id
            ),
            json!({
                "title": "矩阵 review:评审会话载体",
                "story_spec_ids": self.story_spec_ids(),
                "author_provider": self.provider_wire,
                "reviewer_provider": self.provider_wire,
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": true
            }),
        )
        .await;
        if !status.is_success() {
            cells
                .push(observation.denied_cell(format!("review 载体会话创建失败({status}):{body}")));
            cells.push(
                StageObservation::new("review", &self.provider)
                    .denied_cell("review fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        }
        let session_id = body["workspace_session"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            cells.push(observation.denied_cell(format!("review 响应缺会话:{body}")));
            cells.push(
                StageObservation::new("review", &self.provider)
                    .denied_cell("review fresh 未启动,resume 无会话".to_string()),
            );
            return cells;
        }
        observation.workspace_session_id = session_id.clone();
        self.prior_review_session_id = Some(session_id.clone());
        let drive = self
            .drive_workspace_session_ws(&session_id, &mut observation, 4)
            .await;
        observation.completed_product_artifact_exists = drive.artifact_confirmed;
        cells.push(observation.build_cell(self));

        // resume:评审修订重驱(reviewer 原生恢复)。
        let mut resume = StageObservation::new("review", &self.provider);
        resume.role = "reviewer".to_string();
        resume.action = "review_read_only".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = session_id.clone();
        resume.requested_resume_id =
            self.latest_audit_native_id(&session_id, &self.provider, Some("reviewer"));
        let drive = self
            .drive_revision_resume(
                &session_id,
                &mut resume,
                "矩阵 review resume:显式修订重驱,须原生恢复 reviewer 会话",
            )
            .await;
        resume.completed_product_artifact_exists = drive.artifact_confirmed;
        resume.native_confirmed_id =
            self.latest_audit_native_id(&session_id, &self.provider, Some("reviewer"));
        cells.push(resume.build_cell(self));
        cells
    }

    // -----------------------------------------------------------------------
    // 审计读取与证据落盘。
    // -----------------------------------------------------------------------

    /// 会话分区内该 provider(可按 role 过滤)最新 provider_start 的原生 id。
    fn latest_audit_native_id(
        &self,
        workspace_session_id: &str,
        provider: &ProviderName,
        role: Option<&str>,
    ) -> Option<String> {
        self.scan_session_audits(workspace_session_id)
            .into_iter()
            .find(|(_, record)| {
                provider_matches_record(provider, record)
                    && role.is_none_or(|role| record.role == role)
            })
            .map(|(_, record)| record.provider_session_id)
    }

    /// F3:从该 coding attempt 的 provider stream log 目录解析真实子进程 PID。
    fn scan_attempt_stream_log_pid(&self, attempt_id: &str) -> Option<String> {
        let directory = CodingAttemptStore::new(self.app_paths.clone()).provider_stream_log_root(
            PROJECT_ID,
            &self.issue_id,
            attempt_id,
        );
        scan_stream_log_pid(&directory)
    }

    fn count_session_provider_starts(&self, workspace_session_id: &str) -> usize {
        self.scan_session_audits(workspace_session_id)
            .into_iter()
            .filter(|(_, record)| provider_matches_record(&self.provider, record))
            .count()
    }

    /// 扫描会话审计分区(role_run_seq 有界扫描,最新在前)。
    fn scan_session_audits(&self, workspace_session_id: &str) -> Vec<(u64, ProviderStartAudit)> {
        let mut found = Vec::new();
        for seq in 0..=64u64 {
            let Ok(result) = self
                .lifecycle
                .read_tool_policy_lines_with_warnings(workspace_session_id, seq)
            else {
                break;
            };
            for line in result.events {
                if let DurableToolPolicyEvent::ProviderStart(record) = line.event {
                    found.push((seq, record));
                }
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0));
        found
    }

    /// 逐格证据落盘(485 行形态全要素)。F7:IO 失败不静默——返回 Err 由
    /// 上层把该格降级 denied(reason=证据落盘失败),不出现绿格+空目录。
    fn write_cell_evidence(&self, cell: &EvidenceCell) -> Result<(), String> {
        let entrypoint_dir = cell.entrypoint.replace('/', "-");
        let cell_dir = self
            .evidence_root
            .join(&cell.stage)
            .join(entrypoint_dir)
            .join(&cell.fresh_or_resume);
        let write = |name: &str, bytes: Vec<u8>| -> Result<(), String> {
            std::fs::write(cell_dir.join(name), bytes)
                .map_err(|error| format!("证据落盘失败 {}/{}: {error}", cell_dir.display(), name))
        };
        std::fs::create_dir_all(&cell_dir)
            .map_err(|error| format!("证据目录创建失败 {}: {error}", cell_dir.display()))?;
        // cell.json:键 + 断言组字段 + 全要素 + 时间线摘要(敏感项不落盘)。
        write(
            "cell.json",
            serde_json::to_vec_pretty(&cell.to_cell_json()).unwrap_or_default(),
        )?;
        // F1:provider-events.jsonl 落盘全事件——provider_start 首行(真实
        // argv/wire 形态)之后逐事件追加(ts 封包;approval/tool/关键帧
        // 磁盘可核对,不注入政策、不伪造事件)。
        let mut events = String::new();
        let provider_start = json!({
            "ts": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "event": {
                "kind": "provider_start_argv",
                "argv": cell.argv,
                "wire_dialect": cell.wire_dialect,
                "native_session_id": cell.native_session_id,
                "provider_pid": cell.provider_pid,
            }
        });
        events.push_str(&serde_json::to_string(&provider_start).unwrap_or_default());
        events.push('\n');
        for event in &cell.provider_events {
            events.push_str(&serde_json::to_string(event).unwrap_or_default());
            events.push('\n');
        }
        write("provider-events.jsonl", events.into_bytes())?;
        // F4:frozen-facts.json 补齐 485 全要素——#8 最终 policy artifact 从
        // durable store 真实读取(policy_id/revision/raw-body digest+字节数);
        // config artifact ref 如实标注生产约定常量(非独立真实摘要,不冒充);
        // trust/bundle 不可达项显式标 missing,不以 null 混同缺记录。
        let artifact =
            AggregatePolicyArtifactStore::for_lc(self.app_paths.clone(), self.lc_id.clone())
                .get(PROJECT_ID)
                .ok()
                .flatten();
        let raw_body = artifact.as_ref().map(|value| {
            let digest = Sha256::digest(value.policy_text.as_bytes());
            let raw_body_sha256: String =
                digest.iter().map(|byte| format!("{byte:02x}")).collect();
            json!({
                "policy_id": value.policy_id,
                "revision": value.revision,
                "digest": value.digest,
                "raw_body_bytes": value.policy_text.len(),
                "raw_body_sha256": raw_body_sha256,
            })
        })
            .unwrap_or_else(|| {
                json!({"missing": "#8 最终 policy artifact 未在 durable store 解析到(该格不可凭此 Confirmed)"})
            });
        write(
            "frozen-facts.json",
            serde_json::to_vec_pretty(&json!({
                "canonical_cwd": self.canonical_root,
                "target": self.member_worktree,
                "target_git_head": git_head(&self.member_worktree),
                "provider": cell.provider,
                "exact_version": cell.exact_version,
                "policy": raw_body,
                "config_artifact_ref": {
                    "value": "sha256:managed-config-artifact",
                    "provenance": "生产约定常量(engine.rs 同款字面量);托管 config artifact 存储未落地,非独立真实摘要,不冒充",
                },
                "capability_row": {
                    "capability_projection_digest": cell.frozen_projection_digest,
                    "session_projection_digest": cell.audit_projection_digest,
                    "spawn_count": cell.provider_spawn_count,
                },
                "projection": {
                    "gateway_dialect": cell.gateway_dialect,
                    "wire_dialect": cell.wire_dialect,
                    "action": cell.action,
                    "role": cell.role,
                },
                "trust": {
                    "missing": "trust home 状态不在本 harness 读取面;由生产 trust 审计链(#8/trust store)核对",
                },
                "mcp_bundle_digest": {
                    "missing": "Aria 注入 bundle digest 逐会话冻结于服务端 audit;此处不复制(不适用格记 missing)",
                },
            }))
            .unwrap_or_default(),
        )?;
        // pre/post snapshot:成员 worktree git 状态(D4 baseline 引用同源)。
        write(
            "pre-snapshot.json",
            serde_json::to_vec_pretty(&git_snapshot(&self.member_worktree)).unwrap_or_default(),
        )?;
        write(
            "post-snapshot.json",
            serde_json::to_vec_pretty(&git_snapshot(&self.member_worktree)).unwrap_or_default(),
        )?;
        // boundary-attempts.jsonl:本格观测到的越界写尝试(正/负向探针归
        // Task 11 boundary_matrix;此处只留真实流观测位)。
        write("boundary-attempts.jsonl", Vec::new())?;
        // result.json:格子结论 + spawn 计数 + 时间线摘要。
        write(
            "result.json",
            serde_json::to_vec_pretty(&json!({
                "capability_state": cell.capability_state,
                "denied_reason": cell.denied_reason,
                "run_ref": cell.run_ref,
                "workspace_session_id": cell.workspace_session_id,
                "provider_pid": cell.provider_pid,
                "pid_unavailable_reason": cell.pid_unavailable_reason,
                "provider_spawn_count": cell.provider_spawn_count,
                "timeline": cell.timeline_summary(),
            }))
            .unwrap_or_default(),
        )?;
        // sha256 清单(排除自身)。
        let mut names: Vec<String> = std::fs::read_dir(&cell_dir)
            .map_err(|error| format!("证据目录读取失败 {}: {error}", cell_dir.display()))?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut manifest = String::new();
        for name in names {
            if name == "manifest.sha256" {
                continue;
            }
            let bytes = std::fs::read(cell_dir.join(&name)).map_err(|error| {
                format!("清单读取失败 {}/{}: {error}", cell_dir.display(), name)
            })?;
            let digest = Sha256::digest(&bytes);
            let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            manifest.push_str(&format!("{hex}  {name}\n"));
        }
        write("manifest.sha256", manifest.into_bytes())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 阶段观测 → 证据格。
// ---------------------------------------------------------------------------

struct StageObservation {
    stage: &'static str,
    entrypoint: String,
    provider: ProviderName,
    workspace_session_id: String,
    run_ref: String,
    role_run_seq: Option<u64>,
    role: String,
    action: String,
    /// 带 ts 封包的事件载荷(provider-events.jsonl 全量来源;512 上限)。
    events: Vec<Value>,
    statuses: Vec<String>,
    terminal_status: Option<String>,
    permission_events: usize,
    tool_events: usize,
    run_failure: Option<String>,
    force_resume: bool,
    requested_resume_id: Option<String>,
    native_confirmed_id: Option<String>,
    completed_product_artifact_exists: bool,
    observed_spawn_count: usize,
    /// F3:驱动侧解析到的 provider 子进程 PID(stream log 文件名)。
    observed_pid: Option<String>,
}

impl StageObservation {
    fn new(stage: &'static str, provider: &ProviderName) -> Self {
        Self {
            stage,
            entrypoint: ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string(),
            provider: provider.clone(),
            workspace_session_id: String::new(),
            run_ref: String::new(),
            role_run_seq: None,
            role: "orchestrator".to_string(),
            action: "planning_read_only".to_string(),
            events: Vec::new(),
            statuses: Vec::new(),
            terminal_status: None,
            permission_events: 0,
            tool_events: 0,
            run_failure: None,
            force_resume: false,
            requested_resume_id: None,
            native_confirmed_id: None,
            completed_product_artifact_exists: false,
            observed_spawn_count: 0,
            observed_pid: None,
        }
    }

    fn push_event(&mut self, event: Value) {
        // 事件留证上限:防长流撑爆内存(完整流在服务端 durable 审计)。
        // F3:每事件带 wall-clock ts,落 provider-events.jsonl 供时间线核对。
        if self.events.len() < 512 {
            let wrapped = json!({
                "ts": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "event": event,
            });
            self.events.push(wrapped);
        }
    }

    fn record_status(&mut self, status: &str) {
        self.statuses.push(status.to_string());
    }

    /// 失败/阻断格:保留键与入口维度,记录 reason(不删格)。
    fn denied_cell(mut self, reason: String) -> EvidenceCell {
        self.run_failure = Some(reason);
        let mut cell = self.placeholder_cell();
        cell.capability_state = if self.observed_spawn_count > 0 {
            "denied".to_string()
        } else {
            "unknown".to_string()
        };
        cell.denied_reason = self.run_failure.clone();
        cell
    }

    fn placeholder_cell(&self) -> EvidenceCell {
        EvidenceCell {
            provider: self.provider.clone(),
            exact_version: String::new(),
            stage: self.stage.to_string(),
            entrypoint: self.entrypoint.clone(),
            fresh_or_resume: if self.force_resume {
                RESUME.to_string()
            } else {
                FRESH.to_string()
            },
            process_cwd: PathBuf::new(),
            target: PathBuf::new(),
            audit_projection_digest: String::new(),
            frozen_projection_digest: String::new(),
            native_resume_confirmed_id: None,
            requested_resume_id: None,
            argv_or_wire_capture_exists: false,
            approval_and_tool_events_exist: false,
            completed_product_artifact_exists: false,
            run_ref: String::new(),
            run_ref_is_unique_within_entrypoint: false,
            action: self.action.clone(),
            role: self.role.clone(),
            gateway_dialect: String::new(),
            wire_dialect: String::new(),
            native_session_id: String::new(),
            workspace_session_id: self.workspace_session_id.clone(),
            argv: Vec::new(),
            capability_state: "unknown".to_string(),
            denied_reason: None,
            provider_pid: None,
            pid_unavailable_reason: None,
            provider_spawn_count: self.observed_spawn_count as u64,
            provider_events: self.events.clone(),
        }
    }

    /// 由真实观测(审计 + 事件流 + 产品门)组装证据格。
    fn build_cell(&mut self, env: &MatrixEnvironment) -> EvidenceCell {
        let mut cell = self.placeholder_cell();
        cell.process_cwd = env.canonical_root.clone();
        cell.target = env.member_worktree.clone();
        cell.requested_resume_id = self.requested_resume_id.clone();
        cell.native_resume_confirmed_id = self.native_confirmed_id.clone();
        cell.completed_product_artifact_exists = self.completed_product_artifact_exists;
        // F3/F5:PID 可追溯(要么真实 PID,要么不可达说明)+spawn 计数。
        cell.provider_pid = self.observed_pid.clone();
        if cell.provider_pid.is_none() {
            cell.pid_unavailable_reason = Some(
                "该入口未由生产路径提供 provider stream log 目录,PID 不可达 \
                 (时间线以事件 ts+audit role_run_seq 追溯)"
                    .to_string(),
            );
        }
        if !self.run_ref.trim().is_empty() {
            cell.run_ref = self.run_ref.clone();
        }

        // 审计:该会话分区内所选 provider 的最新 provider_start。
        let audits = env.scan_session_audits(&self.workspace_session_id);
        if let Some((seq, record)) = audits
            .iter()
            .find(|(_, record)| provider_matches_record(&env.provider, record))
        {
            cell.exact_version = record.provider_version.clone();
            cell.argv = record.argv.clone();
            cell.argv_or_wire_capture_exists = !record.argv.is_empty();
            cell.gateway_dialect = record.adapter_dialect.clone();
            cell.native_session_id = record.provider_session_id.clone();
            cell.role = record.role.clone();
            if let Some(projection) = &record.lc_projection {
                cell.action = projection.action.clone();
                cell.wire_dialect = projection.wire_dialect.clone();
                cell.frozen_projection_digest = projection.capability_projection_digest.clone();
                cell.audit_projection_digest = projection.projection_digest.clone();
            }
            if self.role_run_seq.is_none() {
                self.role_run_seq = Some(*seq);
            }
            // run_ref:workspace streaming 栈以 run-bound 身份回填。
            if cell.run_ref.trim().is_empty() {
                cell.run_ref = format!(
                    "ws-{}-{}-run-{}",
                    self.workspace_session_id, self.stage, seq
                );
            }
        }
        // approval/tool 事件:审计 approval_decision + WS 事件流合计。
        let audit_approvals = env
            .scan_session_audits(&self.workspace_session_id)
            .into_iter()
            .filter(|(seq, _)| {
                env.lifecycle.contains_tool_policy_event(
                    &self.workspace_session_id,
                    *seq,
                    "approval_decision",
                )
            })
            .count();
        cell.approval_and_tool_events_exist =
            self.tool_events + self.permission_events + audit_approvals > 0;

        // 结论:全部要素齐备且无运行失败才 Confirmed;否则 Unknown/Denied。
        if let Some(reason) = self.run_failure.clone() {
            cell.capability_state = if self.observed_spawn_count > 0 {
                "denied".to_string()
            } else {
                "unknown".to_string()
            };
            cell.denied_reason = Some(reason);
        } else if self.force_resume {
            // resume 格:原生恢复确认必须等于请求 id。
            match (&self.requested_resume_id, &self.native_confirmed_id) {
                (Some(requested), Some(confirmed)) if requested == confirmed => {
                    cell.capability_state = "confirmed".to_string();
                }
                _ => {
                    cell.capability_state = "unknown".to_string();
                    cell.denied_reason = Some(format!(
                        "原生恢复未确认:请求 {:?} 实测 {:?}",
                        self.requested_resume_id, self.native_confirmed_id
                    ));
                }
            }
        } else if cell.exact_version.is_empty() {
            cell.capability_state = "unknown".to_string();
            cell.denied_reason =
                Some("会话审计无该 provider 的 provider_start(未观测到真实启动)".to_string());
        } else {
            cell.capability_state = "confirmed".to_string();
        }
        // 完成产物门:Confirmed 必须有产物。
        if cell.capability_state == "confirmed" && !cell.completed_product_artifact_exists {
            cell.capability_state = "denied".to_string();
            cell.denied_reason = Some("缺完成产物(产品确认门未通过)".to_string());
        }
        cell
    }
}

#[derive(Default)]
struct DriveOutcome {
    artifact_confirmed: bool,
}

// ---------------------------------------------------------------------------
// WS 驱动辅助。
// ---------------------------------------------------------------------------

struct LiveWs {
    inner: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
}

impl LiveWs {
    fn new(inner: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>) -> Self {
        Self { inner }
    }

    async fn recv_json(&mut self) -> Result<Option<Value>, String> {
        loop {
            match self.inner.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        return Ok(Some(value));
                    }
                }
                Some(Ok(Message::Ping(_))) => continue,
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Err(error.to_string()),
            }
        }
    }

    async fn send_json(&mut self, value: &Value) -> Result<(), String> {
        let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
        self.inner
            .send(Message::Text(text.into()))
            .await
            .map_err(|error| error.to_string())
    }
}

// ---------------------------------------------------------------------------
// HTTP/fixture/杂项辅助。
// ---------------------------------------------------------------------------

async fn request_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn expect_ok(
    status: StatusCode,
    body: &Value,
    context: &str,
    stage: Option<&str>,
) -> Result<(), LiveMatrixFailure> {
    if status.is_success() {
        Ok(())
    } else {
        Err(matrix_failure(
            "http_request_failed",
            format!("{context} 失败({status}):{body}"),
            stage,
        ))
    }
}

/// 成员仓规则材料播种:`.claude/rules/language.md`(合法 UTF-8;随
/// `git add .` 进入 fixture 首次提交)。真实 LC 成员仓自带该文件,
/// admission 的 missing_member_rules 门任何 phase 均阻断。
fn seed_member_rule_material(path: &Path, member: &str) {
    let rule_dir = path.join(".claude/rules");
    std::fs::create_dir_all(&rule_dir).expect("create member rule dir");
    std::fs::write(
        rule_dir.join("language.md"),
        format!(
            "# {member} 语言规则\n\n本成员仓为 Rust 仓:公开接口附英文文档注释;\n变更须保持格式化与编译通过;禁止引入未审计依赖。\n"
        ),
    )
    .expect("write member language rule");
}

/// 递归复制目录(诊断抄录用;缺失源返回 Ok(0))。
fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<usize> {
    let mut copied = 0usize;
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    std::fs::create_dir_all(destination)?;
    for entry in entries.flatten() {
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn git_repo_at(path: &Path) {
    std::fs::create_dir_all(path).expect("create repo dir");
    run_git(path, &["init", "-q"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
}

fn run_git(path: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(arguments)
        .current_dir(path)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?} failed");
}

fn commit(path: &Path, message: &str) {
    std::fs::write(path.join("lib.rs"), message).expect("write");
    run_git(path, &["add", "."]);
    run_git(
        path,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

fn git_head(path: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_snapshot(path: &Path) -> Value {
    let porcelain = std::process::Command::new("git")
        .args(["status", "--porcelain", "-b"])
        .current_dir(path)
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    json!({
        "head": git_head(path),
        "status": porcelain,
    })
}

fn env_timeout_secs(name: &str, default_secs: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default_secs)
}

fn provider_type_for(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Pi => ProviderType::Pi,
        ProviderName::KimiCode => ProviderType::KimiCode,
        ProviderName::Fake => ProviderType::Fake,
    }
}

/// F6:tool 事件结构化判定——按事件 type 字段精确匹配(含 permission/approval
/// 审批面),`execution_event` 仅在其 title/agent 字段携带 tool 语义时计入;
/// 不做全文子串匹配。
fn is_tool_event(event: &Value) -> bool {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "tool_call"
        | "tool_result"
        | "tool_use"
        | "tool_use_result"
        | "tool_update"
        | "permission_request"
        | "coding_permission_request"
        | "approval_request" => true,
        "execution_event" => {
            let title = event
                .pointer("/event/title")
                .or_else(|| event.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let title_lower = title.to_ascii_lowercase();
            title_lower.starts_with("tool_") || title == "task_update"
        }
        _ => false,
    }
}

/// F3:从 stream log 目录解析 provider 子进程 PID(文件名
/// `{program}-{pid}-{stdout|stderr}.log`;取最新修改的 stdout)。
fn scan_stream_log_pid(directory: &Path) -> Option<String> {
    let entries = std::fs::read_dir(directory).ok()?;
    let mut newest: Option<(std::time::SystemTime, String)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // 文件名形如 `{program}-{pid}-{stream}.log`:尾部取 stream,再取 pid。
        let Some(stem) = name.strip_suffix(".log") else {
            continue;
        };
        let mut parts = stem.rsplitn(3, '-');
        let stream = parts.next().unwrap_or_default();
        let pid = parts.next().unwrap_or_default();
        if stream != "stdout" || pid.is_empty() || !pid.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if newest.as_ref().is_none_or(|(best, _)| modified >= *best) {
            newest = Some((modified, pid.to_string()));
        }
    }
    newest.map(|(_, pid)| pid)
}

fn provider_matches_record(provider: &ProviderName, record: &ProviderStartAudit) -> bool {
    let snake = serde_json::to_value(provider)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    // 审计 provider 为 canonical 序列(dash 形态,如 claude-code)。
    record.provider == snake || record.provider == snake.replace('_', "-")
}

/// split_sync prompt:harness 携带的真实拆分指令(结构化输出按
/// `WORK_ITEM_SPLIT_OUTPUT_SCHEMA` 契约;产物经 gateway sync bridge 解析)。
fn split_sync_prompt() -> String {
    "你是跨仓逻辑代码库的 work item 拆分引擎。阅读聚合根下成员仓的公开接口与分层,\n\
     按输出 schema 把本次修复拆为 1-3 个可独立交付的 work item(含 title/kind/\n\
     sequence_hint/depends_on/exclusive_write_scopes),以结构化 JSON 输出。\n\
     只读规划:不写任何文件。"
        .to_string()
}

/// `WORK_ITEM_SPLIT_OUTPUT_SCHEMA` 为 `pub(crate)`,跨 crate 不可引用;
/// 编译期从生产源文件嵌入冻结常量原文,消除镜像漂移(格式变化时 expect
/// 在测试现场暴露,而非静默漂移)。
fn work_item_split_output_schema() -> String {
    let source = include_str!("../../../src/product/work_item_split_engine/schema.rs");
    let start = source.find("r#\"").expect("split schema raw start") + 3;
    let end = source[start..].find("\"#;").expect("split schema raw end") + start;
    source[start..end].to_string()
}

/// 把 `connect_async` 的裸流包装为 JSON 帧驱动(与 LiveWs 方法配套)。
async fn connect_live_ws(url: &str) -> Result<LiveWs, String> {
    let (stream, _) = connect_async(url)
        .await
        .map_err(|error| error.to_string())?;
    Ok(LiveWs::new(stream))
}
