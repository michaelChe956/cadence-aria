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
use cadence_aria::product::logical_codebase::provider_boundary_probe::{
    ProviderBoundaryProbe, ResumeChannelKind, ResumeProbeSpec, run_cli_boundary_probe,
};
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

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
const DEFAULT_STAGE_TIMEOUT_SECS: u64 = 3600;
/// 实体会话长阶段(story/design/plan/review:多轮真实交互+长生成)独立
/// 放宽——r12 现场 story fresh 3600s 仅差数分钟,r13 起语义应答减轮次后
/// 仍需覆盖真实 CLI 长思考窗口。
const ENTITY_STAGE_TIMEOUT_ENV: &str = "LC_GATEWAY_E2E_ENTITY_STAGE_TIMEOUT_SECS";
const DEFAULT_ENTITY_STAGE_TIMEOUT_SECS: u64 = 5400;
/// 聚合初始化(root recipe 五步四命令)整体超时。
const INIT_TIMEOUT_ENV: &str = "LC_GATEWAY_E2E_INIT_TIMEOUT_SECS";
const DEFAULT_INIT_TIMEOUT_SECS: u64 = 7200;
/// WS 空闲保活间隔(真实 CLI 长思考期间持续 ping 防 server idle 断连)。
const IDLE_PING_SECS: Duration = Duration::from_secs(30);
/// provider health 就绪等待(真实 CLI 探测刷新)。
const HEALTH_TIMEOUT_ENV: &str = "LC_GATEWAY_E2E_HEALTH_TIMEOUT_SECS";
const DEFAULT_HEALTH_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// r20 黑匣子足迹:全驱动链(阶段/HTTP/WS/轮询)一行足迹 + 关键轮询 30s 心跳。
//
// r16~r20 同位置变体复盘:现场挂死后唯一可观测信号是「无输出/无子进程/
// 线程 park」,格证据只在矩阵收尾落盘,卡点不可定位。足迹打到 stderr,
// 挂死现场日志最后一行=精确卡点;「静默正常」与「挂死」以 30s 心跳区分。
// 相位(阶段/入口/fresh_or_resume)随格推进全局切换,HTTP/WS/轮询足迹
// 自动携带,无需逐调用点透传上下文。
// ---------------------------------------------------------------------------

/// 当前格相位 `stage/entrypoint/fresh_or_resume`(进程内单写;临界区仅
/// 赋值/克隆,同步微段,无跨 await 持锁)。
static FP_PHASE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// 打一行足迹:`[fp HH:MM:SS.mmm] [phase] event detail`。成本一行 stderr,
/// 可忽略;现场以最后一行定位卡点。
fn fp(event: &str, detail: impl std::fmt::Display) {
    let phase = FP_PHASE
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    eprintln!(
        "[fp {}] [{}] {} {}",
        Utc::now().format("%H:%M:%S%.3f"),
        phase,
        event,
        detail
    );
}

/// 切换当前格相位并留痕(后续 HTTP/WS/轮询足迹自动携带新相位)。
fn fp_enter_phase(stage: &str, entrypoint: &str, fresh_or_resume: &str) {
    let next = format!("{stage}/{entrypoint}/{fresh_or_resume}");
    if let Ok(mut guard) = FP_PHASE.lock() {
        *guard = next.clone();
    }
    fp("phase_begin", &next);
}

/// 轮询心跳节流:距上次足迹 ≥30s 才再打一行(防「静默正常/挂死」不可分)。
const FP_HEARTBEAT_SECS: Duration = Duration::from_secs(30);

/// WS 单帧发送有界超时:r20 候选③——`send_json` 原为无超时裸 await(对端
/// 停读+缓冲塞满时永久挂起,无 timer 可观测)。小帧 60s 已远超正常完成。
const WS_SEND_TIMEOUT_SECS: Duration = Duration::from_secs(60);

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
    /// 会话级全投影摘要(含 role/target/trust;审计记录,非相等断言维度)。
    pub session_projection_digest: String,
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
            "session_projection_digest": self.session_projection_digest,
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
        fp_enter_phase("matrix", "env", "build");
        let mut env = MatrixEnvironment::build(provider.clone(), evidence_root).await?;
        fp("env_built", format_args!("lc={} issue={}", env.lc_id, env.issue_id));

        let mut cells = Vec::new();
        // 五阶段顺序固定:Story、Design、Plan、Coding、Review。
        fp("stage_begin", "story");
        cells.extend(
            env.run_workspace_entity_stage(
                "story",
                &format!(
                    "/api/projects/{PROJECT_ID}/issues/{}/story-specs:generate",
                    env.issue_id
                ),
                // r23:story 同样钉定单成员 involved——产品面(r23 根修)对钉定
                // involved 派生 focus=首成员,story 首轮 launch 即锚 alpha 成员
                // checkout;不钉则首轮锚聚合根视图,AI 回写 involved/focus 后
                // 修订轮 target 三元组+git identity 漂移,按设计 supersede
                // (resume_fingerprint_mismatch,r23 现场),原生恢复不可达。
                pinned_story_generate_body(
                    "矩阵 story:alpha 仓会话过期提示",
                    &env.member_logical_id,
                    &env.provider_wire,
                ),
                "story_specs",
            )
            .await,
        );
        fp("stage_end", format_args!("story cells={}", cells.len()));
        fp("stage_begin", "design");
        cells.extend(
            env.run_workspace_entity_stage(
                "design",
                &format!(
                    "/api/projects/{PROJECT_ID}/issues/{}/design-specs:generate",
                    env.issue_id
                ),
                pinned_design_generate_body(
                    "矩阵 design:alpha 仓会话过期后端设计",
                    &env.story_spec_ids(),
                    &env.member_logical_id,
                    &env.provider_wire,
                ),
                "design_specs",
            )
            .await,
        );
        fp("stage_end", format_args!("design cells={}", cells.len()));
        // Plan:workspace streaming 主入口(start_work_item_plan_author caller 链)。
        fp("stage_begin", "plan streaming");
        cells.extend(env.run_plan_streaming_stage().await);
        fp("stage_end", format_args!("plan streaming cells={}", cells.len()));
        // Plan:split_sync 对照(gateway sync bridge)。
        fp("stage_begin", "plan split_sync");
        cells.extend(env.run_split_sync_stage().await);
        fp("stage_end", format_args!("plan split_sync cells={}", cells.len()));
        // Coding(依赖 Plan 确认后的 work item;失败落格)。
        fp("stage_begin", "coding");
        cells.extend(env.run_coding_stage().await);
        fp("stage_end", format_args!("coding cells={}", cells.len()));
        // Review:reviewer 角色经 streaming 栈的真实评审会话。
        fp("stage_begin", "review");
        cells.extend(env.run_review_stage().await);
        fp("stage_end", format_args!("review cells={}", cells.len()));

        fp_enter_phase("matrix", "validate", "final");
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
    plan_work_item_ids: Vec<String>,
    /// r37:streaming plan 的 durable plan id(prepare 响应携带;coding 组
    /// 入口 POST work-item-plans/{plan_id}/coding-attempts 消费)。
    plan_id: String,
    prior_entity_session_id: Option<String>,
    prior_plan_session_id: Option<String>,
    prior_coding_session_id: Option<String>,
    prior_review_session_id: Option<String>,
    evidence_root: PathBuf,
    stage_timeout: Duration,
    entity_stage_timeout: Duration,
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
            plan_work_item_ids: Vec::new(),
            plan_id: String::new(),
            prior_entity_session_id: None,
            prior_plan_session_id: None,
            prior_coding_session_id: None,
            prior_review_session_id: None,
            evidence_root: evidence_root.to_path_buf(),
            stage_timeout: Duration::from_secs(env_timeout_secs(
                STAGE_TIMEOUT_ENV,
                DEFAULT_STAGE_TIMEOUT_SECS,
            )),
            entity_stage_timeout: Duration::from_secs(env_timeout_secs(
                ENTITY_STAGE_TIMEOUT_ENV,
                DEFAULT_ENTITY_STAGE_TIMEOUT_SECS,
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
        // 6) r25 接线:capability 真实验证导入(索引 ready 后、五阶段前)。
        //    bootstrap 只落全 Unknown:launch/write_boundary 靠 2b 过渡桥
        //    放行,resume 无桥(gateway 仅 Confirmed 放行)——story/design
        //    门上修订(require_resume_supported)必须由 6c 真实探针证据经
        //    2d 导入后才可达。失败=BLOCKED 真实报告,不伪造。
        env.seed_provider_capability_probe().await?;
        Ok(env)
    }

    /// r25:对所选 provider 现场执行 6c 真实边界探针(Coding/Planning 两
    /// action,含 ResumeProbeSpec 的 resume 面:真实 launch→同 id resume→
    /// 错 id 负探针),经 2d `record_verified_probe` 导入 durable Confirmed
    ///(与 gateway 消费的 capability store 同一 LC 作用域,工厂
    /// `durable_probe_writer_for_lc` 装配)。证据落 `evidence_root/boundary/`。
    async fn seed_provider_capability_probe(&mut self) -> Result<(), LiveMatrixFailure> {
        fp_enter_phase("env", "capability_probe", "-");
        let Some((cli_program, resume_kind)) = provider_probe_channel(&self.provider) else {
            let failure = matrix_failure(
                "capability_probe_provider_unsupported",
                format!("provider {:?} 无真实探针通道(四家之外不冒充)", self.provider),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        };
        let durable = match self
            .gateway_factory
            .durable_probe_writer_for_lc(PROJECT_ID, Some(&self.lc_id))
        {
            Ok(writer) => writer,
            Err(error) => {
                let failure = matrix_failure(
                    "capability_probe_writer_unavailable",
                    format!("durable probe writer 装配失败:{error:?}"),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
        };
        let boundary_root = self.evidence_root.join("boundary");
        let probe_base = TempDir::new().expect("capability probe base dir");
        for action in [
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::PlanningReadOnly,
        ] {
            let label = format!(
                "matrix-{}-{}",
                matrix_action_text(action),
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
            );
            let probe = run_cli_boundary_probe(
                self.provider.clone(),
                cli_program,
                action,
                probe_base.path(),
                &boundary_root,
                &label,
                Some(ResumeProbeSpec::new(
                    resume_kind,
                    "Reply with exactly: matrix-resume-probe",
                )),
            )
            .await;
            let outcome = match probe {
                Ok(pair) => pair,
                Err(error) => {
                    // BLOCKED 真实报告:不伪造 Confirmed,不让矩阵继续裸跑。
                    let failure = matrix_failure(
                        "capability_probe_failed",
                        format!(
                            "provider {:?} action {:?} 真实探针失败:{error:?}",
                            self.provider, action
                        ),
                        None,
                    );
                    return Err(self.fail_with_diagnostics(failure, None).await);
                }
            };
            let evidence = outcome.evidence;
            let write_state = ProviderBoundaryProbe::evidence_state(&Ok(evidence.clone()));
            // 6c worker 提醒:record 行 resume 格取 artifact_resume_state 结果
            //(工件签发,非自报;材料包 record 同口径构造)。
            let resume_state = ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref());
            if write_state != ProviderCapabilityEvidence::Confirmed
                || resume_state != ProviderCapabilityEvidence::Confirmed
            {
                let failure = matrix_failure(
                    "capability_probe_not_confirmed",
                    format!(
                        "provider {:?} action {:?} 探针未签 Confirmed(write={:?} resume={:?};artifact={})",
                        self.provider,
                        action,
                        write_state,
                        resume_state,
                        evidence.artifact_ref()
                    ),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
            // 2d 导入:三方一致(2c shape + 逐字段)通过才落 durable。record
            // 由产品 seam 材料包签发(write_boundary=Confirmed、resume=工件
            // 签发结果、launch=Unknown 由 2b 过渡桥继续覆盖),harness 不自
            // 造 record 身份字段(投影 getter 冻结为 crate 内)。
            durable
                .record_verified_probe(PROJECT_ID, &outcome.record, &evidence, &outcome.projection)
                .map_err(|error| {
                    matrix_failure(
                        "capability_probe_import_rejected",
                        format!("provider {:?} action {:?} 2d 导入被拒:{error:?}", self.provider, action),
                        None,
                    )
                })?;
            fp(
                "capability_probe_imported",
                format_args!(
                    "action={} resume=Confirmed artifact={}",
                    matrix_action_text(action),
                    evidence.artifact_ref()
                ),
            );
        }
        Ok(())
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
        fp_enter_phase("env", "provider_health", "-");
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(env_timeout_secs(
                HEALTH_TIMEOUT_ENV,
                DEFAULT_HEALTH_TIMEOUT_SECS,
            ));
        let mut last_heartbeat = std::time::Instant::now();
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
                fp("health_ready", &self.provider_wire);
                return Ok(());
            }
            if last_heartbeat.elapsed() >= FP_HEARTBEAT_SECS {
                fp("health_poll", "provider health 未就绪,继续轮询(30s 心跳)");
                last_heartbeat = std::time::Instant::now();
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
        fp_enter_phase("env", "create_project_and_lc", "-");
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
        fp("project_lc_created", format_args!("lc={}", self.lc_id));
        Ok(())
    }

    async fn register_members(&mut self) -> Result<(), LiveMatrixFailure> {
        fp_enter_phase("env", "register_members", "-");
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
        let Some(preflight_id) = body["preflight_id"].as_str() else {
            let failure = matrix_failure(
                "preflight_id_missing",
                format!("登记 preflight 响应缺 preflight_id:{body}"),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        };
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
        fp("members_registered", "");
        Ok(())
    }

    /// 固定 Claude recipe 的真实聚合初始化(产品确认走真实 HTTP gate)。
    /// 真实聚合初始化(固定 Claude recipe):对概率性 receipt Rejected
    /// (现场 /rule-config 审计拒绝两轮复现,产品标 retryable)按新产品
    /// operation 有界重试——每轮失败先抄录该轮 receipts 到
    /// diagnostics/attempt-N/,拒绝轮证据全留;最终失败才保留 tempdir。
    async fn run_real_aggregate_initialization(&mut self) -> Result<(), LiveMatrixFailure> {
        fp_enter_phase("env", "aggregate_init", "-");
        const MAX_ATTEMPTS: usize = 3;
        let mut last_failure = None;
        for attempt in 1..=MAX_ATTEMPTS {
            match self.run_initialization_once(attempt).await {
                Ok(()) => return Ok(()),
                Err(failure) => {
                    // 抄录该轮 receipts(不 keep tempdir,可继续重试)。
                    let diagnostics_dir = self.evidence_root.join("diagnostics");
                    let _ = std::fs::create_dir_all(&diagnostics_dir);
                    let receipts_src = self
                        .app_paths
                        .logical_codebases_root(PROJECT_ID)
                        .join(&self.lc_id)
                        .join("aggregate-recipe-receipts");
                    let _ = copy_tree(
                        &receipts_src,
                        &diagnostics_dir.join(format!("attempt-{attempt}")),
                    );
                    let retryable =
                        failure.reason_code == "initialization_failed" && attempt < MAX_ATTEMPTS;
                    if !retryable {
                        last_failure = Some(failure);
                        break;
                    }
                    last_failure = Some(failure);
                }
            }
        }
        let failure = last_failure.expect("initialization attempts exhausted");
        Err(self.fail_with_diagnostics(failure, None).await)
    }

    async fn run_initialization_once(&mut self, attempt: usize) -> Result<(), LiveMatrixFailure> {
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations",
                self.lc_id
            ),
            json!({"idempotency_key": format!("lcg-matrix-{}-a{attempt}", self.lc_id)}),
        )
        .await;
        if status != StatusCode::ACCEPTED {
            let failure = matrix_failure(
                "initialization_rejected",
                format!("初始化启动被拒({status}):{body}"),
                None,
            );
            return Err(failure);
        }
        let Some(operation_id) = body["operation_id"].as_str() else {
            let failure = matrix_failure(
                "operation_id_missing",
                format!("初始化响应缺 operation_id:{body}"),
                None,
            );
            return Err(failure);
        };
        let operation_id = operation_id.to_string();
        let init_timeout = Duration::from_secs(env_timeout_secs(
            INIT_TIMEOUT_ENV,
            DEFAULT_INIT_TIMEOUT_SECS,
        ));
        let uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations/{operation_id}",
            self.lc_id
        );
        let deadline = tokio::time::Instant::now() + init_timeout;
        let mut last_heartbeat = std::time::Instant::now();
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(matrix_failure(
                    "initialization_timeout",
                    format!("聚合初始化超时({init_timeout:?});环境不可运行须报告 BLOCKED"),
                    None,
                ));
            }
            let (status, snapshot) = request_json(&self.app, Method::GET, &uri, json!({})).await;
            expect_ok(status, &snapshot, "轮询初始化", None)?;
            match snapshot["status"].as_str() {
                Some("completed") => {
                    fp("init_completed", format_args!("operation={operation_id}"));
                    return Ok(());
                }
                Some("failed") | Some("cancelled") => {
                    return Err(matrix_failure(
                        "initialization_failed",
                        format!("聚合初始化 {snapshot}"),
                        None,
                    ));
                }
                _ => {
                    if last_heartbeat.elapsed() >= FP_HEARTBEAT_SECS {
                        fp("init_poll", format_args!("status={} 继续轮询(30s 心跳)", snapshot["status"]));
                        last_heartbeat = std::time::Instant::now();
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// 真实索引 ready(初始化 D5 后 active index 应可用;缺失则 rebuild)。
    async fn wait_for_index_ready(&mut self) -> Result<(), LiveMatrixFailure> {
        fp_enter_phase("env", "index_ready", "-");
        let active_uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes/active",
            self.lc_id
        );
        let rebuild_uri = format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes:rebuild",
            self.lc_id
        );
        let deadline = tokio::time::Instant::now() + self.stage_timeout;
        let mut last_heartbeat = std::time::Instant::now();
        loop {
            let (status, body) = request_json(&self.app, Method::GET, &active_uri, json!({})).await;
            if let Err(failure) = expect_ok(status, &body, "读取 active index", None) {
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
            match body["state"].as_str() {
                // 终态:active 簇 → 通过。
                Some("ready") | Some("active") | Some("completed") => {
                    fp("index_ready", format_args!("state={}", body["state"]));
                    return Ok(());
                }
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
            if last_heartbeat.elapsed() >= FP_HEARTBEAT_SECS {
                fp("index_poll", format_args!("state={} 继续轮询(30s 心跳)", body["state"]));
                last_heartbeat = std::time::Instant::now();
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
        fp_enter_phase("env", "resolve_member_target", "-");
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
        fp_enter_phase("env", "create_issue", "-");
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues"),
            json!({
                // r23 问题2:issue 文案显式 alpha 仓交付域——design 的
                // involved 由 AI 自决回写(prompt 只有成员清单与禁猜规则),
                // r22「跨仓会话过期修复」文案让 AI 如实声明 [alpha,beta],
                // design 修订路由按产品语义 fail-closed(TargetAmbiguous)。
                // 对齐 v1.1 E2E 单成员先例:交付域收窄到 alpha。
                "title": "矩阵 issue:alpha 仓会话过期提示",
                "description": "在 alpha 仓内实现会话过期签发、判定与用户提示;本需求不修改 beta 仓(beta 仅按既有接口消费,不属本需求交付范围)。四家五阶段真实矩阵驱动 issue。",
                "repository_id": self.member_physical_repo_id,
                "logical_codebase_id": self.lc_id,
            }),
        )
        .await;
        if let Err(failure) = expect_ok(status, &body, "创建逻辑 issue", None) {
            return Err(self.fail_with_diagnostics(failure, None).await);
        }
        let Some(issue_id) = body["issue_id"].as_str() else {
            let failure = matrix_failure(
                "issue_id_missing",
                format!("issue 创建响应缺 issue_id:{body}"),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        };
        self.issue_id = issue_id.to_string();
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
    /// r23 问题3 根修:两格共用同一条 WS 连接——真实用户流里门上反馈修订
    /// 不打断连接;跨连接时 manager 随最后一个连接 detach 被回收,engine
    /// new_persistent 重建丢 launch 指纹内存,产品按设计 fail-closed 走
    /// full-prompt fresh(非原生恢复,r22 story 实测无 --resume 新会话)。
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
        // ---- fresh:生成实体 → WS streaming 驱动 → 停在人工门(不 confirm,
        // 门与连接都留给 resume 格的反馈修订;r21 C1 前 fresh 就地 confirm,
        // Completed 终态上 request_revision 被协议矩阵正确拒收,resume 格
        // 空转到超时)----
        let (fresh, gate_ws) = self
            .drive_entity_session_fresh(stage, generate_uri, &generate_body, response_spec_field)
            .await;
        cells.push(fresh);
        // ---- resume:同一连接门上 request_revision 修订重驱(原生恢复确认)
        // → 回门 → 真实 HTTP confirm 定稿(spec Confirmed 供 plan 前置)----
        let resume = self.drive_entity_session_resume(stage, gate_ws).await;
        cells.push(resume);
        cells
    }

    /// 返回(格, 门上连接):门到且干净收口时保留 WS 供 resume 格同连接修订。
    async fn drive_entity_session_fresh(
        &mut self,
        stage: &'static str,
        generate_uri: &str,
        generate_body: &Value,
        response_spec_field: &str,
    ) -> (EvidenceCell, Option<LiveWs>) {
        let mut observation = StageObservation::new(stage, &self.provider);
        fp_enter_phase(stage, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
        let (status, body) =
            request_json(&self.app, Method::POST, generate_uri, generate_body.clone()).await;
        if !status.is_success() {
            return (
                observation.denied_cell(format!("生成 {stage} 实体失败({status}):{body}")),
                None,
            );
        }
        let session_id = body["workspace_session"]["workspace_session_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            return (
                observation
                    .denied_cell(format!("生成 {stage} 响应缺 workspace_session:{body}")),
                None,
            );
        }
        let spec_id_field = if stage == "story" {
            "story_spec_id"
        } else {
            "design_spec_id"
        };
        let mut spec_captured = false;
        if let Some(spec_id) = body[response_spec_field][0][spec_id_field].as_str() {
            match stage {
                "story" => self.story_spec_id = Some(spec_id.to_string()),
                "design" => self.design_spec_id = Some(spec_id.to_string()),
                _ => {}
            }
            spec_captured = true;
        }
        observation.workspace_session_id = session_id.clone();
        self.prior_entity_session_id = Some(session_id.clone());

        // r21 C1 根修:实体 fresh 以零确认预算停在人工门(author_confirm)。
        // Completed 是终态(spec-design-dialog-revision:确认定稿不再经过
        // 任何中间确认阶段),门上的反馈修订(request_revision)才是
        // revision/resume 格的产品入口;fresh 若就地 confirm,resume 格的
        // request_revision 会被协议矩阵正确拒收(Completed 只放行 SC
        // Advance),r21 现场 story/design resume 双双 53-90min 空转即此。
        // 定稿 confirm 由 resume 格在修订轮回门后执行(修订→回门→确认)。
        let (drive, ws) = self
            .drive_workspace_session_ws(&session_id, &mut observation, 0, self.entity_stage_timeout)
            .await;
        // 产物=author 轮已生成的实体 spec(generate 响应携带 spec id),
        // gate_reached 证明 author run 完整驱动到门。
        observation.completed_product_artifact_exists = spec_captured && drive.gate_reached;
        fp(
            "entity_fresh_drive_end",
            format_args!(
                "session={session_id} spec_captured={spec_captured} gate_reached={}",
                drive.gate_reached
            ),
        );
        // 门到且无失败才保留连接(失败路径连接状态未知,交由 resume 格
        // 落可诊断拒绝,不带病复用)。
        let gate_ws = (drive.gate_reached && observation.run_failure.is_none())
            .then_some(ws)
            .flatten();
        (observation.build_cell(self), gate_ws)
    }

    /// 同一会话的显式 revision 重驱:gateway resume 路径的原生恢复确认。
    /// r23 问题3:修订在 fresh 轮保留下来的同一条连接上发起——同 engine
    /// 实例保有 launch 指纹与 provider_conversations,revision 经
    /// gateway.resume_or_start 走原生恢复(--resume 同一 native id)。
    async fn drive_entity_session_resume(
        &mut self,
        stage: &'static str,
        gate_ws: Option<LiveWs>,
    ) -> EvidenceCell {
        let mut observation = StageObservation::new(stage, &self.provider);
        fp_enter_phase(stage, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
        // fix 轮 8:resume 格先定 mode 再早退——早退分支曾以 fresh 落盘,
        // 覆盖 fresh 格目录并掩埋其真实失败原因。
        observation.force_resume = true;
        let Some(session_id) = self.prior_entity_session_id.clone() else {
            return observation
                .denied_cell("无可恢复的本阶段 fresh 会话(fresh 未建立会话)".to_string());
        };
        let Some(mut ws) = gate_ws else {
            return observation.denied_cell(
                "fresh 轮未达人工门(无干净的同连接会话可供门上修订)".to_string(),
            );
        };
        observation.workspace_session_id = session_id.clone();
        fp("resume_audit_scan_begin", format_args!("session={session_id}"));
        // 请求恢复的 native id = fresh 轮审计里的 provider_session_id
        //(必须在 revision 重驱前捕获)。
        observation.requested_resume_id =
            self.latest_audit_native_id(&session_id, &self.provider, None);
        observation.frozen_digest =
            self.latest_audit_projection_digest(&session_id, &self.provider);
        fp(
            "resume_audit_scan_end",
            format_args!("requested_id={:?} frozen={:?}", observation.requested_resume_id, observation.frozen_digest),
        );
        let revision = json!({
            "type": "request_revision",
            "feedback": {
                "feedback_types": ["scope"],
                "description": "矩阵 resume:显式修订重驱",
                "target_artifact_version": null
            }
        });
        if let Err(error) = ws.send_json(&revision).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("request_revision 发送失败:{error}"));
            return observation.build_cell(self);
        }
        let revision = self
            .pump_workspace_session(&mut ws, &mut observation, 1, self.entity_stage_timeout)
            .await;
        observation.completed_product_artifact_exists = revision.artifact_confirmed;
        // 原生恢复确认:revision 轮审计的 provider_session_id == 请求 id。
        observation.native_confirmed_id =
            self.latest_audit_native_id(&session_id, &self.provider, None);
        observation.build_cell(self)
    }

    /// revision 重驱公共路径:连接会话 WS → request_revision → pump。
    /// confirm 轮次由调用面给定:实体 resume=1(修订→回门→确认定稿);
    /// lc-root 先例的长门链场景保留更高预算的调用自由。timeout 由调用面
    /// 传入(实体长阶段独立放宽)。
    async fn drive_revision_resume(
        &self,
        session_id: &str,
        observation: &mut StageObservation,
        description: &str,
        confirm_rounds: usize,
        timeout: Duration,
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
        self.pump_workspace_session(&mut ws, observation, confirm_rounds, timeout)
            .await
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
    /// r23:返回(结果, 连接)——实体 fresh 在门上收口后把连接交给 resume 格
    /// 同连接修订;调用方不需要时丢弃即关闭(plan 阶段即此)。
    async fn drive_workspace_session_ws(
        &self,
        session_id: &str,
        observation: &mut StageObservation,
        confirm_rounds: usize,
        timeout: Duration,
    ) -> (DriveOutcome, Option<LiveWs>) {
        let mut ws = match self.connect_session_ws(session_id).await {
            Ok(ws) => ws,
            Err(error) => {
                observation.push_event(json!({"type": "matrix_error", "message": error}));
                observation.run_failure = Some(format!("workspace 会话 WS 连接失败:{error}"));
                return (DriveOutcome::default(), None);
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
                "review_rounds": 1
            },
            "reviewer_enabled": false
        });
        if let Err(error) = ws.send_json(&start).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("start_generation 发送失败:{error}"));
            return (DriveOutcome::default(), None);
        }
        let outcome = self
            .pump_workspace_session(&mut ws, observation, confirm_rounds, timeout)
            .await;
        (outcome, Some(ws))
    }

    /// 真实确认门:POST /confirm(引擎裁决端点),按响应 DTO status 判定
    /// 终态收口。confirmed → 产物确认并收口;评审接管等中间态 → 继续泵送;
    /// 拒绝 → 拒绝事件落格继续泵送(不旁路产品决策面)。
    async fn confirm_session_gate(
        &self,
        observation: &mut StageObservation,
        outcome: &mut DriveOutcome,
    ) -> bool {
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
        if !status.is_success() {
            observation.push_event(json!({
                "type": "matrix_confirm_rejected",
                "status": status.as_u16(),
                "body": body
            }));
            return false;
        }
        let dto_status = body
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        observation.push_event(json!({
            "type": "matrix_confirm_result",
            "status": dto_status,
        }));
        fp("confirm_gate_result", format_args!("status={dto_status}"));
        match dto_status.as_str() {
            "confirmed" => {
                outcome.artifact_confirmed = true;
                true
            }
            "failed" | "terminated" | "stopped_needs_human" | "blocked_provider_unavailable" => {
                observation.terminal_status = Some(dto_status);
                true
            }
            _ => false,
        }
    }

    /// 泵送会话事件直至终态;waiting_for_human 时走真实 HTTP confirm 门。
    /// timeout 由调用面区分:实体长阶段(story/design/plan/review)独立放宽。
    async fn pump_workspace_session(
        &self,
        ws: &mut LiveWs,
        observation: &mut StageObservation,
        confirm_rounds: usize,
        timeout: Duration,
    ) -> DriveOutcome {
        let mut outcome = DriveOutcome::default();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
        let mut confirms_left = confirm_rounds;
        let pump_started = std::time::Instant::now();
        let mut events_seen = 0u64;
        fp("pump_begin", format_args!("timeout={timeout:?} confirms={confirm_rounds}"));
        loop {
            if tokio::time::Instant::now() >= deadline {
                // r21 C1 同族盲区:该分支此前不落 run_failure——story resume
                // 空转 5400s 后仍以"通过"建格,正是 r21 误判「story resume 过」
                // 的另一半原因。阶段超时=驱动未达终态,必须落格失败原因。
                observation.push_event(json!({"type": "matrix_stage_timeout"}));
                observation.run_failure =
                    Some("workspace 会话阶段超时未达终态(对端零响应或挂死)".to_string());
                fp("pump_stage_timeout", format_args!("elapsed={:?} events={events_seen}", pump_started.elapsed()));
                return outcome;
            }
            let message = tokio::time::timeout_at(idle_deadline, ws.recv_json()).await;
            let message = match message {
                Ok(Ok(Some(value))) => {
                    idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
                    value
                }
                Ok(Ok(None)) => {
                    observation.push_event(json!({"type": "matrix_ws_closed"}));
                    // F5:驱动失败原因必须落在格上,不得被 resume 兜底文案覆盖。
                    observation.run_failure = Some("workspace 会话 WS 在终态前关闭".to_string());
                    fp("pump_ws_closed", format_args!("elapsed={:?} events={events_seen}", pump_started.elapsed()));
                    return outcome;
                }
                Ok(Err(error)) => {
                    observation.push_event(json!({"type": "matrix_ws_error", "message": error}));
                    observation.run_failure = Some(format!("workspace 会话 WS 错误:{error}"));
                    return outcome;
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    // 空闲保活:真实 CLI 长思考期间持续 ping,防 server idle 断连。
                    // r12 复盘:此处不重置 idle_deadline 会退化成 ping 风暴
                    //(过期 deadline 使 timeout_at 立即 Err,循环狂发 ping,
                    // r12 证据里 128 pong 同毫秒突发即此因),必须按间隔节流。
                    fp(
                        "pump_idle_ping",
                        format_args!("elapsed={:?} events={events_seen}(30s 心跳:泵存活/对端静默)", pump_started.elapsed()),
                    );
                    if ws.send_json(&json!({"type": "ping"})).await.is_err() {
                        observation.push_event(json!({"type": "matrix_ws_closed"}));
                        observation.run_failure =
                            Some("workspace 会话 WS 在终态前关闭".to_string());
                        return outcome;
                    }
                    idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
                    continue;
                }
                Err(_) => {
                    observation.push_event(json!({"type": "matrix_stage_timeout"}));
                    observation.run_failure = Some("workspace 会话阶段超时未达终态".to_string());
                    fp("pump_stage_timeout", format_args!("elapsed={:?} events={events_seen}", pump_started.elapsed()));
                    return outcome;
                }
            };
            let kind = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            observation.push_event(message.clone());
            events_seen += 1;
            // F6:tool 事件结构化计数(type/字段精确匹配,非全文子串)。
            if is_tool_event(&message) {
                observation.tool_events += 1;
            }
            // ---- 副作用层(r20 前语义不变):状态留痕/choice 语义应答/审批计数
            // ---- + r27 问题1:产物 markdown 在场捕获 ----
            if frame_carries_artifact_markdown(&kind, &message) {
                observation.artifact_markdown_seen = true;
            }
            match kind.as_str() {
                "session_state" => {
                    if let Some(status) = message.get("status").and_then(Value::as_str) {
                        observation.record_status(status);
                        fp("pump_session_state", format_args!("status={status} events={events_seen}"));
                    }
                }
                "stage_change" => {
                    // r13 复盘:产物就绪后产品端广播的是 stage_change(
                    // author_confirm/human_confirm)+message_complete,并不推
                    // session_state waiting_for_human——人工门就绪必须以
                    // stage_change 为准(r13 会话 00:14 已 waiting_for_human,
                    // 泵只认 session_state 空转到 90min 超时)。
                    let stage = message.get("stage").and_then(Value::as_str).unwrap_or("");
                    fp("pump_stage_change", format_args!("stage={stage}"));
                }
                "choice_request" => {
                    // 真实 choice 门:读选项文本语义应答,不旁路产品决策面。
                    // r12 复盘:恒 opt_0 非语义答案触发 provider 反复追问
                    //(lc-root 先例 story 13 轮确认);且仅答 questions/0 时
                    // 多题请求其余题悬空同样诱发重问。逐题 answers 为准
                    //(P0 1.3:旧单题字段会被引擎清空)。
                    if let Some(choice_id) = message.get("id").and_then(Value::as_str) {
                        let (answers, top_selected) = semantic_choice_answers(&message);
                        fp("pump_choice_request", format_args!("id={choice_id} options={}", top_selected.join(",")));
                        let _ = ws
                            .send_json(&json!({
                                "type": "choice_response",
                                "id": choice_id,
                                "selected_option_ids": top_selected,
                                "free_text": null,
                                "answers": answers
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
            // ---- 处置层(r21 C1 根修):拒收帧秒收口;门到无预算=门上完成 ----
            match pump_frame_disposition(&kind, &message, confirms_left) {
                PumpFrameDisposition::Listen => {}
                PumpFrameDisposition::ConfirmGate => {
                    confirms_left -= 1;
                    if self.confirm_session_gate(observation, &mut outcome).await {
                        return outcome;
                    }
                }
                PumpFrameDisposition::GateReached => {
                    // 门到且无确认预算:驱动在门上完成(不 confirm)。实体
                    // fresh 以零预算停在此处,把 author_confirm 门留给
                    // resume 格的 request_revision(修订→回门→再确认)。
                    outcome.gate_reached = true;
                    fp(
                        "pump_gate_reached",
                        format_args!(
                            "elapsed={:?} events={events_seen}(门到且无确认预算,驱动完成)",
                            pump_started.elapsed()
                        ),
                    );
                    return outcome;
                }
                PumpFrameDisposition::Confirmed => {
                    // confirmed 是终态:确认门已过,立即收口,
                    // 不再空转到阶段超时。
                    outcome.artifact_confirmed = true;
                    fp("pump_confirmed", format_args!("elapsed={:?}", pump_started.elapsed()));
                    return outcome;
                }
                PumpFrameDisposition::Terminal(status) => {
                    observation.terminal_status = Some(status.to_string());
                    fp("pump_terminal", format_args!("status={status} elapsed={:?}", pump_started.elapsed()));
                    return outcome;
                }
                PumpFrameDisposition::ServerError => {
                    // r21 根修:服务端拒收/错误帧立即收口落格(带帧原文),
                    // 不再与死锁同形地空转到阶段超时(r21 现场 story 90min/
                    // design 53min 即此盲区)。
                    fp(
                        "pump_server_error",
                        format_args!("elapsed={:?} frame={message}", pump_started.elapsed()),
                    );
                    observation.run_failure =
                        Some(format!("服务端错误帧收口(kind={kind}):{message}"));
                    return outcome;
                }
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
            let mut plan_resume = StageObservation::new("plan", &self.provider);
            plan_resume.force_resume = true;
            cells.push(
                plan_resume
                    .denied_cell("前置 story/design 未确认,plan resume 无会话可恢复".to_string()),
            );
            return cells;
        }
        // ---- fresh:prepare plan 会话(真实 start_work_item_plan_author
        // caller 链在 WS author 驱动里执行:begin handle→bind sink→start→
        // parse/complete)----
        let mut observation = StageObservation::new("plan", &self.provider);
        fp_enter_phase("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
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
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": true
            }),
        )
        .await;
        if !status.is_success() {
            cells.push(observation.denied_cell(format!("plan prepare 失败({status}):{body}")));
            let mut plan_resume = StageObservation::new("plan", &self.provider);
            plan_resume.force_resume = true;
            cells.push(plan_resume.denied_cell("plan fresh 未启动,resume 无会话".to_string()));
            return cells;
        }
        let session_id = body["workspace_session"]["workspace_session_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            cells.push(observation.denied_cell(format!("plan prepare 缺会话:{body}")));
            let mut plan_resume = StageObservation::new("plan", &self.provider);
            plan_resume.force_resume = true;
            cells.push(plan_resume.denied_cell("plan fresh 未启动,resume 无会话".to_string()));
            return cells;
        }
        observation.workspace_session_id = session_id.clone();
        self.prior_plan_session_id = Some(session_id.clone());
        // 预检:work item 列表直接取 plan DTO 的 work_item_ids(形态已核对),
        // 不依赖列表端点探测。
        if let Some(ids) = body["work_item_plan"]["work_item_ids"].as_array() {
            self.plan_work_item_ids = ids
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect();
        }
        // r37:捕获 durable plan id(coding 组入口 POST
        // work-item-plans/{plan_id}/coding-attempts 消费;schema v2 work
        // item 单件入口被产品正确拒收 schema_v2_group_coding_required)。
        if let Some(plan_id) = body["work_item_plan"]["id"].as_str() {
            self.plan_id = plan_id.to_string();
        }
        // r26 问题1/2:plan fresh 零确认预算停 SC 人工门(human_confirm)。
        // HTTP 会话 confirm 对 WorkItemPlan 恒 500
        // (work_item_plan_confirm_not_supported,r25 现场 18:30 confirm 500
        // 后泵空转 5400s)——SC 门的确认/修订入口是 WS typed 三命令
        // (confirm / human_gate_feedback),与连接一起留给 resume 格。
        let (drive, gate_ws) = self
            .drive_workspace_session_ws(&session_id, &mut observation, 0, self.entity_stage_timeout)
            .await;
        // r27 问题1:产物判定按停门模型差异化——plan 的产物分两级:候选
        // plan markdown(author 轮产出,fresh 格)与 confirmed plan(resume
        // 格 typed confirm 后,work item 才落库)。fresh 判定=产物 markdown
        // 在场(artifact_update/session_state 携带)× gate_reached;此前
        // 依赖 prepare 响应 work_item_ids(r26 实测 prepare 时 work item
        // 尚未产出,ids 恒空)误判"缺完成产物"。
        observation.completed_product_artifact_exists =
            observation.artifact_markdown_seen && drive.gate_reached;
        let gate_ws = (drive.gate_reached && observation.run_failure.is_none())
            .then_some(gate_ws)
            .flatten();
        cells.push(observation.build_cell(self));

        // ---- resume:SC 门 typed feedback 修订 → 回门 → typed confirm 定稿
        //(work item 落库),同连接(与实体阶段同模型)----
        let resume = self.drive_plan_sc_gate_resume(&session_id, gate_ws).await;
        cells.push(resume);
        // Plan confirmed 后解析 work item(coding 前置;r26:从 resume
        // confirm 前移出——fresh 不再 confirm,work item 定稿在 resume 轮)。
        self.resolve_first_work_item().await;
        cells
    }

    /// r26 问题2:SC plan 会话的门上修订重驱。SC human_confirm 门的消息集是
    /// typed 三命令:request_revision 不放行(r25 现场
    /// WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID)。修订入口 =
    /// `human_gate_feedback`(开门 turn→HumanGateScManualRevision run→
    /// `human_gate_turn_completed` 回门),定稿入口 = typed `confirm`
    ///(approve→compile→durable Confirmed+子 WorkItem 落库)。
    /// 在 fresh 轮保留下来的同一条连接上驱动(SC 门快照/引擎内存态保持)。
    async fn drive_plan_sc_gate_resume(
        &mut self,
        session_id: &str,
        gate_ws: Option<LiveWs>,
    ) -> EvidenceCell {
        let mut observation = StageObservation::new("plan", &self.provider);
        fp_enter_phase("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
        observation.role = "work_item_splitter".to_string();
        observation.force_resume = true;
        let Some(mut ws) = gate_ws else {
            return observation
                .denied_cell("plan fresh 未达 SC 人工门(无干净的同连接会话可供门上修订)".to_string());
        };
        observation.workspace_session_id = session_id.to_string();
        observation.requested_resume_id =
            self.latest_audit_native_id(session_id, &self.provider, None);
        observation.frozen_digest = self.latest_audit_projection_digest(session_id, &self.provider);

        // ---- 阶段 1:typed feedback 修订 → 泵至 human_gate_turn_completed ----
        // r33 根修:plan fresh 连续 3 轮 2 中编造 requirement 编号(NFR-1/
        // NFR-2 不在 design 里),教学重驱一轮纠不回——修订反馈文案附
        // confirmed design 的有效需求编号清单(产品 store 读 design 当前
        // 版本正文提取),反馈轮兜底纠偏引用面。
        let requirement_digest = self.design_requirement_ids_digest();
        let feedback_text = if requirement_digest.is_empty() {
            "矩阵 plan resume:按反馈修订拆分方案(补充验收条件与写域口径)".to_string()
        } else {
            format!(
                "矩阵 plan resume:修订拆分方案。当前候选引用了设计中不存在的需求编号。\
                 设计中的有效需求编号只有以下这些,计划任务的 requirement 引用必须且只能\
                 引用它们(不得编造新编号,多余引用一律删除或改为下列编号):\n{requirement_digest}"
            )
        };
        let feedback = json!({
            "type": "human_gate_feedback",
            "command_id": "lcg-matrix-plan-revision",
            "feedback": feedback_text,
        });
        if let Err(error) = ws.send_json(&feedback).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("human_gate_feedback 发送失败:{error}"));
            return observation.build_cell(self);
        }
        let revision_done = self
            .pump_plan_sc_phase(
                &mut ws,
                &mut observation,
                PlanScPumpPhase::AwaitRevisionComplete,
            )
            .await;
        if !revision_done {
            return observation.build_cell(self);
        }
        // ---- 阶段 2:typed confirm → 泵至终态(stage completed/confirmed)。
        // r34:v1.1 缺陷 #12 同族——首拍 approve 可撞 Final Compile
        // recovery_required(「single-candidate approval compile failed; human
        // gate remains open」)。按 v1.1 §5.1 先例续跑:发 WS
        // work_item_plan_compile_recovery_action: continue 步进编译游标,
        // 直至终态或真失败(有界 2 次;非该族失败不重试)。
        let mut recovery_actions_left = 2usize;
        let mut confirm_command = json!({"type": "confirm"});
        let mut confirmed = false;
        loop {
            if let Err(error) = ws.send_json(&confirm_command).await {
                observation.push_event(json!({"type": "matrix_error", "message": error}));
                observation.run_failure = Some(format!("typed confirm 发送失败:{error}"));
                return observation.build_cell(self);
            }
            confirmed = self
                .pump_plan_sc_phase(
                    &mut ws,
                    &mut observation,
                    PlanScPumpPhase::AwaitConfirmTerminal,
                )
                .await;
            if confirmed {
                break;
            }
            let failure_text = observation.run_failure.clone().unwrap_or_default();
            if recovery_actions_left > 0
                && plan_confirm_failure_is_recoverable(&failure_text)
            {
                recovery_actions_left -= 1;
                observation.run_failure = None;
                fp(
                    "plan_sc_compile_recovery_continue",
                    format_args!("left={}", recovery_actions_left + 1),
                );
                confirm_command = json!({
                    "type": "work_item_plan_compile_recovery_action",
                    "action": "continue",
                    "reason": "lcg-matrix: 续跑 Final Compile(v1.1 缺陷 #12 先例)",
                });
                continue;
            }
            break;
        }
        // r35:confirm 失败附 run 预算消耗账(超限族失败的产品正确限制需
        // 带实测数字落格——transitions_used/manual_repairs_used/repairs_
        // 计数 vs RunBudgets 默认 12/3/1 与初评复评各小于等于 1。
        if !confirmed && observation.run_failure.is_some() {
            if let Ok(record) = self.lifecycle.get_workspace_session(session_id) {
                let history = &record.run_history;
                let cycles = history
                    .review_cycles
                    .iter()
                    .map(|(key, cycle)| {
                        format!(
                            "{}[initial={},verification={},repairs={}]",
                            key, cycle.initial_count, cycle.verification_count, cycle.repairs_used
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(";");
                let account = format!(
                    "run 预算消耗账:transitions_used={}/12,manual_repairs_used={}/3,repairs_used={}/1,review_cycles=[{cycles}]",
                    history.transitions_used,
                    history.manual_repairs_used,
                    history.repairs_used
                );
                fp("plan_sc_budget_account", format_args!("{account}"));
                observation.run_failure =
                    Some(format!("{}/{}", observation.run_failure.clone().unwrap_or_default(), account));
            }
        }
        observation.completed_product_artifact_exists = confirmed;
        observation.native_confirmed_id =
            self.latest_audit_native_id(session_id, &self.provider, None);
        observation.build_cell(self)
    }

    /// SC 门两阶段泵:AwaitRevisionComplete 等 `human_gate_turn_completed`
    ///(回门信号);AwaitConfirmTerminal 等 stage_change completed /
    /// session_state confirmed(定稿终态)。通用面:30s 心跳保活、choice
    /// 语义应答、错误帧秒收口(r21 处置面)、阶段超时落 run_failure。
    async fn pump_plan_sc_phase(
        &self,
        ws: &mut LiveWs,
        observation: &mut StageObservation,
        phase: PlanScPumpPhase,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + self.entity_stage_timeout;
        let mut idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
        let phase_started = std::time::Instant::now();
        let mut events_seen = 0u64;
        fp(
            "plan_sc_pump_begin",
            format_args!("phase={phase:?} timeout={:?}", self.entity_stage_timeout),
        );
        loop {
            if tokio::time::Instant::now() >= deadline {
                observation.run_failure = Some(format!(
                    "plan SC 门阶段超时(phase={phase:?},未达预期信号)"
                ));
                fp(
                    "plan_sc_pump_timeout",
                    format_args!("phase={phase:?} elapsed={:?}", phase_started.elapsed()),
                );
                return false;
            }
            let message = tokio::time::timeout_at(idle_deadline, ws.recv_json()).await;
            let message = match message {
                Ok(Ok(Some(value))) => {
                    idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
                    value
                }
                Ok(Ok(None)) => {
                    observation.run_failure =
                        Some("plan SC 门会话 WS 在终态前关闭".to_string());
                    return false;
                }
                Ok(Err(error)) => {
                    observation.run_failure = Some(format!("plan SC 门会话 WS 错误:{error}"));
                    return false;
                }
                Err(_) => {
                    fp(
                        "plan_sc_pump_idle_ping",
                        format_args!("phase={phase:?} elapsed={:?}", phase_started.elapsed()),
                    );
                    if ws.send_json(&json!({"type": "ping"})).await.is_err() {
                        observation.run_failure =
                            Some("plan SC 门会话 WS 在终态前关闭".to_string());
                        return false;
                    }
                    idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
                    continue;
                }
            };
            let kind = message
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            observation.push_event(message.clone());
            events_seen += 1;
            if is_tool_event(&message) {
                observation.tool_events += 1;
            }
            if frame_carries_artifact_markdown(&kind, &message) {
                observation.artifact_markdown_seen = true;
            }
            match kind.as_str() {
                "choice_request" => {
                    if let Some(choice_id) = message.get("id").and_then(Value::as_str) {
                        let (answers, top_selected) = semantic_choice_answers(&message);
                        fp("plan_sc_pump_choice_request", format_args!("id={choice_id}"));
                        let _ = ws
                            .send_json(&json!({
                                "type": "choice_response",
                                "id": choice_id,
                                "selected_option_ids": top_selected,
                                "free_text": null,
                                "answers": answers
                            }))
                            .await;
                    }
                }
                "permission_request" => {
                    observation.permission_events += 1;
                }
                "stage_change" => {
                    let stage = message.get("stage").and_then(Value::as_str).unwrap_or("");
                    fp("plan_sc_pump_stage_change", format_args!("stage={stage}"));
                }
                "session_state" => {
                    if let Some(status) = message.get("status").and_then(Value::as_str) {
                        observation.record_status(status);
                    }
                }
                _ => {}
            }
            // 处置(r26 纯函数决策表):revision 轮回门 / confirm 终态 /
            // 错误帧秒收口。
            match plan_sc_frame_signal(&kind, &message, phase) {
                PlanScFrameSignal::Listen => {}
                PlanScFrameSignal::PhaseDone => {
                    fp(
                        "plan_sc_phase_done",
                        format_args!(
                            "phase={phase:?} elapsed={:?} events={events_seen}",
                            phase_started.elapsed()
                        ),
                    );
                    return true;
                }
                PlanScFrameSignal::ServerError => {
                    fp(
                        "plan_sc_pump_server_error",
                        format_args!("phase={phase:?} frame={message}"),
                    );
                    observation.run_failure =
                        Some(format!("plan SC 门服务端错误帧收口(kind={kind}):{message}"));
                    return false;
                }
            }
        }
    }

    async fn resolve_first_work_item(&mut self) {
        // r31:两个真实来源——①plan prepare 响应携带的 work_item_ids(SC
        // 流 prepare 时已建候选条目);②issue lifecycle 查询(v1.1 E2E
        // §5.1 的 work item 查询形态,plan Confirmed 后条目齐全)。
        // GET /work-items 列表路由不存在(r28/r30 两轮 404 的根因),
        // 不得再用;lifecycle 条目的 id 字段=work_item_id(logical id,
        // coding-attempts 创建路由同口径消费)。
        if let Some(id) = self.plan_work_item_ids.first().cloned() {
            self.work_item_id = Some(id);
            return;
        }
        let (status, body) = request_json(
            &self.app,
            Method::GET,
            &format!("/api/issues/{}/lifecycle?project_id={PROJECT_ID}", self.issue_id),
            json!({}),
        )
        .await;
        if status.is_success() {
            if let Some(id) = body
                .pointer("/work_items/0/work_item_id")
                .and_then(Value::as_str)
            {
                self.work_item_id = Some(id.to_string());
            }
        }
    }

    /// r33:confirmed design 当前版本正文的有效需求编号清单(供 plan SC
    /// 修订反馈纠偏 AI 编造编号;产品 store list_versions 读正文,空/缺
    /// 失时返回空串=反馈退回基础文案)。
    fn design_requirement_ids_digest(&self) -> String {
        let Some(entity_id) = self.design_spec_id.as_deref() else {
            return String::new();
        };
        let Ok(mut versions) =
            self.lifecycle
                .list_versions(PROJECT_ID, &self.issue_id, entity_id)
        else {
            return String::new();
        };
        versions.sort_by_key(|version| version.version);
        let Some(latest) = versions.last() else {
            return String::new();
        };
        extract_requirement_ids(&latest.markdown)
            .into_iter()
            .map(|(id, brief)| {
                if brief.is_empty() {
                    format!("- {id}")
                } else {
                    format!("- {id}:{brief}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        fp_enter_phase(stage, ENTRYPOINT_SPLIT_SYNC, FRESH);
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
                let mut split_resume = StageObservation::new(stage, &self.provider);
                split_resume.entrypoint = entrypoint.to_string();
                split_resume.force_resume = true;
                cells.push(
                    split_resume.denied_cell("split_sync fresh 未启动,resume 无对照".to_string()),
                );
                return cells;
            }
        };
        fp("split_sync_begin_handle", format_args!("session={workspace_session_id}(同步段:持审计互斥锁)"));
        let handle = match self.lifecycle.begin_work_item_split_provider_run(
            PROJECT_ID,
            &self.issue_id,
            &self.provider,
            &workspace_session_id,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                cells.push(fresh.denied_cell(format!("split_sync begin handle 失败:{error}")));
                let mut split_resume = StageObservation::new(stage, &self.provider);
                split_resume.entrypoint = entrypoint.to_string();
                split_resume.force_resume = true;
                cells.push(
                    split_resume.denied_cell("split_sync fresh 未启动,resume 无对照".to_string()),
                );
                return cells;
            }
        };
        fp("split_sync_handle_ok", format_args!("run_ref={}", handle.run_ref));
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
                    let mut split_resume = StageObservation::new(stage, &self.provider);
                    split_resume.entrypoint = entrypoint.to_string();
                    split_resume.force_resume = true;
                    cells.push(
                        split_resume
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
        fp("split_sync_prepare", "prepare_sync_launch(同步段)");
        let launch = gateway
            .prepare_sync_launch(adapter_input, request, context)
            .map_err(|error| error.to_string());
        fp("split_sync_prepare_done", "");
        match launch {
            // B案(r16/r17 冻死根修):run_sync 的同步桥在自有 OS 线程 join 到
            // 终态,直接在测试 runtime 线程上调用会把唯一 runtime(含 in-process
            // server 的全部 timer/WS/HTTP)同步冻死——现场 r16 67min/r17 51min
            // 后 kill(EXIT=137)。join 经 spawn_blocking 移入阻塞线程池 +
            // stage 预算有界观测:超时/失败如实落格,不拖全矩阵;超时后桥线程
            // 与阻塞任务泄漏至进程退出(受 AdapterInput.timeout=stage_timeout
            // 兜底自灭),由外层 kill 回收。
            Ok(launch) => {
                fp(
                    "split_sync_run_sync_begin",
                    format_args!("spawn_blocking 桥启动,stage 预算={}s(桥线程 lc-gateway-sync-bridge)", self.stage_timeout.as_secs()),
                );
                let bounded = tokio::time::timeout(
                    self.stage_timeout,
                    tokio::task::spawn_blocking(move || gateway.run_sync(launch)),
                )
                .await;
                fp("split_sync_run_sync_end", format_args!("bounded={}", bounded.is_ok()));
                match bounded {
                    Ok(Ok(Ok(output))) => {
                        // r26 问题4:F1 事件载荷——sync 栈无 WS 事件流,以
                        // 真实运行观测回填:完成事实(exit/duration/structured)
                        // + CLI stdout 的 stream-json wire 事件行(有界节录,
                        // 磁盘可核对,不伪造)。r25 该格因 events 恒空被
                        // validate 拒("缺事件载荷")。
                        fresh.push_event(json!({
                            "type": "matrix_sync_run_completed",
                            "exit_code": output.exit_code,
                            "duration_ms": output.duration_ms,
                            "timeout_status": format!("{:?}", output.timeout_status),
                            "structured_output_present": output.structured_output.is_some(),
                        }));
                        let mut wire_lines = 0usize;
                        for line in output.stdout.lines() {
                            if wire_lines >= SPLIT_SYNC_WIRE_EVENT_LIMIT {
                                break;
                            }
                            let trimmed = line.trim();
                            if !trimmed.starts_with('{') {
                                continue;
                            }
                            let Ok(mut value) = serde_json::from_str::<Value>(trimmed) else {
                                continue;
                            };
                            truncate_verbose_wire_fields(&mut value);
                            fresh.push_event(json!({
                                "type": "matrix_sync_wire_event",
                                "line": value,
                            }));
                            wire_lines += 1;
                        }
                        if wire_lines == 0 {
                            // stdout 无 JSON 行也是真实观测(如实记录,由
                            // validate 的产物/argv 面兜底判定)。
                            fresh.push_event(json!({
                                "type": "matrix_sync_wire_empty",
                                "stdout_len": output.stdout.len(),
                            }));
                        }
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
                            let reason =
                                format!("split_sync 结构化产物缺失/收口失败:{output:?}");
                            let _ = self
                                .lifecycle
                                .fail_work_item_split_provider_run(&handle, &reason);
                            fresh.run_failure = Some(reason);
                        }
                    }
                    Ok(Ok(Err(error))) => {
                        fp("split_sync_failed", format_args!("{error}"));
                        let _ = self
                            .lifecycle
                            .fail_work_item_split_provider_run(&handle, &error.to_string());
                        fresh.run_failure = Some(format!("split_sync run_sync 失败:{error}"));
                    }
                    Ok(Err(join_error)) => {
                        let reason = format!("split_sync run_sync 阻塞任务异常:{join_error}");
                        let _ = self
                            .lifecycle
                            .fail_work_item_split_provider_run(&handle, &reason);
                        fresh.run_failure = Some(reason);
                    }
                    Err(_elapsed) => {
                        let reason = format!(
                            "split_sync run_sync 超过 stage 预算({}s),降级落格不冻结矩阵",
                            self.stage_timeout.as_secs()
                        );
                        let _ = self
                            .lifecycle
                            .fail_work_item_split_provider_run(&handle, &reason);
                        fresh.run_failure = Some(reason);
                    }
                }
            }
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
        fp_enter_phase(stage, ENTRYPOINT_SPLIT_SYNC, RESUME);
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
            let mut coding_resume = StageObservation::new("coding", &self.provider);
            coding_resume.force_resume = true;
            cells.push(coding_resume.denied_cell("coding fresh 未启动,resume 无会话".to_string()));
            return cells;
        };
        // fresh:组入口建 attempt → coding WS 驱动(StartCoding/阶段门)→
        // FinalConfirm。r37:schema v2 work item 的单件创建入口被产品正确
        // 拒收(schema_v2_group_coding_required,v2 必须经 work item group
        /// lc-root v1.2 §2 coding 腿先例同款组链)——改走
        // POST work-item-plans/{plan_id}/coding-attempts(create_group_
        // coding_attempt:Confirmed plan 校验+组初始化 journal+组锁)。
        let mut observation = StageObservation::new("coding", &self.provider);
        fp_enter_phase("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
        observation.role = "executor".to_string();
        observation.action = "coding_workspace_write".to_string();
        if self.plan_id.is_empty() {
            cells.push(observation.denied_cell(
                "缺 durable plan id(前置 plan prepare 未捕获),coding 组入口无法启动".to_string(),
            ));
            let mut coding_resume = StageObservation::new("coding", &self.provider);
            coding_resume.force_resume = true;
            cells.push(coding_resume.denied_cell("coding fresh 未启动,resume 无会话".to_string()));
            return cells;
        }
        let _ = work_item_id;
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{}/work-item-plans/{}/coding-attempts",
                self.issue_id, self.plan_id
            ),
            json!({}),
        )
        .await;
        if !status.is_success() {
            cells
                .push(observation.denied_cell(format!("coding attempt 创建失败({status}):{body}")));
            let mut coding_resume = StageObservation::new("coding", &self.provider);
            coding_resume.force_resume = true;
            cells.push(coding_resume.denied_cell("coding fresh 未启动,resume 无会话".to_string()));
            return cells;
        }
        let attempt_id = body["attempt_id"].as_str().unwrap_or_default().to_string();
        if attempt_id.is_empty() {
            cells.push(observation.denied_cell(format!("coding attempt 响应缺 id:{body}")));
            let mut coding_resume = StageObservation::new("coding", &self.provider);
            coding_resume.force_resume = true;
            cells.push(coding_resume.denied_cell("coding fresh 未启动,resume 无会话".to_string()));
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
        fp_enter_phase("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
        resume.role = "executor".to_string();
        resume.action = "coding_workspace_write".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = attempt_id.clone();
        resume.requested_resume_id = self.latest_audit_native_id(&attempt_id, &self.provider, None);
        resume.frozen_digest = self.latest_audit_projection_digest(&attempt_id, &self.provider);
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
        let mut idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
        let coding_started = std::time::Instant::now();
        let mut events_seen = 0u64;
        fp("coding_pump_begin", format_args!("attempt={attempt_id} timeout={:?}", self.stage_timeout));
        loop {
            if tokio::time::Instant::now() >= deadline {
                observation.push_event(json!({"type": "matrix_stage_timeout"}));
                return outcome;
            }
            let message = tokio::time::timeout_at(idle_deadline, ws.recv_json()).await;
            let message = match message {
                Ok(Ok(Some(value))) => {
                    idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
                    value
                }
                Ok(Ok(None)) | Ok(Err(_)) => {
                    observation.run_failure = Some("coding 会话 WS 在终态前关闭/错误".to_string());
                    fp("coding_pump_closed", format_args!("elapsed={:?} events={events_seen}", coding_started.elapsed()));
                    return outcome;
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    // 空闲保活:防 coding server idle 断连。同 workspace 泵:
                    // ping 后必须重置 idle_deadline,否则过期 deadline 使
                    // timeout_at 立即 Err 退化成 ping 风暴。
                    fp(
                        "coding_pump_idle_ping",
                        format_args!("elapsed={:?} events={events_seen}(30s 心跳:泵存活/对端静默)", coding_started.elapsed()),
                    );
                    if ws.send_json(&json!({"type": "coding_ping"})).await.is_err() {
                        observation.run_failure =
                            Some("coding 会话 WS 在终态前关闭/错误".to_string());
                        return outcome;
                    }
                    idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
                    continue;
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
            events_seen += 1;
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
                        fp("coding_pump_state", format_args!("status={status} events={events_seen}"));
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
        fp_enter_phase("review", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
        observation.role = "reviewer".to_string();
        observation.action = "review_read_only".to_string();
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{}/design-specs:generate",
                self.issue_id
            ),
            pinned_design_generate_body(
                "矩阵 review:评审会话载体",
                &self.story_spec_ids(),
                &self.member_logical_id,
                &self.provider_wire,
            ),
        )
        .await;
        if !status.is_success() {
            cells
                .push(observation.denied_cell(format!("review 载体会话创建失败({status}):{body}")));
            let mut review_resume = StageObservation::new("review", &self.provider);
            review_resume.force_resume = true;
            cells.push(review_resume.denied_cell("review fresh 未启动,resume 无会话".to_string()));
            return cells;
        }
        let session_id = body["workspace_session"]["workspace_session_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_id.is_empty() {
            cells.push(observation.denied_cell(format!("review 响应缺会话:{body}")));
            let mut review_resume = StageObservation::new("review", &self.provider);
            review_resume.force_resume = true;
            cells.push(review_resume.denied_cell("review fresh 未启动,resume 无会话".to_string()));
            return cells;
        }
        observation.workspace_session_id = session_id.clone();
        self.prior_review_session_id = Some(session_id.clone());
        let (drive, _ws) = self
            .drive_workspace_session_ws(&session_id, &mut observation, 8, self.entity_stage_timeout)
            .await;
        observation.completed_product_artifact_exists = drive.artifact_confirmed;
        cells.push(observation.build_cell(self));

        // resume:评审修订重驱(reviewer 原生恢复)。
        let mut resume = StageObservation::new("review", &self.provider);
        fp_enter_phase("review", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
        resume.role = "reviewer".to_string();
        resume.action = "review_read_only".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = session_id.clone();
        resume.requested_resume_id =
            self.latest_audit_native_id(&session_id, &self.provider, Some("reviewer"));
        resume.frozen_digest = self.latest_audit_projection_digest(&session_id, &self.provider);
        let drive = self
            .drive_revision_resume(
                &session_id,
                &mut resume,
                "矩阵 review resume:显式修订重驱,须原生恢复 reviewer 会话",
                8,
                self.entity_stage_timeout,
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

    /// 会话分区内该 provider 最新 provider_start 的会话全投影摘要。
    fn latest_audit_projection_digest(
        &self,
        workspace_session_id: &str,
        provider: &ProviderName,
    ) -> Option<String> {
        self.scan_session_audits(workspace_session_id)
            .into_iter()
            .find(|(_, record)| provider_matches_record(provider, record))
            .and_then(|(_, record)| {
                record
                    .lc_projection
                    .as_ref()
                    .map(|projection| projection.projection_digest.clone())
            })
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
        fp(
            "write_cell_evidence",
            format_args!("{}/{}/{}(同步段)", cell.stage, cell.entrypoint, cell.fresh_or_resume),
        );
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
                    "audit_projection_digest": cell.audit_projection_digest,
                    "frozen_projection_digest": cell.frozen_projection_digest,
                    "session_projection_digest": cell.session_projection_digest,
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
    /// resume 轮的冻结基准:fresh 轮审计的会话投影摘要(revision 重驱前
    /// 捕获;fresh 轮为 None→以本轮审计值为冻结值)。
    frozen_digest: Option<String>,
    requested_resume_id: Option<String>,
    native_confirmed_id: Option<String>,
    completed_product_artifact_exists: bool,
    /// r27 问题1:产物 markdown 在场(artifact_update/session_state 携带
    /// 非空 markdown)——plan fresh 的产物判定用(停门模型:候选产物已
    /// 生成,confirmed plan 归 resume 格 confirm 后)。
    artifact_markdown_seen: bool,
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
            frozen_digest: None,
            requested_resume_id: None,
            native_confirmed_id: None,
            completed_product_artifact_exists: false,
            artifact_markdown_seen: false,
            observed_spawn_count: 0,
            observed_pid: None,
        }
    }

    fn push_event(&mut self, event: Value) {
        // 事件留证上限:防长流撑爆内存(完整流在服务端 durable 审计)。
        // F3:每事件带 wall-clock ts,落 provider-events.jsonl 供时间线核对。
        // r12 复盘:仅按先到 512 截断会丢掉长阶段尾部的关键事件
        //(choice_request 选项文本/session_state 终态),高频流式块
        //(stream_chunk/pong)满额后让位给低频关键事件。
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let bulky = matches!(kind, "stream_chunk" | "pong");
        if self.events.len() >= 512 && bulky {
            return;
        }
        let wrapped = json!({
            "ts": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "event": event,
        });
        self.events.push(wrapped);
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
            session_projection_digest: String::new(),
            provider_events: self.events.clone(),
        }
    }

    /// 由真实观测(审计 + 事件流 + 产品门)组装证据格。
    fn build_cell(&mut self, env: &MatrixEnvironment) -> EvidenceCell {
        fp(
            "build_cell",
            format_args!("{}/{}(同步段:审计扫描+git 快照)", self.stage, self.force_resume),
        );
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
                cell.gateway_dialect = record.adapter_dialect.clone();
                // fix 轮 10:摘要语义=「audit 记录的会话全投影 == 冻结的
                // 会话全投影」。fresh 轮的冻结值即本轮 prepare→launch 冻结
                // 的投影(audit 即其载体);resume 轮以 fresh 轮审计摘要为
                // 冻结基准,比对 revision 重驱后的实测值——相等=投影无漂移。
                cell.audit_projection_digest = projection.projection_digest.clone();
                cell.frozen_projection_digest = self
                    .frozen_digest
                    .clone()
                    .unwrap_or_else(|| projection.projection_digest.clone());
                cell.session_projection_digest = projection.projection_digest.clone();
            }
            if self.role_run_seq.is_none() {
                self.role_run_seq = Some(*seq);
            }
            // run_ref:workspace streaming 栈以 run-bound 身份回填。
            // r14 复盘:resume 格零 spawn(路由 fail-closed 拒启)时会借
            // fresh 的同一 run_ref,与 fresh 格判重把已 Confirmed 的 fresh
            // 格拖成 denied——resume 格追加相位标记保持唯一(schema 要求
            // 非空,空值也不得参与判重豁免)。
            let phase_suffix = if self.force_resume { "-resume" } else { "" };
            if cell.run_ref.trim().is_empty() {
                cell.run_ref = format!(
                    "ws-{}-{}-run-{}{}",
                    self.workspace_session_id, self.stage, seq, phase_suffix
                );
            }
        }
        // approval/tool 事件:审计 approval_decision + WS 事件流合计;
        // sync 栈(无 WS 事件流)以审计 approval 或真实 argv 携带的权限
        // 投影 wire 段(--allowedTools/--disallowedTools/
        // --permission-prompt-tool 或等价 token)为投放证据。
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
            self.tool_events + self.permission_events + audit_approvals > 0
                || argv_carries_permission_wire(&cell.argv);

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
    /// r21 C1:门到且无确认预算时收口(实体 fresh 停在人工门,
    /// revision 由 resume 格驱动——见 pump_frame_disposition)。
    gate_reached: bool,
}

/// r26 问题4:split_sync fresh 回填 wire 事件行的节录上限(真实 stdout
/// stream-json;过大产物节录防证据膨胀,完整原始输出在 stream-logs/)。
const SPLIT_SYNC_WIRE_EVENT_LIMIT: usize = 24;

/// wire 事件行的大字段截断(逐字段落上限 512 字符;保留 wire 形态与
/// type/id 等身份字段原样)。
fn truncate_verbose_wire_fields(value: &mut Value) {
    const FIELD_TEXT_LIMIT: usize = 512;
    if let Some(object) = value.as_object_mut() {
        for (_key, field) in object.iter_mut() {
            if let Some(text) = field.as_str() {
                if text.len() > FIELD_TEXT_LIMIT {
                    let truncated: String = text.chars().take(FIELD_TEXT_LIMIT).collect();
                    *field = Value::String(format!("{truncated}…[truncated]"));
                }
            }
        }
    }
}

/// r33:从 design 正文提取有效需求编号清单(REQ-xxx/NFR-xxx token + 同行
/// 剩余文本作一句话描述,截 80 字符;去重保序)。纯函数可单测。
fn extract_requirement_ids(markdown: &str) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered: Vec<(String, String)> = Vec::new();
    for line in markdown.lines() {
        let mut rest = line;
        while let Some((id, after)) = find_requirement_token(rest) {
            let brief: String = after
                .trim()
                .trim_start_matches(|c| matches!(c, ':' | '：' | '-' | '—'))
                .trim()
                .chars()
                .take(80)
                .collect();
            if seen.insert(id.clone()) {
                ordered.push((id, brief));
            }
            rest = after;
        }
    }
    ordered
}

/// 在文本中找首个需求编号 token(REQ-/NFR- 前缀 + 字母数字/-/_ 尾部),
/// 返回 (编号, 其后剩余文本);无命中返回 None。
fn find_requirement_token(text: &str) -> Option<(String, &str)> {
    for (offset, _) in text.char_indices() {
        let window = &text[offset..];
        if !(window.starts_with("REQ-") || window.starts_with("NFR-")) {
            continue;
        }
        let id_end = window[4..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .map(|end| 4 + end)
            .unwrap_or(window.len());
        if id_end > 4 {
            return Some((window[..id_end].to_string(), &window[id_end..]));
        }
    }
    None
}

/// r34:confirm 失败是否属于 Final Compile recovery_required 族(v1.1
/// 缺陷 #12 同族:门保持打开,WS work_item_plan_compile_recovery_action:
/// continue 可续跑编译游标)。纯函数可单测。
fn plan_confirm_failure_is_recoverable(failure_text: &str) -> bool {
    failure_text.contains("single-candidate approval compile failed")
}

/// r27 问题1:帧是否携带非空产物 markdown(artifact_update 的 payload,或
/// session_state 全量快照的 artifact 字段)。plan fresh 的产物判定用
/// (停门模型:候选产物已生成即可,confirmed plan 归 resume 格 confirm 后)。
fn frame_carries_artifact_markdown(kind: &str, message: &Value) -> bool {
    let markdown = match kind {
        // r29 问题1:真实帧形态(r28 实测)——artifact_update 的 markdown 在
        // 事件顶层(ArtifactPayload 枚举字段被 serde 展平);保留 payload
        // 嵌套形态兜底。
        "artifact_update" => message
            .get("markdown")
            .and_then(Value::as_str)
            .or_else(|| {
                message
                    .get("payload")
                    .and_then(|payload| payload.get("markdown"))
                    .and_then(Value::as_str)
            }),
        "session_state" => message
            .get("artifact")
            .and_then(|artifact| artifact.get("markdown"))
            .and_then(Value::as_str),
        _ => None,
    };
    markdown.is_some_and(|text| !text.trim().is_empty())
}

/// r26 问题2:SC 门泵单帧信号决策(纯函数,决策表可单测)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanScFrameSignal {
    /// 继续泵送。
    Listen,
    /// 当前相位完成(revision 轮回门 / confirm 定稿终态)。
    PhaseDone,
    /// 服务端拒收/错误帧:立即失败收口。
    ServerError,
}

fn plan_sc_frame_signal(kind: &str, message: &Value, phase: PlanScPumpPhase) -> PlanScFrameSignal {
    match kind {
        "protocol_error" | "error" => PlanScFrameSignal::ServerError,
        // r30 根修(r29 现场):修订 turn 的失败终态帧(validation_reject/
        // provider_err)此前被 Listen——01:52:30 帧已到,泵空转到 03:21
        // 阶段超时。turn 失败=该相位有界收口,帧原文落格。
        "human_gate_turn_failed" => PlanScFrameSignal::ServerError,
        "human_gate_turn_completed" if phase == PlanScPumpPhase::AwaitRevisionComplete => {
            PlanScFrameSignal::PhaseDone
        }
        "stage_change" => {
            let stage = message.get("stage").and_then(Value::as_str).unwrap_or("");
            (phase == PlanScPumpPhase::AwaitConfirmTerminal && stage == "completed")
                .then_some(PlanScFrameSignal::PhaseDone)
                .unwrap_or(PlanScFrameSignal::Listen)
        }
        "session_state" => {
            let status = message.get("status").and_then(Value::as_str).unwrap_or("");
            (phase == PlanScPumpPhase::AwaitConfirmTerminal && status == "confirmed")
                .then_some(PlanScFrameSignal::PhaseDone)
                .unwrap_or(PlanScFrameSignal::Listen)
        }
        _ => PlanScFrameSignal::Listen,
    }
}

/// 泵送单帧处置决策(r21 C1 根修提为纯函数,决策表可单测)。
///
/// r21 现场:resume 格在 stage=Completed 会话上发 request_revision,服务端
/// 按协议矩阵正确拒收(protocol.rs Completed 只放行 SC Advance)并回
/// protocol_error 帧;泵的 `_ => {}` 把它连同 error 帧一起吞掉——无足迹、
/// 无收口,与死锁同形空转到 5400s 阶段超时(story 90min/design 53min 被
/// kill 即此观测盲区)。处置规则:
/// - `protocol_error`/`error`:服务端明确拒收/失败,立即收口落格(带帧
///   原文),不再空等——消灭「拒收与死锁同形」;
/// - 人工门(author_confirm/human_confirm)到且无确认预算:本格驱动完成
///   (实体 fresh 停在门上;Completed 是终态不可修订,门上的反馈修订才是
///   revision/resume 格的产品入口——spec-design-dialog-revision)。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PumpFrameDisposition {
    /// 继续泵送(流片段/choice/审批等常规帧)。
    Listen,
    /// 人工门就绪且有确认预算:走真实 HTTP confirm 门。
    ConfirmGate,
    /// 人工门就绪且无确认预算:驱动在门上完成(不 confirm)。
    GateReached,
    /// confirm 终态:产物确认收口。
    Confirmed,
    /// 失败/阻断终态。
    Terminal(&'static str),
    /// 服务端拒收/错误帧:立即失败收口。
    ServerError,
}

/// r26:SC plan 门 resume 的两阶段泵相位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanScPumpPhase {
    /// 阶段 1:`human_gate_feedback` 修订轮,等 `human_gate_turn_completed`。
    AwaitRevisionComplete,
    /// 阶段 2:typed `confirm` 定稿,等 stage_change completed /
    /// session_state confirmed。
    AwaitConfirmTerminal,
}

fn pump_frame_disposition(
    kind: &str,
    message: &Value,
    confirms_left: usize,
) -> PumpFrameDisposition {
    match kind {
        "session_state" => match message.get("status").and_then(Value::as_str).unwrap_or("") {
            "confirmed" => PumpFrameDisposition::Confirmed,
            "failed" => PumpFrameDisposition::Terminal("failed"),
            "blocked_provider_unavailable" => {
                PumpFrameDisposition::Terminal("blocked_provider_unavailable")
            }
            "terminated" => PumpFrameDisposition::Terminal("terminated"),
            "stopped_needs_human" => PumpFrameDisposition::Terminal("stopped_needs_human"),
            "waiting_for_human" if confirms_left > 0 => PumpFrameDisposition::ConfirmGate,
            "waiting_for_human" => PumpFrameDisposition::GateReached,
            _ => PumpFrameDisposition::Listen,
        },
        "stage_change" => match message.get("stage").and_then(Value::as_str).unwrap_or("") {
            "author_confirm" | "human_confirm" if confirms_left > 0 => {
                PumpFrameDisposition::ConfirmGate
            }
            "author_confirm" | "human_confirm" => PumpFrameDisposition::GateReached,
            _ => PumpFrameDisposition::Listen,
        },
        "protocol_error" | "error" => PumpFrameDisposition::ServerError,
        _ => PumpFrameDisposition::Listen,
    }
}

/// r23 问题2:design 生成请求体——产品端点原生支持调用方钉定聚合视野
/// (`GenerateDesignSpecsRequest.involved_repository_ids`/`change_order`,
/// 方案X 阶段1);不传则由 AI 结构化输出自决回写,r22 实测自决
/// [alpha,beta] 令 design 修订路由 TargetAmbiguous(产品语义:≥2 involved
/// fail-closed,workspace_repository.rs unique_target)。矩阵按 v1.1 E2E
/// 单成员先例钉定 alpha;issue 文案交付域同向收敛 AI 自决回写。
fn pinned_design_generate_body(
    title: &str,
    story_spec_ids: &[String],
    member_logical_id: &str,
    provider_wire: &str,
) -> Value {
    json!({
        "title": title,
        "story_spec_ids": story_spec_ids,
        "involved_repository_ids": [member_logical_id],
        "change_order": [member_logical_id],
        "author_provider": provider_wire,
        "reviewer_provider": provider_wire,
        "review_rounds": 1,
        "superpowers_enabled": false,
        "openspec_enabled": true
    })
}

/// r23:story 生成请求体——钉定单成员 involved。产品面(r23 根修)对钉定
/// involved 派生 focus=首成员,首轮 launch 即锚该成员 checkout;story 的
/// focus 无独立请求面,这是唯一让首轮与修订轮 target 锚一致(指纹可比对、
/// 原生恢复可达)的钉定入口。
fn pinned_story_generate_body(
    title: &str,
    member_logical_id: &str,
    provider_wire: &str,
) -> Value {
    json!({
        "title": title,
        "involved_repository_ids": [member_logical_id],
        "author_provider": provider_wire,
        "reviewer_provider": provider_wire,
        "review_rounds": 1,
        "superpowers_enabled": false,
        "openspec_enabled": true
    })
}

/// r25:所选 provider 的真实探针通道(CLI 程序 + resume 面 native id 提取
/// 通道),同 6c live_probe 冻结映射;四家之外无通道(不冒充)。
fn provider_probe_channel(provider: &ProviderName) -> Option<(&'static str, ResumeChannelKind)> {
    match provider {
        ProviderName::ClaudeCode => Some(("claude", ResumeChannelKind::ClaudePrintJson)),
        ProviderName::Codex => Some(("codex", ResumeChannelKind::CodexExecJson)),
        ProviderName::Pi => Some(("pi", ResumeChannelKind::PiSessionId)),
        ProviderName::KimiCode => Some(("kimi", ResumeChannelKind::KimiStreamJson)),
        ProviderName::Fake => None,
    }
}

/// 探针证据目录 label 用的 action 稳定文本(与产品 action_text 同口径)。
fn matrix_action_text(action: SessionPolicyAction) -> &'static str {
    match action {
        SessionPolicyAction::PlanningReadOnly => "planning_read_only",
        SessionPolicyAction::CodingTargetWrite => "coding_target_write",
        SessionPolicyAction::ReviewReadOnly => "review_read_only",
    }
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

    /// r20 候选③修复:单帧发送原为无超时裸 await(对端停读+缓冲塞满时
    /// 永久挂起,且无 timer 可观测——与 r20 现场「无活跃 timer」吻合)。
    /// 小帧 60s 有界:超时按发送失败落格,不再可能无限等待。
    async fn send_json(&mut self, value: &Value) -> Result<(), String> {
        let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("?");
        let send = self.inner.send(Message::Text(text.into()));
        fp("ws_send_begin", format_args!("type={kind}"));
        let sent = match tokio::time::timeout(WS_SEND_TIMEOUT_SECS, send).await {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err(format!(
                "WS 发送超时({WS_SEND_TIMEOUT_SECS:?};对端不读或链路冻结,type={kind})"
            )),
        };
        match &sent {
            Ok(()) => fp("ws_send_ok", format_args!("type={kind}")),
            Err(error) => fp("ws_send_fail", format_args!("type={kind} error={error}")),
        }
        sent
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
    let http_label = format!("{method} {uri}");
    fp("http_begin", format_args!("{http_label}"));
    // r13 复盘:oneshot 由同 runtime 服务,服务端任务死锁时永不返回
    //(现场 futex 挂死 90min+ 无任何 IO)。有界超时把全矩阵挂死降级为
    // 单格 HTTP 失败,原因可落格。
    let request = async {
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
    };
    fp("http_wait", format_args!("{http_label}(180s 有界)"));
    match tokio::time::timeout(Duration::from_secs(180), request).await {
        Ok(result) => {
            fp("http_end", format_args!("{http_label} -> {}", result.0.as_u16()));
            result
        }
        Err(_) => {
            fp("http_timeout", format_args!("{http_label}(180s 超时:服务端任务疑似挂起)"));
            (
                StatusCode::GATEWAY_TIMEOUT,
                json!({
                    "type": "matrix_request_timeout",
                    "message": "HTTP 请求 180s 超时(服务端任务疑似挂起)"
                }),
            )
        }
    }
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

/// argv 是否携带真实权限投影投放段(sync 栈无 WS 事件流时,权限
/// allowlist/denylist/approval 通道的 argv 痕迹即真实投放证据)。
fn argv_carries_permission_wire(argv: &[String]) -> bool {
    argv.iter().any(|argument| {
        argument.starts_with("--allowedTools")
            || argument.starts_with("--disallowedTools")
            || argument.starts_with("--permission-prompt-tool")
            || argument.starts_with("--allowed-tools")
            || argument.starts_with("--exclude-tools")
            || argument.starts_with("--tools")
    })
}

fn provider_matches_record(provider: &ProviderName, record: &ProviderStartAudit) -> bool {
    let snake = serde_json::to_value(provider)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    // 审计 provider 为 canonical 序列(dash 形态,如 claude-code)。
    record.provider == snake || record.provider == snake.replace('_', "-")
}

/// choice 语义应答:读选项 label(+description)文本逐题选答。
///
/// r12 复盘:恒选首个选项(opt_0)是非语义答案,真实 provider 会把
/// 「聚合视野要求必须确认」类问题反复重问(lc-root 先例 story 13 轮)。
/// 选答口径:
/// 1. 优先聚合/全量口径——「全部/所有/两者/全选」类;
/// 2. 次选确认/正向口径——「确认/继续/是/同意/按建议」类;
/// 3. 规避负向项——「取消/否/不/停止/拒绝/跳过」类(仅当无其他选项才落);
/// 4. 无语义命中时维持首个非负向选项(与旧行为兼容)。
/// 多选题:有全量项只选全量项;否则全选所有非负向项(聚合视野)。
fn semantic_choice_answers(message: &Value) -> (Vec<Value>, Vec<String>) {
    let questions: Vec<&Value> = message
        .get("questions")
        .and_then(Value::as_array)
        .map(|questions| questions.iter().collect())
        .filter(|questions: &Vec<&Value>| !questions.is_empty())
        .unwrap_or_default();
    // 无 questions 数组时按单题顶层形态(options/首题 id)构造伪题,
    // 与 bridge `effective_questions` 的 default 题语义一致。
    let answers = if questions.is_empty() {
        let question_id = message
            .pointer("/questions/0/id")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        let options = message
            .get("options")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let selected = semantic_option_selection(
            message.get("prompt").and_then(Value::as_str).unwrap_or(""),
            options,
            false,
        );
        vec![semantic_answer_entry(&question_id, &selected)]
    } else {
        questions
            .iter()
            .map(|question| {
                let question_id = question
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("default")
                    .to_string();
                let allow_multiple = question
                    .get("allow_multiple")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let options = question
                    .get("options")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let selected = semantic_option_selection(
                    question.get("prompt").and_then(Value::as_str).unwrap_or(""),
                    options,
                    allow_multiple,
                );
                semantic_answer_entry(&question_id, &selected)
            })
            .collect()
    };
    // 顶层 selected_option_ids 以首题答案为准(逐题 answers 才是权威)。
    let top_selected = answers
        .first()
        .and_then(|answer| answer.get("selected_option_ids"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    (answers, top_selected)
}

fn semantic_answer_entry(question_id: &str, selected: &[String]) -> Value {
    json!({
        "question_id": question_id,
        "selected_option_ids": selected,
        "free_text": null
    })
}

/// 单题选答:返回选项 id 列表(语义口径见 `semantic_choice_answers`)。
fn semantic_option_selection(
    question_text: &str,
    options: &[Value],
    allow_multiple: bool,
) -> Vec<String> {
    if options.is_empty() {
        return Vec::new();
    }
    eprintln!(
        "[lcg-choice] options={}",
        options
            .iter()
            .map(|option| {
                format!(
                    "{}|{}|{}",
                    option.get("id").and_then(Value::as_str).unwrap_or("?"),
                    option.get("label").and_then(Value::as_str).unwrap_or(""),
                    option
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join(" ;; ")
    );
    let classified: Vec<(usize, SemanticTier)> = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            // r13 复盘:只按 label 分级——description 常含范围界定语
            //("不含自动续期/不含主动登出清理")会误触负向口径,把
            //「(推荐)」正解挤出选择。
            let label = option.get("label").and_then(Value::as_str).unwrap_or("");
            (index, semantic_tier(label))
        })
        .collect();
    let id_at = |index: usize| -> String {
        options[index]
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("opt_{index}"))
    };
    // r14/v1.1 对照结论:仓库范围题(「涉及哪些逻辑仓库/成员」)优先单成员
    // 口径——plan 阶段 single-candidate 预检按 Design involved 计数(exactly
    // one),story 语义应答选「全部成员」会沿 involved=[alpha,beta] 传导,
    // 把 plan prepare 打死在 preflight(found 2),且 focus 缺席使实体
    // resume 路由 fail-closed。v1.1 E2E 先例(§5.1 plan Confirmed)即单
    // 成员 involved 形态。识别:题面含「仓库/成员/repo」且存在「仅含一个
    // 跨选项枚举词」的选项;该口径优先于全量/推荐。
    let repo_scope_question = ["仓库", "成员", "repo", "repository"]
        .iter()
        .any(|marker| question_text.contains(marker));
    let single_scope = if repo_scope_question {
        let labels: Vec<&str> = options
            .iter()
            .map(|option| option.get("label").and_then(Value::as_str).unwrap_or(""))
            .collect();
        let tokens: Vec<String> = labels
            .iter()
            .flat_map(|label| label.split(|c: char| !c.is_ascii_alphanumeric()))
            .filter(|token| token.len() >= 3)
            .map(str::to_lowercase)
            .collect();
        let enumerated: std::collections::BTreeSet<String> = tokens
            .iter()
            .filter(|token| {
                let needle = token.as_str();
                labels
                    .iter()
                    .filter(|l| l.to_lowercase().contains(needle))
                    .count()
                    >= 2
            })
            .cloned()
            .collect();
        classified
            .iter()
            .filter(|(index, tier)| {
                *tier != SemanticTier::Negative && *tier != SemanticTier::SelectAll
            })
            .find(|(index, _)| {
                let label = labels[*index].to_lowercase();
                enumerated
                    .iter()
                    .filter(|token| label.contains(token.as_str()))
                    .count()
                    == 1
            })
            .map(|(index, _)| *index)
    } else {
        None
    };
    let selected: Vec<usize> = if let Some(index) = single_scope {
        vec![index]
    } else if let Some((index, _)) = classified
        .iter()
        .find(|(_, tier)| *tier == SemanticTier::SelectAll)
    {
        vec![*index]
    } else if let Some((index, _)) = classified
        .iter()
        .find(|(_, tier)| *tier == SemanticTier::Confirm)
    {
        vec![*index]
    } else {
        let non_negative: Vec<usize> = classified
            .iter()
            .filter(|(_, tier)| *tier != SemanticTier::Negative)
            .map(|(index, _)| *index)
            .collect();
        if allow_multiple && non_negative.len() >= 2 {
            non_negative
        } else {
            vec![non_negative.first().copied().unwrap_or(0)]
        }
    };
    eprintln!(
        "[lcg-choice] selected={:?} labels={:?}",
        selected,
        selected
            .iter()
            .map(|&index| options[index]
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("?"))
            .collect::<Vec<_>>()
    );
    selected.into_iter().map(id_at).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticTier {
    Negative,
    Neutral,
    Confirm,
    SelectAll,
}

/// 选项文本分级:负向判定优先(「不确认」不得命中确认口径),
/// 其次全量口径,再次确认口径,其余中性。中文走子串,英文走词边界
///(防 "small" 命中 "all"、"notify" 命中 "no" 一类误配)。
fn semantic_tier(text: &str) -> SemanticTier {
    let normalized = text.trim().to_lowercase();
    let words: Vec<&str> = normalized
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .collect();
    let negative_cjk = [
        "取消", "否", "不", "停止", "终止", "拒绝", "跳过", "暂停", "放弃", "无需",
    ];
    let select_all_cjk = ["全部", "所有", "两者", "全都", "全选", "都包含", "都涉及"];
    let confirm_cjk = [
        "确认",
        "继续",
        "确定",
        "同意",
        "接受",
        "按建议",
        "保持",
        "默认",
        "推荐",
        "执行",
    ];
    let negative_words = [
        "cancel", "stop", "abort", "skip", "discard", "pause", "no", "none", "not",
    ];
    let select_all_words = ["all", "both", "everything"];
    let confirm_words = ["yes", "ok", "confirm", "continue", "approve"];
    if negative_cjk
        .iter()
        .any(|pattern| normalized.contains(pattern))
        || words.iter().any(|word| negative_words.contains(word))
    {
        return SemanticTier::Negative;
    }
    if select_all_cjk
        .iter()
        .any(|pattern| normalized.contains(pattern))
        || words.iter().any(|word| select_all_words.contains(word))
    {
        return SemanticTier::SelectAll;
    }
    if confirm_cjk
        .iter()
        .any(|pattern| normalized.contains(pattern))
        || words.iter().any(|word| confirm_words.contains(word))
        || normalized == "是"
        || normalized == "对"
    {
        return SemanticTier::Confirm;
    }
    SemanticTier::Neutral
}

/// split_sync prompt:harness 携带的真实拆分指令(结构化输出按
/// `WORK_ITEM_SPLIT_OUTPUT_SCHEMA` 契约;产物经 gateway sync bridge 解析)。
fn split_sync_prompt() -> String {
    // fix 轮 8:sync bridge 的 completion parser 只解析
    // `<ARIA_STRUCTURED_OUTPUT nonce=...>...</ARIA_STRUCTURED_OUTPUT>`
    // 包裹的 JSON(生产 split prompt 由 invocation.sentinel_nonce 注入
    // 同款指令);缺失该指令时 provider 输出裸 JSON/文本,必落
    // "missing structured output sentinel"。
    let nonce = "lcg-matrix-split";
    format!(
        "你是跨仓逻辑代码库的 work item 拆分引擎。阅读聚合根下成员仓的公开接口与分层,\n\
         按输出 schema 把本次修复拆为 1-3 个可独立交付的 work item(含 title/kind/\n\
         sequence_hint/depends_on/exclusive_write_scopes)。\n\
         只读规划:不写任何文件。\n\n\
         [output]\n\
         使用 nonce `{nonce}` 包裹唯一 JSON:开始标签 \"<ARIA_STRUCTURED_OUTPUT nonce=\\\"{nonce}\\\">\",\n\
         结束标签 \"</ARIA_STRUCTURED_OUTPUT>\"。JSON 顶层必须先含 \"nonce\":\"{nonce}\",\n\
         再含 repository_profile/plan/work_items(按输出 schema;不用 Markdown code fence)。"
    )
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
    // r16 兜底:服务端冻结时连接/升级可能悬死(现场 67min 无 IO);
    // 有界超时把全矩阵挂死降级为单格 WS 连接失败可落格。
    fp("ws_connect_begin", format_args!("{url}(180s 有界)"));
    let connect = async {
        let (stream, _) = connect_async(url)
            .await
            .map_err(|error| error.to_string())?;
        Ok::<LiveWs, String>(LiveWs::new(stream))
    };
    match tokio::time::timeout(Duration::from_secs(180), connect).await {
        Ok(Ok(ws)) => {
            fp("ws_connect_ok", format_args!("{url}"));
            Ok(ws)
        }
        Ok(Err(error)) => {
            fp("ws_connect_fail", format_args!("{url} error={error}"));
            Err(error)
        }
        Err(_) => {
            fp("ws_connect_timeout", format_args!("{url}(180s 超时:服务端任务疑似冻结)"));
            Err("WS 连接 180s 超时(服务端任务疑似冻结)".to_string())
        }
    }
}

#[cfg(test)]
mod pump_disposition_tests {
    use super::*;
    use serde_json::json;

    fn frame(value: Value) -> Value {
        value
    }

    /// r21 C1 回归锚点:服务端拒收帧(protocol_error)必须秒收口——
    /// 修复前该帧落入 `_ => {}` 无足迹空转,story 90min/design 53min
    /// 与死锁同形;拒收即失败,不得再等阶段超时。
    #[test]
    fn lcg_pump_protocol_error_frame_fails_fast() {
        let message = frame(json!({
            "type": "protocol_error",
            "code": "INVALID_MESSAGE_FOR_STAGE",
            "message": "message request_revision not allowed in stage completed",
            "context": {"stage": "completed", "received": "request_revision"},
        }));
        assert_eq!(
            pump_frame_disposition("protocol_error", &message, 8),
            PumpFrameDisposition::ServerError
        );
    }

    /// r21 C1:error 帧同形秒收口(运行失败类错误帧立即落格)。
    #[test]
    fn lcg_pump_error_frame_fails_fast() {
        let message = frame(json!({"type": "error", "message": "provider failed"}));
        assert_eq!(
            pump_frame_disposition("error", &message, 8),
            PumpFrameDisposition::ServerError
        );
    }

    /// r21 C1:实体 fresh 以零预算停在 author_confirm 门(门上的反馈修订
    /// 才是 resume 格入口;Completed 终态不可修订——spec-design-dialog-
    /// revision),门到且无预算=驱动完成,不得空转到阶段超时。
    #[test]
    fn lcg_pump_author_confirm_gate_without_budget_completes_drive() {
        let message = frame(json!({"type": "stage_change", "stage": "author_confirm"}));
        assert_eq!(
            pump_frame_disposition("stage_change", &message, 0),
            PumpFrameDisposition::GateReached
        );
    }

    /// 对称面:waiting_for_human 状态帧无预算同样按门到收口。
    #[test]
    fn lcg_pump_waiting_for_human_without_budget_completes_drive() {
        let message = frame(json!({"type": "session_state", "status": "waiting_for_human"}));
        assert_eq!(
            pump_frame_disposition("session_state", &message, 0),
            PumpFrameDisposition::GateReached
        );
    }

    /// 回归守卫:有预算的门照常走真实 confirm 门(r13 语义不变)。
    #[test]
    fn lcg_pump_gate_with_budget_still_confirms() {
        let stage_change = frame(json!({"type": "stage_change", "stage": "author_confirm"}));
        assert_eq!(
            pump_frame_disposition("stage_change", &stage_change, 1),
            PumpFrameDisposition::ConfirmGate
        );
        let human_change = frame(json!({"type": "stage_change", "stage": "human_confirm"}));
        assert_eq!(
            pump_frame_disposition("stage_change", &human_change, 8),
            PumpFrameDisposition::ConfirmGate
        );
        let waiting = frame(json!({"type": "session_state", "status": "waiting_for_human"}));
        assert_eq!(
            pump_frame_disposition("session_state", &waiting, 2),
            PumpFrameDisposition::ConfirmGate
        );
    }

    /// 回归守卫:confirmed/失败终态、常规帧处置与 r20 前一致。
    #[test]
    fn lcg_pump_terminal_and_ordinary_frames_unchanged() {
        let confirmed = frame(json!({"type": "session_state", "status": "confirmed"}));
        assert_eq!(
            pump_frame_disposition("session_state", &confirmed, 0),
            PumpFrameDisposition::Confirmed
        );
        let failed = frame(json!({"type": "session_state", "status": "failed"}));
        assert_eq!(
            pump_frame_disposition("session_state", &failed, 3),
            PumpFrameDisposition::Terminal("failed")
        );
        let running = frame(json!({"type": "stage_change", "stage": "running"}));
        assert_eq!(
            pump_frame_disposition("stage_change", &running, 0),
            PumpFrameDisposition::Listen
        );
        let chunk = frame(json!({"type": "stream_chunk", "content": "..."}));
        assert_eq!(
            pump_frame_disposition("stream_chunk", &chunk, 8),
            PumpFrameDisposition::Listen
        );
        // 快照 session_state 无 status 字段:只听不处置(连接初帧不打断泵)。
        let snapshot = frame(json!({"type": "session_state", "stage": "author_confirm"}));
        assert_eq!(
            pump_frame_disposition("session_state", &snapshot, 1),
            PumpFrameDisposition::Listen
        );
    }
    /// r23 问题2 回归锚点:design 生成必须钉定单成员聚合视野——
    /// involved ≥2 时 design 修订路由按产品语义 fail-closed(TargetAmbiguous,
    /// r22 现场 design_spec_0001/0002 双双命中),矩阵 resume 格永不可达。
    #[test]
    fn lcg_design_generate_body_pins_single_member_aggregate_scope() {
        let body = pinned_design_generate_body(
            "矩阵 design:alpha 仓会话过期后端设计",
            &["story_spec_0001".to_string()],
            "logical-uuid-alpha",
            "claude-code",
        );
        let involved = body["involved_repository_ids"]
            .as_array()
            .expect("design 生成请求必须携带 involved_repository_ids 钉定");
        assert_eq!(
            involved,
            &vec![json!("logical-uuid-alpha")],
            "design involved 必须钉定单成员(修订路由 ≥2 fail-closed)"
        );
        let change_order = body["change_order"]
            .as_array()
            .expect("design 生成请求必须携带 change_order");
        assert_eq!(
            change_order,
            &vec![json!("logical-uuid-alpha")],
            "change_order 必须恰好覆盖 involved(端点 validate_requested_aggregate_scope)"
        );
        // story spec 引用透传(design 生成前置)。
        assert_eq!(body["story_spec_ids"], json!(["story_spec_0001"]));
    }

    /// r23 回归锚点:story 生成体必须钉定单成员 involved——不钉则首轮 launch
    /// 锚聚合根视图(记录 involved 空),AI 回写后修订轮锚成员,指纹 target
    /// 三元组+git identity 必漂移(resume_fingerprint_mismatch supersede)。
    #[test]
    fn lcg_story_generate_body_pins_single_member_involved() {
        let body = pinned_story_generate_body(
            "矩阵 story:alpha 仓会话过期提示",
            "logical-uuid-alpha",
            "claude-code",
        );
        let involved = body["involved_repository_ids"]
            .as_array()
            .expect("story 生成请求必须携带 involved_repository_ids 钉定");
        assert_eq!(
            involved,
            &vec![json!("logical-uuid-alpha")],
            "story involved 必须钉定单成员(产品面据此派生 focus,首轮即锚成员)"
        );
    }
    /// r26 问题2 回归锚点:SC 门泵信号决策表——
    /// - SC 门修订轮回门信号=human_gate_turn_completed(仅阶段1);
    /// - confirm 定稿终态=stage_change completed / session_state confirmed
    ///   (仅阶段2);
    /// - 拒收/错误帧恒秒收口(r25 现场 request_revision 被门拒后不得再
    ///   空转)。
    #[test]
    fn lcg_plan_sc_frame_signal_decision_table() {
        let revision = PlanScPumpPhase::AwaitRevisionComplete;
        let confirm = PlanScPumpPhase::AwaitConfirmTerminal;
        let turn_completed = json!({"type": "human_gate_turn_completed", "turn_id": "t1"});
        assert_eq!(
            plan_sc_frame_signal("human_gate_turn_completed", &turn_completed, revision),
            PlanScFrameSignal::PhaseDone
        );
        assert_eq!(
            plan_sc_frame_signal("human_gate_turn_completed", &turn_completed, confirm),
            PlanScFrameSignal::Listen
        );
        let completed = json!({"type": "stage_change", "stage": "completed"});
        assert_eq!(
            plan_sc_frame_signal("stage_change", &completed, confirm),
            PlanScFrameSignal::PhaseDone
        );
        let confirmed = json!({"type": "session_state", "status": "confirmed"});
        assert_eq!(
            plan_sc_frame_signal("session_state", &confirmed, confirm),
            PlanScFrameSignal::PhaseDone
        );
        assert_eq!(
            plan_sc_frame_signal("stage_change", &completed, revision),
            PlanScFrameSignal::Listen
        );
        let rejected = json!({
            "type": "protocol_error",
            "code": "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
        });
        assert_eq!(
            plan_sc_frame_signal("protocol_error", &rejected, revision),
            PlanScFrameSignal::ServerError
        );
        let error = json!({"type": "error", "message": "provider failed"});
        assert_eq!(
            plan_sc_frame_signal("error", &error, confirm),
            PlanScFrameSignal::ServerError
        );
        let chunk = json!({"type": "stream_chunk", "content": "..."});
        assert_eq!(
            plan_sc_frame_signal("stream_chunk", &chunk, confirm),
            PlanScFrameSignal::Listen
        );
        // r30:turn 失败终态帧两相位都秒收口(r29 现场 validation_reject 被
        // Listen 空转 90min)。
        let turn_failed = json!({
            "type": "human_gate_turn_failed",
            "failure_class": "validation_reject",
            "message": "lowering_error:1:session context 缺少 target repository。",
        });
        assert_eq!(
            plan_sc_frame_signal("human_gate_turn_failed", &turn_failed, revision),
            PlanScFrameSignal::ServerError
        );
        assert_eq!(
            plan_sc_frame_signal("human_gate_turn_failed", &turn_failed, confirm),
            PlanScFrameSignal::ServerError
        );
    }

    /// r26 问题4:wire 事件行大字段截断保留形态。
    #[test]
    fn lcg_truncate_verbose_wire_fields_bounds_payload() {
        let long_text = "x".repeat(2048);
        let mut value = json!({
            "type": "system",
            "subtype": "init",
            "session_id": "11111111-2222-3333-4444-555555555555",
            "long_field": long_text,
        });
        truncate_verbose_wire_fields(&mut value);
        assert_eq!(value["type"], json!("system"));
        assert_eq!(
            value["session_id"],
            json!("11111111-2222-3333-4444-555555555555")
        );
        let truncated = value["long_field"].as_str().expect("truncated text");
        assert!(truncated.len() < 600, "长字段必须截断:{}", truncated.len());
        assert!(truncated.ends_with("…[truncated]"));
    }

    /// r27 问题1:产物 markdown 在场判定(artifact_update payload / session_state
    /// artifact 字段;空 markdown 不算)。
    #[test]
    fn lcg_frame_carries_artifact_markdown_detects_non_empty_payload() {
        // r28 真实形态:markdown 在事件顶层(载荷枚举字段展平)。
        let artifact_update = json!({
            "type": "artifact_update",
            "event_seq": 7334,
            "markdown": "# Work Item Plan\n## Work Item WI-001 …",
        });
        assert!(frame_carries_artifact_markdown("artifact_update", &artifact_update));
        // 嵌套 payload 形态兜底同样命中。
        let nested_update = json!({
            "type": "artifact_update",
            "payload": {"markdown": "# 会话过期提示 Story Spec\n内容…", "version": 2},
        });
        assert!(frame_carries_artifact_markdown("artifact_update", &nested_update));
        let snapshot = json!({
            "type": "session_state",
            "artifact": {"markdown": "# plan 候选"},
        });
        assert!(frame_carries_artifact_markdown("session_state", &snapshot));
        // 空 markdown/缺失/其他帧不算(plan fresh 产物判定不得被空帧误置)。
        let empty_update = json!({"type": "artifact_update", "payload": {"markdown": "   "}});
        assert!(!frame_carries_artifact_markdown("artifact_update", &empty_update));
        let no_artifact = json!({"type": "session_state", "artifact": null});
        assert!(!frame_carries_artifact_markdown("session_state", &no_artifact));
        let chunk = json!({"type": "stream_chunk", "content": "# 不是产物"});
        assert!(!frame_carries_artifact_markdown("stream_chunk", &chunk));
    }

    /// r33:需求编号提取器——design 正文中的 REQ-/NFR- 编号(含同行简述,
    /// 截 80 字符)去重保序列出;一行多编号逐个提取;非编号文本忽略。
    #[test]
    fn lcg_extract_requirement_ids_lists_design_ids_with_brief() {
        let markdown = "# Design Spec\n\n## REQ-ENV-01: 会话签发接口\n签发与会话查询。\n\n## NFR-PERF-2 延迟上限\nP99 < 50ms(REQ-ENV-01 同样适用)。\n\n无关行不含编号。";
        let ids = extract_requirement_ids(markdown);
        let plain: Vec<&str> = ids.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(plain, vec!["REQ-ENV-01", "NFR-PERF-2"], "编号保序去重: {ids:?}");
        assert!(
            ids[0].1.contains("会话签发接口"),
            "同行剩余文本作一句话描述: {ids:?}"
        );
        // 同编号第二次出现不重复;编号后无尾字符(如孤立 REQ-)不提取。
        let dup = extract_requirement_ids("REQ-A-1: x\nREQ-A-1: y\nREQ-");
        assert_eq!(dup.len(), 1, "重复编号去重、裸前缀不提取: {dup:?}");
        assert!(dup[0].1.contains('x'), "保留首次出现的简述: {dup:?}");
    }

    /// r34:confirm 失败的可恢复族判定——v1.1 缺陷 #12 同族(门保持打开,
    /// recovery continue 可续跑);非该族(真失败)不重试。
    #[test]
    fn lcg_plan_confirm_failure_recoverable_family() {
        let recoverable = "plan SC 门服务端错误帧收口(kind=error):{\"message\": \"single-candidate approval compile failed; human gate remains open\\nfailure_reason: compile transaction recovery_required\"}";
        assert!(plan_confirm_failure_is_recoverable(recoverable));
        let genuine = "plan SC 门服务端错误帧收口(kind=error):{\"message\": \"validator findings: [error] requirement_not_found\"}";
        assert!(!plan_confirm_failure_is_recoverable(genuine));
    }

}
