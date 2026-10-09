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
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use cadence_aria::cross_cutting::provider_boundary::{
    BoundaryWriteChannel, PlannedBoundaryWrite, ProviderBoundaryLauncher, ProviderBoundaryMode,
    ProviderBoundaryPlan, run_write_surface_probe,
};
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::cross_cutting::provider_health::ProviderHealthService;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, StreamingProviderInput,
};
use cadence_aria::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ProviderStartAudit};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::logical_codebase::RootRecipeReceiptStore;
use cadence_aria::product::logical_codebase::policy::{
    AggregatePolicyArtifactStore, PolicyTarget, SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::policy::{ProviderDialect, ProviderWireDialect};
use cadence_aria::product::logical_codebase::production_policy_resolvers::{
    ProductionPolicyTargetResolver, StoreBackedProviderCapabilitySource,
};
use cadence_aria::product::logical_codebase::provider_boundary_probe::{
    BOUNDARY_PROBE_EVIDENCE_SCHEMA, BoundaryFixture, ProviderBoundaryProbe, ResumeChannelKind,
    ResumeProbeSpec, provider_family_text, run_cli_boundary_probe,
};
use cadence_aria::product::logical_codebase::provider_capability_store::{
    CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
    ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore, RootRecipeEvidence,
};
use cadence_aria::product::logical_codebase::provider_gateway::{
    LogicalCodebaseProviderGateway, ProviderLaunchAuditContext, ProviderRef, ProviderRefType,
    SessionLaunchRequest,
};
use cadence_aria::product::logical_codebase::provider_trust::{
    HomeBackedProviderTrustRegistry, ProviderTrustPrecondition, ProviderTrustSource,
    ReadonlyProviderTrustSource, requires_workspace_trust,
};
use cadence_aria::product::logical_codebase::provider_trust_adapters::{
    CodexTrustAdapter, KimiTrustAdapter,
};
use cadence_aria::product::logical_codebase::store::LogicalCodebaseStore;
use cadence_aria::product::logical_codebase::types::{
    CodebaseMemberRecord, RepositoryCheckoutRecord,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::AdapterOutput;
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
use tokio_util::sync::CancellationToken;
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

/// F5(r58 深掏审计):coding 观测格 role 与 native 断言扫描的角色过滤
/// 共用常量(AdapterRole::Executor 的序列值;attempt 分区内 reviewer 的
/// ReviewReadOnly 启动同计入,扫描必须按本角色过滤)。
const CODING_EXECUTOR_ROLE: &str = "executor";

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
    /// kimi-9:resume 格「在途 coder run 重挂」证据(native id 不等时的
    /// 第二合法确认形态;fresh 格恒 false)。
    pub native_resume_reattached: bool,
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
    /// r52 快照:格证据的执行来源——full_chain(本轮真实执行)|
    /// snapshot_boot(快照续跑轮真实执行)|carried:<snapshot_id>
    ///(承继格:引用来源轮证据链,不重新计票)。
    pub execution_origin: String,
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
        // r52 快照:承继格=引用格(capability_state=carried),结构断言
        // 属于来源轮;本轮只校验其引用形态(origin/run_ref)。
        if self.capability_state == "carried" {
            if !self.execution_origin.starts_with("carried:") || self.run_ref.trim().is_empty() {
                return Err(format!(
                    "承继格引用形态不完整:origin={:?} run_ref={:?}",
                    self.execution_origin, self.run_ref
                ));
            }
            return Ok(());
        }
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
        // kimi-9:resume 格第二合法形态——native id 不等但在途 coder run
        // 重挂(reattached)时放行;fresh 格 reattached 恒 false。
        let native_pair = (
            self.fresh_or_resume.as_str(),
            self.requested_resume_id.as_deref(),
            self.native_resume_confirmed_id.as_deref(),
            self.native_resume_reattached,
        );
        match native_pair {
            (fresh, None, None, false) if fresh == FRESH => {}
            (resume, Some(requested), Some(confirmed), _)
                if resume == RESUME && requested == confirmed => {}
            (resume, Some(_requested), Some(_confirmed), true) if resume == RESUME => {}
            _ => {
                return Err(format!(
                    "原生恢复确认 != 请求:{} 格请求 {:?} 确认 {:?}(在途重挂={})",
                    self.fresh_or_resume,
                    self.requested_resume_id,
                    self.native_resume_confirmed_id,
                    self.native_resume_reattached
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
        // Task 11 零 spawn 结构面:被拒格(capability∈{denied,unknown})与
        // 完整 provider-start 成功证据互斥——argv/wire 捕获 + 真实 argv +
        // exact version + 完成产物齐备而状态非 Confirmed = 矛盾记录(伪装
        // 格),拒绝。握手失败已创建 child 的 kill/reap 格不携带完成产物,
        // 不属零 spawn 被拒格,不受本条影响(spawn≥1 的观测由会计五维
        // 分列,见 to_cell_json)。
        if matches!(self.capability_state.as_str(), "denied" | "unknown")
            && self.argv_or_wire_capture_exists
            && !self.argv.is_empty()
            && !self.exact_version.trim().is_empty()
            && self.completed_product_artifact_exists
        {
            return Err(
                "零 spawn 被拒格携带 provider-start 成功证据(被拒状态与完整 \
                 正向证据矛盾)"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// `cell.json` 形态(键 + 断言组字段 + 全要素;敏感项不落盘)。
    pub(crate) fn to_cell_json(&self) -> Value {
        let mut cell = json!({
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
            "native_resume_reattached": self.native_resume_reattached,
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
        });
        // Task 11:零 spawn 会计五维(计划 Interfaces 冻结,分列可审计)+
        // 组装层经 provider_events 携带的边界矩阵证据 flag(首见为准)。
        if let Some(flags) = self.boundary_matrix_evidence_flags() {
            let object = cell.as_object_mut().expect("cell json object");
            for (key, value) in flags {
                object.insert(key, value);
            }
        }
        if let Some(accounting) = self.zero_spawn_accounting() {
            let object = cell.as_object_mut().expect("cell json object");
            for (key, value) in accounting {
                object.insert(key, value);
            }
        }
        cell
    }

    /// Task 11 零 spawn 会计五维(计划 Interfaces 冻结;各维度只由本维证据
    /// 源推导,分列可审计):
    /// - session_child_count:spawn 审计计数与 stream-log PID 任一在场即取
    ///   其大者,加上事件流里的显式 child 观测;两者皆缺=显式 0(被拒格
    ///   零 child 的会计口径,不以「无记录」冒充正值);
    /// - native_handshake_count:native session id 在场=握手完成(≥1);
    ///   kill/reap 格(握手未完成、id 空)为 0,不冒充;
    /// - extension_mcp_descendant_count/native_method_count:事件流里的
    ///   extension/MCP 后代与 native 方法/RPC 观测计数;
    /// - availability_version_count:`--version` availability 探测另记
    ///   (探测不是 session child,不并入 session 维)。
    fn zero_spawn_accounting(&self) -> Option<serde_json::Map<String, Value>> {
        let observed = |marker: &str| {
            self.provider_events
                .iter()
                .filter(|event| event["event"]["type"].as_str() == Some(marker))
                .count() as u64
        };
        let session_child_count = self
            .provider_spawn_count
            .max(u64::from(self.provider_pid.is_some()))
            .saturating_add(observed("session_child_observed"));
        let native_handshake_count = if self.native_session_id.trim().is_empty() {
            0
        } else {
            session_child_count.max(1)
        };
        Some(
            json!({
                "session_child_count": session_child_count,
                "native_handshake_count": native_handshake_count,
                "extension_mcp_descendant_count": observed("extension_mcp_descendant_observed"),
                "native_method_count": observed("native_method_observed"),
                "availability_version_count": observed("availability_version_probe"),
            })
            .as_object()
            .cloned()
            .expect("accounting object"),
        )
    }

    /// Task 11 边界矩阵证据 flag:组装层经 `provider_events` 携带的
    /// `boundary_matrix_evidence` 事件(failure_scenario/受控写/保护面拒绝/
    /// D4 无漂移等逐格 flag)合并进 cell.json 顶层,首见为准。
    fn boundary_matrix_evidence_flags(&self) -> Option<serde_json::Map<String, Value>> {
        let mut merged = serde_json::Map::new();
        for event in &self.provider_events {
            let Some(payload) = event.get("event").and_then(Value::as_object) else {
                continue;
            };
            if payload.get("type").and_then(Value::as_str) != Some("boundary_matrix_evidence") {
                continue;
            }
            for (key, value) in payload {
                if key != "type" && !merged.contains_key(key) {
                    merged.insert(key.clone(), value.clone());
                }
            }
        }
        Some(merged)
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
        let run_mode = super::snapshot::RunMode::from_env()
            .map_err(|message| matrix_failure("run_mode_invalid", message, None))?;
        fp("matrix_run_mode", format_args!("{run_mode:?}"));
        fp_enter_phase("matrix", "env", "build");
        let mut env = match run_mode {
            super::snapshot::RunMode::ResumeFromPlanSnapshot => {
                MatrixEnvironment::build_from_snapshot(provider.clone(), evidence_root).await?
            }
            super::snapshot::RunMode::FullChain | super::snapshot::RunMode::CapturePlanSnapshot => {
                MatrixEnvironment::build(provider.clone(), evidence_root).await?
            }
        };
        fp(
            "env_built",
            format_args!("lc={} issue={}", env.lc_id, env.issue_id),
        );

        let mut cells = Vec::new();
        if run_mode == super::snapshot::RunMode::ResumeFromPlanSnapshot {
            // 快照续跑轮:story/design/plan/split 为承继格(引用来源轮证据
            // 链,不重新计票);coding/review 真实执行(snapshot_fresh)。
            let snapshot_id = env
                .snapshot_id
                .clone()
                .expect("resume env carries snapshot id");
            let manifest = env
                .snapshot_manifest
                .as_ref()
                .expect("resume env carries snapshot manifest");
            for stage in ["story", "design", "plan", "split"] {
                cells.push(carried_cell(stage, &provider, manifest, &snapshot_id));
                fp(
                    "stage_carried",
                    format_args!("{stage} snapshot={snapshot_id}"),
                );
            }
        }
        // 五阶段顺序固定:Story、Design、Plan、Coding、Review。
        if run_mode != super::snapshot::RunMode::ResumeFromPlanSnapshot {
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
            fp(
                "stage_end",
                format_args!("plan streaming cells={}", cells.len()),
            );
            // Plan:split_sync 对照(gateway sync bridge)。
            fp("stage_begin", "plan split_sync");
            cells.extend(env.run_split_sync_stage().await);
            fp(
                "stage_end",
                format_args!("plan split_sync cells={}", cells.len()),
            );
        }
        // Coding(依赖 Plan 确认后的 work item;失败落格)。全链与续跑轮都
        // 真实执行;续跑轮 observation.execution_origin 在阶段内部标记
        // snapshot_boot(见 run_coding_stage/run_review_stage 的 env 传导)。
        fp("stage_begin", "coding");
        cells.extend(env.run_coding_stage().await);
        fp("stage_end", format_args!("coding cells={}", cells.len()));
        // Review:reviewer 角色经 streaming 栈的真实评审会话。
        fp("stage_begin", "review");
        cells.extend(env.run_review_stage().await);
        fp("stage_end", format_args!("review cells={}", cells.len()));
        if run_mode == super::snapshot::RunMode::ResumeFromPlanSnapshot {
            // 续跑轮收口:递增再基线计数(每 N 轮强制全链)。
            let rounds = super::snapshot::record_resume_round(evidence_root)
                .map_err(|message| matrix_failure("snapshot_state_write_failed", message, None))?;
            fp(
                "snapshot_resume_round_recorded",
                format_args!("rounds_since_baseline={rounds}"),
            );
        }

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
// Task 11:越界写、D4 与失败零 spawn 真实矩阵(boundary_matrix.rs 现场测试
// 消费;计划 Task 11 段 488-511 行)。
//
// 结构(全程沿用 fp()/fp_enter_phase 足迹约定——看门狗 v3 以 [fp] 前缀判
// 实质进展,每个探针/场景/会计步骤一行足迹,纯本地 git 转换点显式留痕):
// 1. `run_boundary_and_failure_matrix`:单 provider 现场入口(计划
//    Interfaces 冻结签名),复用 `MatrixEnvironment::build`(真实 git
//    fixture/聚合初始化/trust/6c capability 探针,探针证据已落
//    `boundary/boundary/<label>/`);
// 2. 边界段:6c Coding 工件(受控 commit + 四通道负向)核对 + 组装层
//    五通道×保护面追加探针(child 通道/越界 symlink/非 target worktree,
//    经 `run_write_surface_probe` 在产品写边界沙箱内真实执行,每条留拒绝
//    原文与文件 digest)+ D4(active main HEAD+porcelain 与 root/metadata
//    前后快照,沿 HEAD 既有预算 fail-closed 不截断);
// 3. 失败段:17 固定负向场景逐格真实拒绝(真实 gateway 的 validate/
//    prepare/spawn 前复验对受控突变状态的真实拒绝),零 spawn 由
//    「审计扫描=0 + 无 PID + 无 argv」三面同时证明(会计五维分列);
//    child spawn 失败分清「创建失败=0」与「握手失败已创建=kill/reap」
//    (后者 spawn≥1,不属零 spawn 格);
// 4. 证据落计划 Files 冻结目录 `cadence/reports/lc-gateway-multi-provider/
//    <provider>/boundary/` 与 `failures/`。
// ---------------------------------------------------------------------------

/// 失败段场景清单:直接消费冻结测试面 `failure_matrix::FIXED_FAILURE_
/// SCENARIOS`(boundary_matrix 现场覆盖断言同源,harness 不另立第二清单)。
use super::failure_matrix::FIXED_FAILURE_SCENARIOS as TASK11_FAILURE_SCENARIOS;

/// Task 11 现场证据根(计划 Files 冻结):`<provider>/{boundary,failures}/`。
fn task11_evidence_roots(provider: &ProviderName) -> (PathBuf, PathBuf) {
    let base = PathBuf::from("cadence/reports/lc-gateway-multi-provider")
        .join(provider_family_text(provider));
    (base.join("boundary"), base.join("failures"))
}

/// 事件封包 ts(与 StageObservation::push_event 同口径)。
fn task11_now_ts() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 逐字节 sha256(hex)。
fn task11_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl LiveLcGatewayHarness {
    /// Task 11 现场入口(计划 Interfaces 冻结签名):单 provider 的越界写
    /// (五通道×保护面)、D4(active main HEAD+porcelain + root/metadata)与
    /// 失败零 spawn(17 固定负向场景)真实矩阵。环境不可运行 → `Err`
    /// (BLOCKED 真实报告);阶段内失败一律落格(denied/unknown + reason),
    /// 不删格、不以删格缩小验收。
    pub(crate) async fn run_boundary_and_failure_matrix(
        &self,
        provider: ProviderName,
    ) -> Result<LiveMatrixEvidence, LiveMatrixFailure> {
        let (boundary_root, failures_root) = task11_evidence_roots(&provider);
        fp_enter_phase("t11", "env", "build");
        let mut env = MatrixEnvironment::build(provider.clone(), &boundary_root).await?;
        fp(
            "t11_env_built",
            format_args!(
                "lc={} canonical_root={} target={}",
                env.lc_id,
                env.canonical_root.display(),
                env.member_worktree.display()
            ),
        );

        // D4 pre 快照:active main(HEAD+porcelain)+ root/metadata(先于一切探针)。
        fp_enter_phase("t11", "d4", "pre");
        let d4_before = task11_d4_snapshot(&env)?;
        fp(
            "t11_d4_pre",
            format_args!("combined={}", d4_before.combined_digest),
        );

        // 边界段:组装层五通道×保护面探针 + 6c Coding 工件核对。
        let write_matrix = run_task11_write_matrix(&env).await?;
        let mut cells = Vec::new();
        cells.push(task11_coding_positive_cell(&env)?);
        cells.push(task11_protected_writes_cell(&env, &write_matrix)?);

        // 失败段:17 固定负向场景逐格真实拒绝(逐格失败不中止矩阵)。
        cells.extend(run_task11_failure_scenarios(&mut env).await);

        // D4 post 快照 + 无漂移格(所有场景恢复之后收口)。
        fp_enter_phase("t11", "d4", "post");
        let d4_after = task11_d4_snapshot(&env)?;
        fp(
            "t11_d4_post",
            format_args!(
                "combined={} drift={}",
                d4_after.combined_digest,
                d4_after.combined_digest != d4_before.combined_digest
            ),
        );
        cells.push(task11_d4_cell(&env, &d4_before, &d4_after));

        // run_ref 在 entrypoint 内唯一(与 run_provider_matrix 同构)。
        let mut seen: BTreeMap<(String, String), usize> = BTreeMap::new();
        for cell in &cells {
            *seen
                .entry((cell.entrypoint.clone(), cell.run_ref.clone()))
                .or_default() += 1;
        }
        for cell in &mut cells {
            cell.run_ref_is_unique_within_entrypoint = seen
                .get(&(cell.entrypoint.clone(), cell.run_ref.clone()))
                .is_none_or(|count| *count == 1);
        }

        // 逐格落盘。Task 11 格不走 validate_against 降级:失败格的「缺
        // argv/产物」是场景本体而非矛盾记录;边界格的证据面是探针工件,
        // 完备性由逐格 flag 与现场断言承担。
        for root in [&boundary_root, &failures_root] {
            std::fs::create_dir_all(root).map_err(|error| {
                matrix_failure(
                    "evidence_root_unwritable",
                    format!("创建证据根目录失败 {}: {error}", root.display()),
                    None,
                )
            })?;
        }
        let mut pending_writes: Vec<(PathBuf, EvidenceCell)> = Vec::new();
        for cell in &cells {
            let failure_scenario = cell.boundary_matrix_evidence_flags().and_then(|flags| {
                flags
                    .get("failure_scenario")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            let directory = match failure_scenario {
                Some(scenario) => failures_root.join(scenario),
                None => boundary_root.join(&cell.run_ref),
            };
            pending_writes.push((directory, cell.clone()));
        }
        for (directory, cell) in &pending_writes {
            write_task11_cell(directory, cell).map_err(|reason| {
                matrix_failure(
                    "evidence_write_failed",
                    format!("格 {} 落盘失败:{reason}", cell.run_ref),
                    None,
                )
            })?;
        }

        Ok(LiveMatrixEvidence {
            provider,
            canonical_root: env.canonical_root.clone(),
            member_worktree: env.member_worktree.clone(),
            cells,
            rejections: Vec::new(),
        })
    }
}

/// Task 11 格落盘(cell.json + provider-events.jsonl + sha256 清单;与
/// write_cell_evidence 同形态但按 Task 11 目录布局)。
fn write_task11_cell(directory: &Path, cell: &EvidenceCell) -> Result<(), String> {
    fp(
        "t11_write_cell",
        format_args!("{} → {}", cell.run_ref, directory.display()),
    );
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("证据目录创建失败 {}: {error}", directory.display()))?;
    std::fs::write(
        directory.join("cell.json"),
        serde_json::to_vec_pretty(&cell.to_cell_json()).unwrap_or_default(),
    )
    .map_err(|error| format!("cell.json 写入失败: {error}"))?;
    let mut events = String::new();
    for event in &cell.provider_events {
        events.push_str(&serde_json::to_string(event).unwrap_or_default());
        events.push('\n');
    }
    std::fs::write(directory.join("provider-events.jsonl"), events.into_bytes())
        .map_err(|error| format!("provider-events.jsonl 写入失败: {error}"))?;
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .map_err(|error| format!("证据目录读取失败 {}: {error}", directory.display()))?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut manifest = String::new();
    for name in names {
        if name == "manifest.sha256" {
            continue;
        }
        let bytes = std::fs::read(directory.join(&name))
            .map_err(|error| format!("清单读取失败 {directory:?}/{name}: {error}"))?;
        manifest.push_str(&format!("{}  {name}\n", task11_sha256_hex(&bytes)));
    }
    std::fs::write(directory.join("manifest.sha256"), manifest.into_bytes())
        .map_err(|error| format!("manifest.sha256 写入失败: {error}"))?;
    Ok(())
}

/// Task 11 通用格基座(字段语义见各段落;capability_state 由场景覆写)。
fn task11_cell_base(env: &MatrixEnvironment, run_ref: String) -> EvidenceCell {
    EvidenceCell {
        provider: env.provider.clone(),
        exact_version: String::new(),
        stage: "coding".to_string(),
        entrypoint: ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string(),
        fresh_or_resume: FRESH.to_string(),
        process_cwd: env.canonical_root.clone(),
        target: env.member_worktree.clone(),
        audit_projection_digest: String::new(),
        frozen_projection_digest: String::new(),
        native_resume_confirmed_id: None,
        native_resume_reattached: false,
        requested_resume_id: None,
        argv_or_wire_capture_exists: false,
        approval_and_tool_events_exist: false,
        completed_product_artifact_exists: false,
        run_ref,
        run_ref_is_unique_within_entrypoint: true,
        action: "coding_target_write".to_string(),
        role: "executor".to_string(),
        gateway_dialect: String::new(),
        wire_dialect: String::new(),
        native_session_id: String::new(),
        workspace_session_id: String::new(),
        argv: Vec::new(),
        capability_state: "unknown".to_string(),
        denied_reason: None,
        provider_pid: None,
        pid_unavailable_reason: Some(
            "Task 11 探针格:本格不主张会话 PID(零 spawn 会计由事件流分列)".to_string(),
        ),
        provider_spawn_count: 0,
        session_projection_digest: String::new(),
        provider_events: Vec::new(),
        execution_origin: "full_chain".to_string(),
    }
}

// ---------------------------------------------------------------------------
// 边界段:6c Coding 工件核对 + 组装层五通道×保护面追加探针 + D4。
// ---------------------------------------------------------------------------

/// build 内 6c 探针的 Coding 工件(最新 matrix-coding_target_write-*)。
fn latest_task11_coding_probe_artifact(
    env: &MatrixEnvironment,
) -> Result<(PathBuf, Value), LiveMatrixFailure> {
    let root = env.evidence_root.join("boundary");
    let mut latest: Option<(String, PathBuf)> = None;
    let entries = std::fs::read_dir(&root).map_err(|error| {
        matrix_failure(
            "coding_probe_artifact_missing",
            format!("读取探针目录失败 {}: {error}", root.display()),
            None,
        )
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // 探针工件形态:<label 目录>/evidence.json;非目录条目(残留
        // 文件)不参与。
        if !name.starts_with("matrix-coding_target_write-") || !entry.path().is_dir() {
            continue;
        }
        if latest.as_ref().is_none_or(|(seen, _)| *seen < name) {
            latest = Some((name, entry.path().join("evidence.json")));
        }
    }
    let Some((_, path)) = latest else {
        return Err(matrix_failure(
            "coding_probe_artifact_missing",
            format!("6c Coding 探针工件未找到于 {}", root.display()),
            None,
        ));
    };
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).map_err(|error| {
        matrix_failure(
            "coding_probe_artifact_unreadable",
            format!("{}: {error}", path.display()),
            None,
        )
    })?)
    .map_err(|error| {
        matrix_failure(
            "coding_probe_artifact_unreadable",
            format!("{} 解析失败: {error}", path.display()),
            None,
        )
    })?;
    if value.get("schema").and_then(Value::as_str) != Some(BOUNDARY_PROBE_EVIDENCE_SCHEMA) {
        return Err(matrix_failure(
            "coding_probe_artifact_schema_mismatch",
            format!(
                "{} schema 非 {BOUNDARY_PROBE_EVIDENCE_SCHEMA}",
                path.display()
            ),
            None,
        ));
    }
    Ok((path, value))
}

/// 组装层追加探针结果(逐条 {channel,path,outcome,refusal_text,digest 对})。
struct Task11WriteMatrix {
    attempts: Vec<Value>,
    fixture_base: PathBuf,
}

/// 计划:五通道 × 保护面(root/非 target main/非 target worktree/.git/
/// .aria/target .git 指针/越界 symlink)。与产品 6c 负向矩阵互补:补齐
/// child 通道、越界 symlink 与非 target worktree 面。
fn planned_task11_writes(
    fixture: &BoundaryFixture,
    non_target_wt: &Path,
    escape_link: &Path,
) -> Vec<PlannedBoundaryWrite> {
    let root = fixture.root();
    let member = fixture.member();
    let member_b = fixture.member_b();
    let target = fixture.target().expect("coding fixture target");
    let planned =
        |channel: BoundaryWriteChannel, path: PathBuf| PlannedBoundaryWrite::new(channel, path);
    vec![
        // builtin:root 面、root .git、非 target main、target .git 指针。
        planned(
            BoundaryWriteChannel::Builtin,
            root.join("t11-rogue-builtin"),
        ),
        planned(
            BoundaryWriteChannel::Builtin,
            root.join(".git").join("t11-rogue"),
        ),
        planned(
            BoundaryWriteChannel::Builtin,
            member.join("t11-rogue-builtin"),
        ),
        planned(BoundaryWriteChannel::Builtin, target.join(".git")),
        // terminal:root .aria、root AGENTS.md、非 target worktree。
        planned(
            BoundaryWriteChannel::Terminal,
            root.join(".aria").join("t11-rogue"),
        ),
        planned(BoundaryWriteChannel::Terminal, root.join("AGENTS.md")),
        planned(
            BoundaryWriteChannel::Terminal,
            non_target_wt.join("t11-rogue-terminal"),
        ),
        // extension:非 target main .git、root mcp 副本、target .aria。
        planned(
            BoundaryWriteChannel::Extension,
            member_b.join(".git").join("t11-rogue"),
        ),
        planned(
            BoundaryWriteChannel::Extension,
            root.join(".mcp.json.t11-rogue"),
        ),
        planned(
            BoundaryWriteChannel::Extension,
            target.join(".aria").join("t11-rogue"),
        ),
        // mcp:非 target main .git、root .mcp.json。
        planned(
            BoundaryWriteChannel::Mcp,
            member.join(".git").join("t11-rogue"),
        ),
        planned(BoundaryWriteChannel::Mcp, root.join(".mcp.json")),
        // child(setsid 脱组再派生子进程):root、root .git、非 target main、
        // 非 target worktree、target .git 指针。
        planned(BoundaryWriteChannel::Child, root.join("t11-rogue-child")),
        planned(
            BoundaryWriteChannel::Child,
            root.join(".git").join("t11-rogue-child"),
        ),
        planned(
            BoundaryWriteChannel::Child,
            member_b.join("t11-rogue-child"),
        ),
        planned(
            BoundaryWriteChannel::Child,
            non_target_wt.join("t11-rogue-child"),
        ),
        planned(BoundaryWriteChannel::Child, target.join(".git")),
        // 越界 symlink:target 内链接指向 root 受保护文件,写经链接必须被拒。
        planned(BoundaryWriteChannel::Builtin, escape_link.to_path_buf()),
        planned(BoundaryWriteChannel::Child, escape_link.to_path_buf()),
    ]
}

/// 逐路径内容 digest(absent=哨兵;symlink 走目标文件内容;目录形态以
/// 条目名 digest 计)。
fn task11_path_digests(planned: &[PlannedBoundaryWrite]) -> Vec<String> {
    planned
        .iter()
        .map(|write| match std::fs::symlink_metadata(write.path()) {
            Ok(metadata) if metadata.is_dir() => {
                let mut names: Vec<String> = std::fs::read_dir(write.path())
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|entry| entry.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                names.sort();
                format!("dir:{}", task11_sha256_hex(names.join("\n").as_bytes()))
            }
            Ok(_) => match std::fs::read(write.path()) {
                Ok(bytes) => format!("sha256:{}", task11_sha256_hex(&bytes)),
                Err(_) => "unreadable".to_string(),
            },
            Err(_) => "absent".to_string(),
        })
        .collect()
}

/// 受控 fixture 上的五通道×保护面真实探针(产品写边界沙箱内执行)。
async fn run_task11_write_matrix(
    env: &MatrixEnvironment,
) -> Result<Task11WriteMatrix, LiveMatrixFailure> {
    fp_enter_phase("t11-boundary", "write_matrix", "fresh");
    let Some((cli_program, _)) = provider_probe_channel(&env.provider) else {
        return Err(matrix_failure(
            "capability_probe_provider_unsupported",
            format!("provider {:?} 无真实探针通道", env.provider),
            None,
        ));
    };
    let launcher = ProviderBoundaryLauncher::probe_environment();
    if !launcher.is_available() {
        return Err(matrix_failure(
            "boundary_launcher_unavailable",
            "bwrap/user namespace 不可用:写面探针无法真实执行(fail-closed,不冒充)".to_string(),
            None,
        ));
    }
    let base = TempDir::new().expect("t11 write matrix base");
    let label = format!("t11-write-matrix-{}", Utc::now().format("%Y%m%dT%H%M%SZ"));
    let fixture = BoundaryFixture::create(
        env.provider.clone(),
        cli_program,
        SessionPolicyAction::CodingTargetWrite,
        base.path(),
        &env.evidence_root.join("boundary"),
        &label,
    )
    .map_err(|error| {
        matrix_failure(
            "boundary_fixture_invalid",
            format!("受控 fixture 搭建失败: {error:?}"),
            None,
        )
    })?;
    // 非 target worktree 面:root 仓自身 worktree(沙箱 ro-bind / 恒可见,
    // 越界写必得真实 EROFS 而非 ENOENT)。
    let non_target_wt = fixture.root().join("t11-non-target-wt");
    {
        let git = |arguments: &[&str]| -> std::io::Result<()> {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(fixture.root())
                .args(["-c", "user.email=t11@aria", "-c", "user.name=t11"])
                .args(arguments)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!(
                    "git {arguments:?} 失败:{status:?}"
                )))
            }
        };
        // member-b 补 git init(受控材料):使「非 target main 的 .git」面
        // 真实在场,越界写得真实 EROFS 而非 ENOENT。
        let member_b_init = std::process::Command::new("git")
            .arg("-C")
            .arg(fixture.member_b())
            .args(["init", "-q"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status();
        if !matches!(&member_b_init, Ok(status) if status.success()) {
            return Err(matrix_failure(
                "boundary_fixture_invalid",
                format!(
                    "member-b git init 失败:{:?}",
                    member_b_init.map(|status| status.to_string())
                ),
                None,
            ));
        }
        fp("t11_member_b_git_ready", "非 target main .git 面在场");
        git(&[
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "t11 non-target worktree base",
        ])
        .map_err(|error| {
            matrix_failure(
                "boundary_fixture_invalid",
                format!("非 target worktree 基提交失败: {error}"),
                None,
            )
        })?;
        git(&[
            "worktree",
            "add",
            "-q",
            non_target_wt.to_string_lossy().as_ref(),
            "-b",
            "t11-non-target",
        ])
        .map_err(|error| {
            matrix_failure(
                "boundary_fixture_invalid",
                format!("非 target worktree 挂载失败: {error}"),
                None,
            )
        })?;
    }
    fp(
        "t11_non_target_worktree_ready",
        format_args!("{}", non_target_wt.display()),
    );
    // 越界 symlink:target(可写面)内链接指向 root 受保护文件。
    let escape_link = fixture
        .target()
        .expect("coding fixture target")
        .join("t11-escape-link");
    std::os::unix::fs::symlink(fixture.root().join("AGENTS.md"), &escape_link).map_err(
        |error| {
            matrix_failure(
                "boundary_fixture_invalid",
                format!("越界 symlink 搭建失败: {error}"),
                None,
            )
        },
    )?;
    let planned = planned_task11_writes(&fixture, &non_target_wt, &escape_link);
    fp("t11_boundary_digest", "pre");
    let digests_before = task11_path_digests(&planned);
    // 与 6c derive_boundary_plan 同构的 plan(TargetWriteOnly,root,protect=[])
    // + 同一 env overlay,沙箱语义与产品探针逐字节一致。
    let plan = ProviderBoundaryPlan::probe_plan(
        ProviderBoundaryMode::TargetWriteOnly,
        fixture.root().to_path_buf(),
        fixture.target().map(Path::to_path_buf),
        Vec::new(),
    )
    .map_err(|error| {
        matrix_failure(
            "boundary_plan_invalid",
            format!("探测 plan 构造失败: {error:?}"),
            None,
        )
    })?;
    let sandbox_env = fixture.probe_env();
    fp(
        "t11_boundary_probe_run",
        format_args!("attempts={} launcher=bwrap", planned.len()),
    );
    let attempts = run_write_surface_probe(&launcher, &plan, &sandbox_env, &planned)
        .await
        .map_err(|error| {
            matrix_failure(
                "boundary_probe_channel_failed",
                format!("写面探针通道失败(观测前): {error:?}"),
                None,
            )
        })?;
    fp(
        "t11_boundary_probe_done",
        format_args!("attempts={}", attempts.len()),
    );
    fp("t11_boundary_digest", "post");
    let digests_after = task11_path_digests(&planned);
    if attempts.len() != planned.len() {
        return Err(matrix_failure(
            "boundary_probe_attempt_count_mismatch",
            format!("探针回报 {} 条 != 计划 {}", attempts.len(), planned.len()),
            None,
        ));
    }
    // 分类:每条必须被拒且带真实拒绝原文(模型「不写」不是 attempt 证据),
    // 且受保护文件 digest 前后一致;任一违例=防护失效 → 保留现场并阻断。
    let mut records = Vec::new();
    let mut violation: Option<String> = None;
    for (index, attempt) in attempts.iter().enumerate() {
        let refused_with_evidence = attempt.was_refused_with_evidence();
        let digest_before = digests_before[index].clone();
        let digest_after = digests_after[index].clone();
        let unchanged = digest_before == digest_after;
        let allowed = attempt.result() == Ok(());
        if allowed || !refused_with_evidence || !unchanged {
            violation = Some(violation.unwrap_or_else(|| {
                format!(
                    "channel={:?} path={} allowed={allowed} refused_with_evidence=\
                     {refused_with_evidence} digest_unchanged={unchanged}",
                    attempt.channel(),
                    attempt.path().display()
                )
            }));
        }
        fp(
            "t11_boundary_attempt",
            format_args!(
                "channel={:?} path={} outcome={} evidence={}",
                attempt.channel(),
                attempt.path().display(),
                if allowed { "allowed" } else { "refused" },
                attempt.evidence()
            ),
        );
        records.push(json!({
            "channel": format!("{:?}", attempt.channel()),
            "path": attempt.path().display().to_string(),
            "outcome": if allowed { "allowed" } else { "refused" },
            "refusal_text": attempt.evidence(),
            "digest_before": digest_before,
            "digest_after": digest_after,
        }));
    }
    if let Some(violation) = violation {
        let preserved = base.keep();
        fp(
            "t11_boundary_scene_preserved",
            format_args!("violation={violation} fixture={}", preserved.display()),
        );
        return Err(matrix_failure(
            "boundary_protection_violation",
            format!(
                "越界写未被拒或受保护面漂移:{violation};现场已保留 {}",
                preserved.display()
            ),
            None,
        ));
    }
    Ok(Task11WriteMatrix {
        attempts: records,
        fixture_base: base.keep(),
    })
}

/// Coding 正向格:6c 工件的受控 commit(subject+宿主复核+真实 git
/// add/commit argv)与正向 target 写(真实落盘)。
fn task11_coding_positive_cell(env: &MatrixEnvironment) -> Result<EvidenceCell, LiveMatrixFailure> {
    fp_enter_phase("t11-boundary", "coding_positive", "fresh");
    let (artifact_path, artifact) = latest_task11_coding_probe_artifact(env)?;
    let commit = artifact
        .get("controlled_commit")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            matrix_failure(
                "coding_positive_missing",
                format!("6c 工件缺受控 commit:{}", artifact_path.display()),
                None,
            )
        })?;
    let verified_on_host = commit
        .get("verified_on_host")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let subject = commit
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let positives = artifact
        .get("positives")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let target_write = positives.iter().find(|attempt| {
        attempt
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.ends_with("aria-boundary-probe.txt"))
            && attempt.get("outcome").and_then(Value::as_str) == Some("allowed")
    });
    let Some(target_write) = target_write else {
        return Err(matrix_failure(
            "coding_positive_missing",
            format!("6c 工件缺正向 target 写:{}", artifact_path.display()),
            None,
        ));
    };
    if !verified_on_host || subject.trim().is_empty() {
        return Err(matrix_failure(
            "coding_positive_missing",
            format!(
                "6c 受控 commit 未宿主复核:{},subject={subject:?}",
                artifact_path.display()
            ),
            None,
        ));
    }
    fp(
        "t11_coding_positive_verified",
        format_args!("subject={subject:?} artifact={}", artifact_path.display()),
    );
    let controlled_file = target_write
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut events = vec![json!({
        "ts": task11_now_ts(),
        "event": {
            "type": "boundary_matrix_evidence",
            "coding_target_controlled_file_written": true,
            "controlled_file": controlled_file,
            "controlled_file_digest": format!(
                "sha256:{}",
                task11_sha256_hex(b"aria-boundary-probe")
            ),
            "controlled_commit_subject": subject,
            "git_add_commit_argv": commit.get("argv").cloned().unwrap_or(Value::Null),
            "probe_artifact": artifact_path.display().to_string(),
        },
    })];
    // 会计观测:resume 段真实 CLI 子进程(launch/同 id resume/错 id 负探针)
    // 与 native 协议交互——真实观察,逐条带 argv;错 id 负探针是「握手失败
    // 已创建=kill/reap」形态(child 创建→失败退出),不属零 spawn 格。
    if let Some(resume) = artifact.get("resume").and_then(Value::as_object) {
        if resume.get("probed").and_then(Value::as_bool) == Some(true) {
            for (phase, argv_key, exit_key, id_key) in [
                (
                    "launch",
                    "launch_argv",
                    "launch_exit",
                    "launch_native_session_id",
                ),
                (
                    "resume",
                    "resume_argv",
                    "resume_exit",
                    "resume_native_session_id",
                ),
                (
                    "wrong_id_negative",
                    "wrong_id_argv",
                    "wrong_id_exit",
                    "wrong_id_native_session_id",
                ),
            ] {
                let argv = resume.get(argv_key).cloned().unwrap_or(Value::Null);
                let argv_non_empty = argv.as_array().is_some_and(|items| !items.is_empty());
                if !argv_non_empty {
                    continue;
                }
                events.push(json!({
                    "ts": task11_now_ts(),
                    "event": {
                        "type": "session_child_observed",
                        "phase": phase,
                        "argv": argv,
                        "exit": resume.get(exit_key).cloned().unwrap_or(Value::Null),
                    },
                }));
                events.push(json!({
                    "ts": task11_now_ts(),
                    "event": {
                        "type": "native_method_observed",
                        "phase": phase,
                        "native_session_id": resume.get(id_key).cloned().unwrap_or(Value::Null),
                    },
                }));
                if phase == "wrong_id_negative" {
                    events.push(json!({
                        "ts": task11_now_ts(),
                        "event": {
                            "type": "probe_child_kill_reap_observed",
                            "note": "握手失败已创建=kill/reap(child 创建→失败退出;spawn≥1,不属零 spawn 格)",
                            "argv": argv,
                            "exit": resume.get(exit_key).cloned().unwrap_or(Value::Null),
                        },
                    }));
                }
            }
        }
    }
    let mut cell = task11_cell_base(
        env,
        format!(
            "t11-boundary-coding-positive-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ),
    );
    // 本格证据面是探针工件(其 fixture 为已回收 tempdir):cwd/target 不
    // 主张矩阵 env 路径,真实路径落在事件与 probe_artifact。
    cell.process_cwd = PathBuf::new();
    cell.target = PathBuf::new();
    cell.capability_state = "confirmed".to_string();
    cell.completed_product_artifact_exists = true;
    cell.provider_events = events;
    Ok(cell)
}

/// 保护面拒绝格:组装层五通道×保护面逐条拒绝原文+digest,叠加 6c 工件
/// 四通道负向与 D4 受保护面 pre/post 摘要。
fn task11_protected_writes_cell(
    env: &MatrixEnvironment,
    write_matrix: &Task11WriteMatrix,
) -> Result<EvidenceCell, LiveMatrixFailure> {
    fp_enter_phase("t11-boundary", "protected_writes", "fresh");
    let (artifact_path, artifact) = latest_task11_coding_probe_artifact(env)?;
    let negatives = artifact
        .get("negatives")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let all_6c_refused = !negatives.is_empty()
        && negatives.iter().all(|attempt| {
            attempt.get("outcome").and_then(Value::as_str) == Some("refused")
                && attempt
                    .get("evidence")
                    .and_then(Value::as_str)
                    .is_some_and(|evidence| !evidence.trim().is_empty())
        });
    let all_matrix_refused = write_matrix.attempts.iter().all(|attempt| {
        attempt.get("outcome").and_then(Value::as_str) == Some("refused")
            && attempt
                .get("refusal_text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
            && attempt.get("digest_before") == attempt.get("digest_after")
    });
    // 6c 工件的 D4(受保护面 pre/post 摘要;缺证据=哨兵,现场断言红)。
    let d4 = artifact.get("d4").cloned().unwrap_or(Value::Null);
    let protected_digest_before = d4
        .get("protected_pre_digest")
        .and_then(Value::as_str)
        .unwrap_or("evidence-missing#protected_digest_before")
        .to_string();
    let protected_digest_after = d4
        .get("protected_post_digest")
        .and_then(Value::as_str)
        .unwrap_or("evidence-missing#protected_digest_after")
        .to_string();
    let flag = all_6c_refused && all_matrix_refused;
    fp(
        "t11_protected_writes_verdict",
        format_args!(
            "all_refused={flag} matrix_attempts={} 6c_negatives={}",
            write_matrix.attempts.len(),
            negatives.len()
        ),
    );
    let mut events = vec![json!({
        "ts": task11_now_ts(),
        "event": {
            "type": "boundary_matrix_evidence",
            "protected_writes_all_refused_by_os_or_native_policy": flag,
            "protected_digest_before": protected_digest_before,
            "protected_digest_after": protected_digest_after,
            "attempt_count": write_matrix.attempts.len() + negatives.len(),
            "write_matrix_fixture": write_matrix.fixture_base.display().to_string(),
            "probe_artifact": artifact_path.display().to_string(),
        },
    })];
    // 逐条 attempt 留证(拒绝原文+digest);extension/MCP 通道探针在沙箱内
    // 派生真实后代进程,按 attempt 计入会计维度。
    for attempt in &write_matrix.attempts {
        let channel = attempt
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or_default();
        events.push(json!({
            "ts": task11_now_ts(),
            "event": {
                "type": "boundary_write_attempt",
                "attempt": attempt,
            },
        }));
        if matches!(channel, "Mcp" | "Extension" | "Terminal" | "Child") {
            events.push(json!({
                "ts": task11_now_ts(),
                "event": {
                    "type": "extension_mcp_descendant_observed",
                    "channel": channel,
                    "path": attempt.get("path").cloned().unwrap_or(Value::Null),
                },
            }));
        }
    }
    // availability `--version` 另记:6c 工件的真实版本探测(argv+结果)。
    events.push(json!({
        "ts": task11_now_ts(),
        "event": {
            "type": "availability_version_probe",
            "argv": artifact.get("cli_version_argv").cloned().unwrap_or(Value::Null),
            "exact_version": artifact.get("cli_exact_version").cloned().unwrap_or(Value::Null),
            "source": artifact_path.display().to_string(),
        },
    }));
    let mut cell = task11_cell_base(
        env,
        format!(
            "t11-boundary-protected-writes-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ),
    );
    cell.process_cwd = PathBuf::new();
    cell.target = PathBuf::new();
    cell.capability_state = if flag { "confirmed" } else { "denied" }.to_string();
    if !flag {
        cell.denied_reason =
            Some("保护面写未全部被拒(或拒绝无证据/受保护面漂移):防护失效,现场断言将红".to_string());
    }
    cell.provider_events = events;
    Ok(cell)
}

// ---------------------------------------------------------------------------
// D4:active main(HEAD+porcelain)+ root/metadata 快照。
// ---------------------------------------------------------------------------

struct Task11D4Snapshot {
    faces: Vec<Value>,
    root_metadata_digest: String,
    /// root/metadata 逐条目清单(rel,line)——与 digest 同源同预算。
    /// r63 kimi 现场:combined 漂移但 git faces 全等,漂移在 root/metadata
    /// 面且未逐面记录(可观测性缺口);本清单进 cell,漂移物自证。
    root_metadata_entries: Vec<(String, String)>,
    combined_digest: String,
}

/// root/metadata 面快照:聚合 digest + 逐条目清单(同一次遍历产出)。
struct Task11RootMetadataFace {
    digest: String,
    entries: Vec<(String, String)>,
}

/// root/metadata 有界递归面快照(跳过成员工作树——它们由 git 面覆盖;
/// 沿 HEAD 既有预算 20_000 条目/64 MiB,超限 fail-closed 不截断):
/// 聚合 digest 与逐条目清单(rel,line)同一次遍历产出——漂移可逐面实名
/// (r63 kimi 现场:combined 漂移但 git faces 全等,root/metadata 面
/// 未逐面记录导致漂移物无法自证)。
fn task11_root_metadata_face(
    canonical_root: &Path,
) -> Result<Task11RootMetadataFace, LiveMatrixFailure> {
    const MAX_ENTRIES: usize = 20_000;
    const MAX_BYTES: u64 = 64 * 1024 * 1024;
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut total_bytes = 0u64;
    fn walk(
        directory: &Path,
        relative: &str,
        entries: &mut Vec<(String, String)>,
        total_bytes: &mut u64,
    ) -> Result<(), String> {
        let children = std::fs::read_dir(directory)
            .map_err(|error| format!("read_dir {}: {error}", directory.display()))?;
        for child in children.flatten() {
            let name = child.file_name().to_string_lossy().into_owned();
            let child_relative = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            // DirEntry::metadata 在 Unix 上即 lstat 语义(不随 symlink)。
            let metadata = child
                .metadata()
                .map_err(|error| format!("metadata {}: {error}", child.path().display()))?;
            if metadata.is_symlink() {
                let target = std::fs::read_link(child.path())
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "unreadable".to_string());
                entries.push((
                    child_relative.clone(),
                    format!("{child_relative}:link:{target}"),
                ));
            } else if metadata.is_dir() {
                walk(&child.path(), &child_relative, entries, total_bytes)?;
            } else if metadata.is_file() {
                let bytes = std::fs::read(child.path())
                    .map_err(|error| format!("read {}: {error}", child.path().display()))?;
                *total_bytes += bytes.len() as u64;
                entries.push((
                    child_relative.clone(),
                    format!(
                        "{child_relative}:{}:{}",
                        metadata.len(),
                        task11_sha256_hex(&bytes)
                    ),
                ));
            } else {
                // unix socket/fifo 等特殊文件:不可读字节,以类型标记入摘要
                //(在场性即元数据事实;不 fail,也不冒充内容)。
                entries.push((
                    child_relative.clone(),
                    format!("{child_relative}:special:{:?}", metadata.file_type()),
                ));
            }
            if entries.len() > MAX_ENTRIES || *total_bytes > MAX_BYTES {
                return Err(format!(
                    "root/metadata 快照超预算(>{MAX_ENTRIES} 条目或 {MAX_BYTES}B):fail-closed 不截断"
                ));
            }
        }
        Ok(())
    }
    let direct = std::fs::read_dir(canonical_root)
        .map_err(|error| {
            matrix_failure(
                "d4_snapshot_failed",
                format!("read_dir {}: {error}", canonical_root.display()),
                None,
            )
        })?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for name in direct {
        if matches!(name.as_str(), "alpha" | "beta") {
            continue;
        }
        if name == ".codegraph" {
            // 本机 codegraph watcher 的自写状态(非 fixture 材料/产品工件):
            // 排除出 root/metadata 面,理由随 fp 留痕;成员 git 面不受影响。
            fp(
                "t11_d4_face_excluded",
                ".codegraph(工具自写状态,非聚合根元数据)",
            );
            continue;
        }
        if name == ".provider-session-cache" {
            // r63 kimi 现场实证的漂移物:kimi LC Executor 会话构造时,
            // 产品 F-17 冻结面把 writable_git_paths.json 持久化到
            // `<target-parent>/.provider-session-cache/<key>/`(host 领土、
            // sandbox 只读、防 coder 改写 .git 指针的防御性缓存;
            // provider_boundary launcher 与 kimi client services 共用)。
            // 写入者是产品安全机制而非 provider 越界/模型写——非聚合根
            // 元数据,按 .codegraph 同款语义排除;git faces 不受影响,
            // 其余 root 路径越界写仍由本面抓取。
            fp(
                "t11_d4_face_excluded",
                ".provider-session-cache(产品 F-17 冻结面自管缓存,非聚合根元数据)",
            );
            continue;
        }
        let path = canonical_root.join(&name);
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            matrix_failure(
                "d4_snapshot_failed",
                format!("metadata {}: {error}", path.display()),
                None,
            )
        })?;
        if metadata.is_symlink() {
            let target = std::fs::read_link(&path)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "unreadable".to_string());
            entries.push((name.clone(), format!("{name}:link:{target}")));
        } else if metadata.is_dir() {
            walk(&path, &name, &mut entries, &mut total_bytes)
                .map_err(|reason| matrix_failure("d4_snapshot_failed", reason, None))?;
        } else if metadata.is_file() {
            let bytes = std::fs::read(&path).map_err(|error| {
                matrix_failure(
                    "d4_snapshot_failed",
                    format!("read {}: {error}", path.display()),
                    None,
                )
            })?;
            total_bytes += bytes.len() as u64;
            entries.push((
                name.clone(),
                format!("{name}:{}:{}", bytes.len(), task11_sha256_hex(&bytes)),
            ));
        } else {
            // unix socket/fifo 等特殊文件:以类型标记入摘要(在场性即
            // 元数据事实;不 fail,也不冒充内容)。
            entries.push((
                name.clone(),
                format!("{name}:special:{:?}", metadata.file_type()),
            ));
        }
        if entries.len() > MAX_ENTRIES || total_bytes > MAX_BYTES {
            return Err(matrix_failure(
                "d4_snapshot_failed",
                format!(
                    "root/metadata 快照超预算(>{MAX_ENTRIES} 条目或 {MAX_BYTES}B):fail-closed 不截断"
                ),
                None,
            ));
        }
    }
    entries.sort();
    let digest = format!(
        "sha256:{}",
        task11_sha256_hex(
            entries
                .iter()
                .map(|(_, line)| line.as_str())
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes()
        )
    );
    Ok(Task11RootMetadataFace { digest, entries })
}

/// D4 快照:所有 active main(alpha/beta 主 checkout,HEAD+porcelain)+
/// root/metadata(预算内全量)。
fn task11_d4_snapshot(env: &MatrixEnvironment) -> Result<Task11D4Snapshot, LiveMatrixFailure> {
    let mut faces = Vec::new();
    for (face, path) in [
        ("alpha-main", env.member_worktree.clone()),
        ("beta-main", env.canonical_root.join("beta")),
    ] {
        let snapshot = git_snapshot(&path);
        let digest = format!(
            "sha256:{}",
            task11_sha256_hex(
                serde_json::to_string(&snapshot)
                    .unwrap_or_default()
                    .as_bytes()
            )
        );
        fp(
            "t11_d4_face",
            format_args!("{face} head={:?} digest={digest}", snapshot["head"]),
        );
        faces.push(json!({
            "face": face,
            "path": path.display().to_string(),
            "head": snapshot["head"],
            "porcelain": snapshot["status"],
            "digest": digest,
        }));
    }
    let root_metadata = task11_root_metadata_face(&env.canonical_root)?;
    fp(
        "t11_d4_root_metadata",
        format_args!(
            "digest={} entries={}",
            root_metadata.digest,
            root_metadata.entries.len()
        ),
    );
    let root_metadata_digest = root_metadata.digest;
    let mut combined = String::new();
    for face in &faces {
        combined.push_str(
            face.get("digest")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        combined.push('\n');
    }
    combined.push_str(&root_metadata_digest);
    Ok(Task11D4Snapshot {
        faces,
        root_metadata_digest,
        root_metadata_entries: root_metadata.entries,
        combined_digest: format!("sha256:{}", task11_sha256_hex(combined.as_bytes())),
    })
}

/// D4 收口格:前后快照逐面对比,非 target 漂移即格 denied(现场断言红)。
/// r63 起记录 root/metadata 拆面 + 逐条目清单与差分——漂移物自证
/// (r63 kimi 现场:git faces 全等+combined 漂移,聚合 digest 无实径可查)。
fn task11_d4_cell(
    env: &MatrixEnvironment,
    before: &Task11D4Snapshot,
    after: &Task11D4Snapshot,
) -> EvidenceCell {
    let no_drift = before.combined_digest == after.combined_digest;
    // root/metadata 拆面进 faces 记录(与 git 面同列;digest 即面聚合)。
    let root_metadata_face = |digest: &str, entries: &[(String, String)]| {
        json!({
            "face": "root-metadata",
            "path": env.canonical_root.display().to_string(),
            "digest": digest,
            "entry_count": entries.len(),
        })
    };
    let mut faces_before = before.faces.clone();
    faces_before.push(root_metadata_face(
        &before.root_metadata_digest,
        &before.root_metadata_entries,
    ));
    let mut faces_after = after.faces.clone();
    faces_after.push(root_metadata_face(
        &after.root_metadata_digest,
        &after.root_metadata_entries,
    ));
    // 拆面对比:哪些面漂了(逐面 digest 对齐;faces 同构等长)。
    let drifted_faces: Vec<String> = faces_before
        .iter()
        .zip(faces_after.iter())
        .filter(|(before_face, after_face)| before_face.get("digest") != after_face.get("digest"))
        .filter_map(|(before_face, _)| {
            before_face
                .get("face")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let root_metadata_drift = task11_root_metadata_entry_diff(
        &before.root_metadata_entries,
        &after.root_metadata_entries,
    );
    let mut cell = task11_cell_base(
        env,
        format!("t11-boundary-d4-{}", Utc::now().format("%Y%m%dT%H%M%SZ")),
    );
    cell.process_cwd = PathBuf::new();
    cell.target = PathBuf::new();
    cell.capability_state = if no_drift { "confirmed" } else { "denied" }.to_string();
    if !no_drift {
        cell.denied_reason = Some(format!(
            "D4 非 target 漂移:pre={} post={} drifted_faces={drifted_faces:?} \
             root_metadata_drift={}",
            before.combined_digest,
            after.combined_digest,
            task11_root_metadata_drift_summary(&root_metadata_drift),
        ));
    }
    cell.provider_events = vec![json!({
        "ts": task11_now_ts(),
        "event": {
            "type": "boundary_matrix_evidence",
            "d4_detected_or_delivered_without_non_target_drift": no_drift,
            "combined_digest_before": before.combined_digest,
            "combined_digest_after": after.combined_digest,
            "root_metadata_digest_before": before.root_metadata_digest,
            "root_metadata_digest_after": after.root_metadata_digest,
            "drifted_faces": drifted_faces,
            "root_metadata_drift": root_metadata_drift,
            "root_metadata_entries_before": before
                .root_metadata_entries
                .iter()
                .map(|(_, line)| line.as_str())
                .collect::<Vec<_>>(),
            "root_metadata_entries_after": after
                .root_metadata_entries
                .iter()
                .map(|(_, line)| line.as_str())
                .collect::<Vec<_>>(),
            "faces_before": faces_before,
            "faces_after": faces_after,
        },
    })];
    cell
}

/// root/metadata 逐条目漂移差分(漂移物自证):按 rel 对齐前后清单,
/// added/removed/changed 三列实名;changed 携带前后条目原文。
fn task11_root_metadata_entry_diff(
    before: &[(String, String)],
    after: &[(String, String)],
) -> Value {
    let before_map: BTreeMap<&str, &str> = before
        .iter()
        .map(|(rel, line)| (rel.as_str(), line.as_str()))
        .collect();
    let after_map: BTreeMap<&str, &str> = after
        .iter()
        .map(|(rel, line)| (rel.as_str(), line.as_str()))
        .collect();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (rel, line) in &after_map {
        match before_map.get(*rel) {
            None => added.push(json!(rel)),
            Some(old) if *old != *line => {
                changed.push(json!({"path": rel, "before": old, "after": line}))
            }
            _ => {}
        }
    }
    for rel in before_map.keys() {
        if !after_map.contains_key(*rel) {
            removed.push(json!(rel));
        }
    }
    json!({"added": added, "removed": removed, "changed": changed})
}

/// 漂移差分紧凑摘要(denied_reason 用;每列实名至多 8 条+总数,防清单爆破)。
fn task11_root_metadata_drift_summary(diff: &Value) -> String {
    let column = |key: &str| {
        let count = diff
            .get(key)
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let listed: Vec<String> = diff
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(8)
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .or_else(|| {
                                item.get("path").and_then(Value::as_str).map(str::to_string)
                            })
                            .unwrap_or_else(|| "?".to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        format!("{count}{listed:?}(至多列8)")
    };
    format!(
        "added={} removed={} changed={}",
        column("added"),
        column("removed"),
        column("changed")
    )
}

// ---------------------------------------------------------------------------
// 失败段:17 固定负向场景逐格真实拒绝(零 spawn 会计三面证明)。
// ---------------------------------------------------------------------------

/// 单场景真实探测结果。
struct ScenarioOutcome {
    /// 真实拒绝是否发生(被真实产品面拒绝)。
    refused: bool,
    /// 拒绝发生的真实产品面(validate/prepare/spawn 前复验/…)。
    refusal_gate: String,
    /// 真实拒绝原文(或不可观察说明)。
    refusal_text: String,
    /// 落格三态:denied(被拒/防护失效)/unknown(不可观察)。
    state: &'static str,
    detail: Value,
    extra_events: Vec<Value>,
}

fn scenario_refused(gate: impl Into<String>, text: impl Into<String>) -> ScenarioOutcome {
    ScenarioOutcome {
        refused: true,
        refusal_gate: gate.into(),
        refusal_text: text.into(),
        state: "denied",
        detail: Value::Null,
        extra_events: Vec::new(),
    }
}

fn scenario_not_refused(gate: impl Into<String>, detail: String) -> ScenarioOutcome {
    let gate = gate.into();
    ScenarioOutcome {
        refused: false,
        refusal_text: format!(
            "零 spawn 被拒验证失败[{gate}]:复验未拒,{detail}(真实 spawn 已立即收口;证据保留,现场断言将红)"
        ),
        refusal_gate: gate,
        state: "denied",
        detail: Value::Null,
        extra_events: Vec::new(),
    }
}

fn scenario_unobservable(text: String) -> ScenarioOutcome {
    ScenarioOutcome {
        refused: false,
        refusal_gate: "unobservable".to_string(),
        refusal_text: text,
        state: "unknown",
        detail: Value::Null,
        extra_events: Vec::new(),
    }
}

/// 基线请求:与产品 coder root launch 同构(cwd=canonical root、target/writable
/// =成员 worktree、config artifact=生产约定常量)。
fn task11_coding_request(env: &MatrixEnvironment) -> SessionLaunchRequest {
    let provider = ProviderRef::from_provider_name(&env.provider, "cap_managed_snapshot")
        .expect("real provider ref");
    SessionLaunchRequest {
        project_id: PROJECT_ID.to_string(),
        provider,
        action: SessionPolicyAction::CodingTargetWrite,
        target: PolicyTarget::checkout(
            env.member_logical_id.clone(),
            env.member_checkout_id.clone(),
            env.member_worktree.clone(),
        ),
        working_directory: env.canonical_root.clone(),
        readable_roots: vec![env.canonical_root.clone()],
        writable_roots: vec![env.member_worktree.clone()],
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    }
}

/// 基线流式输入(Coder/Executor 语义:无通用 tool policy)。
fn task11_streaming_input(env: &MatrixEnvironment) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: provider_type_for(&env.provider),
        role: AdapterRole::Executor,
        prompt: "aria t11 failure-matrix probe: reply with exactly: t11-no-spawn".to_string(),
        working_dir: env.canonical_root.clone(),
        working_directory: Some(env.canonical_root.clone()),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        tool_policy: None,
        audit_sink: None,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
        baseline_tree: None,
    }
}

fn task11_audit_context(env: &MatrixEnvironment, sid: &str) -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: sid.to_string(),
        role_run_seq: 0,
        audit_sink: Arc::new(env.lifecycle.clone()),
    }
}

fn task11_readonly_gateway(
    env: &MatrixEnvironment,
) -> Result<LogicalCodebaseProviderGateway, String> {
    env.gateway_factory
        .build_readonly_for_lc(PROJECT_ID, Some(&env.lc_id))
        .map_err(|error| error.to_string())
}

/// 组装层 gateway(真实组件装配):policies/capabilities/targets 均生产
/// for_lc 源;registry 空(validate/spawn 前复验不触达 registry);sync
/// adapter 未行使(本段只走流式拒绝面);availability gate 用真实
/// ProviderHealthService 的未刷新快照(readiness 未建立,缺 readiness 场景
/// 的受控负向;trust 场景的拒绝发生在 #4b,先于 availability #9)。
fn task11_assembled_gateway(
    env: &MatrixEnvironment,
    trust: Option<Arc<dyn ProviderTrustSource>>,
) -> Result<LogicalCodebaseProviderGateway, String> {
    let gateway = LogicalCodebaseProviderGateway::new(
        AggregatePolicyArtifactStore::for_lc(env.app_paths.clone(), env.lc_id.clone()),
        Arc::new(StoreBackedProviderCapabilitySource::for_lc(
            env.app_paths.clone(),
            PROJECT_ID.to_string(),
            env.lc_id.clone(),
        )),
        Arc::new(ProductionPolicyTargetResolver::for_lc(
            env.app_paths.clone(),
            &env.lc_id,
        )),
        Arc::new(ProviderRegistry::new()),
        Arc::new(Task11UnexercisedSyncAdapter),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(
            ProviderHealthService::new(env.workspace_root_path.clone()),
        ))),
        env.canonical_root.clone(),
    );
    let Some(trust) = trust else {
        return Ok(gateway);
    };
    Ok(gateway.with_readonly_lc_facts(
        RootRecipeReceiptStore::for_lc(env.app_paths.clone(), env.lc_id.clone()),
        trust,
        Some(env.lc_id.clone()),
    ))
}

/// 未行使的 sync adapter 占位(组装 gateway 的 run_sync 面不在 Task 11
/// 探测范围;被调用即显式失败,不冒充)。
struct Task11UnexercisedSyncAdapter;

impl ProviderAdapter for Task11UnexercisedSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "t11 组装 gateway 的 sync adapter 未行使(本段只走流式拒绝面)",
            0,
        ))
    }
}

/// capability 记录受控突变守卫(Drop 恢复原记录;原缺失时如实留痕——
/// store 无删除 API,受控行保留并记录)。
struct Task11CapabilityRestore {
    store: ProviderCapabilityStore,
    original: Option<ProviderCapabilityRecord>,
    provider_type: ProviderRefType,
}

impl Task11CapabilityRestore {
    fn capture(env: &MatrixEnvironment, provider_type: ProviderRefType) -> Self {
        let store = ProviderCapabilityStore::for_lc(env.app_paths.clone(), env.lc_id.clone());
        let original = store.get(PROJECT_ID, provider_type).ok().flatten();
        Self {
            store,
            original,
            provider_type,
        }
    }

    fn fixture_record(provider_type: ProviderRefType) -> ProviderCapabilityRecord {
        // 受控 setup 行(只为通过 capability 门到达被测面;版本/证据引用
        // 如实标注 fixture,不冒充探针签发)。
        let (adapter_dialect, wire_dialect) = match provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        let row = ProviderActionCapability {
            action: SessionPolicyAction::CodingTargetWrite,
            launch: ProviderCapabilityEvidence::Confirmed,
            resume: ProviderCapabilityEvidence::Unknown,
            write_boundary: ProviderCapabilityEvidence::Confirmed,
            projection_digest: "sha256:t11-fixture-row".to_string(),
            evidence_ref: "t11-fixture(受控 setup,非探针签发)".to_string(),
        };
        ProviderCapabilityRecord {
            provider_type,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: "0.0.0-t11-fixture".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: cadence_aria::product::logical_codebase::provider_gateway::ResumeEvidenceState::Unsupported,
            supported_actions: vec![SessionPolicyAction::CodingTargetWrite],
            action_matrix: ProviderActionMatrix::from_rows(vec![row]).expect("t11 fixture rows"),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: None,
            probe_artifact_ref: None,
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }
}

impl Drop for Task11CapabilityRestore {
    fn drop(&mut self) {
        match &self.original {
            Some(original) => {
                if let Err(error) = self.store.upsert(PROJECT_ID, original) {
                    fp(
                        "t11_capability_restore_failed",
                        format_args!("{:?}: {error:?}", self.provider_type),
                    );
                } else {
                    fp(
                        "t11_capability_restored",
                        format_args!("{:?}", self.provider_type),
                    );
                }
            }
            None => fp(
                "t11_capability_restore_absent_origin",
                format_args!(
                    "{:?} 原记录缺失:受控行保留(store 无删除 API,不影响本家行)",
                    self.provider_type
                ),
            ),
        }
    }
}

/// 受控突变所选 provider 的 capability 记录(守卫 Drop 恢复)。
fn task11_mutate_selected_record(
    env: &MatrixEnvironment,
    mutate: impl FnOnce(&mut ProviderCapabilityRecord),
) -> Result<Task11CapabilityRestore, String> {
    let provider = ProviderRef::from_provider_name(&env.provider, "cap_managed_snapshot")
        .map_err(|error| error.to_string())?;
    let guard = Task11CapabilityRestore::capture(env, provider.provider_type);
    let mut record = guard
        .original
        .clone()
        .unwrap_or_else(|| Task11CapabilityRestore::fixture_record(provider.provider_type));
    mutate(&mut record);
    guard
        .store
        .upsert(PROJECT_ID, &record)
        .map_err(|error| error.to_string())?;
    fp(
        "t11_capability_mutated",
        format_args!("{:?}", provider.provider_type),
    );
    Ok(guard)
}

/// 文件字节守卫(Drop 原字节恢复;路径缺失则 Drop 删除——受控恢复)。
struct Task11FileRestore {
    path: PathBuf,
    original: Option<Vec<u8>>,
}

impl Task11FileRestore {
    fn capture(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            original: std::fs::read(path).ok(),
        }
    }
}

impl Drop for Task11FileRestore {
    fn drop(&mut self) {
        match &self.original {
            Some(bytes) => {
                let _ = std::fs::write(&self.path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&self.path);
            }
        }
        fp("t11_file_restored", format_args!("{}", self.path.display()));
    }
}

/// 基线 prepare(真实 gateway 的 validate+role guard)。
async fn task11_prepare_baseline(
    env: &MatrixEnvironment,
    gateway: &LogicalCodebaseProviderGateway,
    sid: &str,
    input: StreamingProviderInput,
    request: SessionLaunchRequest,
) -> Result<
    cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
    ScenarioOutcome,
> {
    gateway
        .prepare_streaming_launch(input, request, task11_audit_context(env, sid))
        .map_err(|error| scenario_refused("prepare(validate+role guard)", error.to_string()))
}

/// 受控突变守卫(持有至 spawn 前复验完成;Drop 恢复)。
enum Task11MutationGuard {
    Capability(Task11CapabilityRestore),
    File(Task11FileRestore),
    None,
}

/// spawn 前复验驱动的漂移场景共用骨架:prepare(基线全绿)→ 受控突变
///(守卫持有)→ start_streaming(复验在 registry lookup/真实 adapter
/// 之前 fail-closed)→ Drop 恢复。
async fn task11_revalidate_drift(
    env: &MatrixEnvironment,
    sid: &str,
    dimension: &str,
    mutate: impl FnOnce(&MatrixEnvironment) -> Result<Task11MutationGuard, String>,
) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let prepared = match task11_prepare_baseline(
        env,
        &gateway,
        sid,
        task11_streaming_input(env),
        task11_coding_request(env),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    fp(
        "t11_scenario_mutation",
        format_args!("dimension={dimension}"),
    );
    let guard = match mutate(env) {
        Ok(guard) => guard,
        Err(problem) => return scenario_unobservable(format!("受控突变失败:{problem}")),
    };
    let cancel = CancellationToken::new();
    let started = gateway.start_streaming(prepared, cancel.clone()).await;
    drop(guard);
    match started {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("dimension={dimension} error={error}"),
            );
            scenario_refused(format!("spawn 前复验({dimension})"), error.to_string())
        }
        Ok(session) => {
            drop(session);
            cancel.cancel();
            fp(
                "t11_scenario_unexpected_spawn",
                format_args!("dimension={dimension}(已 cancel+drop 收口)"),
            );
            scenario_not_refused(
                format!("spawn 前复验({dimension})"),
                format!("维度 {dimension} 漂移未被复验拒绝"),
            )
        }
    }
}

/// 场景 1:missing_readiness——readiness(health)未建立时 spawn 前复验 #9
/// 以真实 availability 门拒绝(未刷新 health 快照=受控负向,零 spawn)。
async fn scenario_missing_readiness(env: &MatrixEnvironment, sid: &str) -> ScenarioOutcome {
    let gateway = match task11_assembled_gateway(env, None) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("组装 gateway 失败:{error}")),
    };
    let prepared = match task11_prepare_baseline(
        env,
        &gateway,
        sid,
        task11_streaming_input(env),
        task11_coding_request(env),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    let cancel = CancellationToken::new();
    let started = gateway.start_streaming(prepared, cancel.clone()).await;
    match started {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=availability error={error}"),
            );
            scenario_refused("spawn 前复验(#9 availability/readiness)", error.to_string())
        }
        Ok(session) => {
            drop(session);
            cancel.cancel();
            scenario_not_refused(
                "spawn 前复验(#9 availability/readiness)",
                "readiness 未建立未被 availability 门拒绝".to_string(),
            )
        }
    }
}

/// 场景 2:missing_body——#8 发布链正文缺失(validate 的 receipt 链正文
/// 复验读取失败)。
fn scenario_missing_body(env: &MatrixEnvironment) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let artifact =
        match AggregatePolicyArtifactStore::for_lc(env.app_paths.clone(), env.lc_id.clone())
            .get(PROJECT_ID)
        {
            Ok(Some(artifact)) => artifact,
            _ => return scenario_unobservable("policy artifact 缺失".to_string()),
        };
    let body_path = gateway.authority_root().join(&artifact.policy_id);
    if std::fs::read(&body_path).is_err() {
        return scenario_unobservable(format!(
            "发布正文不可读(无法建立基线):{}",
            body_path.display()
        ));
    }
    let _restore = Task11FileRestore::capture(&body_path);
    if let Err(error) = std::fs::remove_file(&body_path) {
        return scenario_unobservable(format!("受控移除正文失败:{error}"));
    }
    fp(
        "t11_scenario_mutation",
        format_args!("policy body 移除:{}", body_path.display()),
    );
    match gateway.validate(task11_coding_request(env)) {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=policy_body error={error}"),
            );
            scenario_refused("validate(#8 发布链正文复验)", error.to_string())
        }
        Ok(_) => scenario_not_refused(
            "validate(#8 发布链正文复验)",
            "正文缺失未被 validate 拒绝".to_string(),
        ),
    }
}

/// 场景 3:missing_receipt——finalized receipt 的 policy_digest 被篡改
///(#8 receipt 链断裂:有效 receipt 缺失),validate 的三方一致校验拒绝。
fn scenario_missing_receipt(env: &MatrixEnvironment) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let receipts_root = env
        .app_paths
        .logical_codebases_root(PROJECT_ID)
        .join(&env.lc_id)
        .join("aggregate-recipe-receipts");
    let mut latest: Option<(String, PathBuf)> = None;
    let Ok(entries) = std::fs::read_dir(&receipts_root) else {
        return scenario_unobservable(format!("receipts 目录不可读:{}", receipts_root.display()));
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !path.to_string_lossy().ends_with(".json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let finalized_at = value
            .get("finalized_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if finalized_at.is_empty() {
            continue;
        }
        if latest.as_ref().is_none_or(|(seen, _)| *seen < finalized_at) {
            latest = Some((finalized_at, path));
        }
    }
    let Some((finalized_at, receipt_path)) = latest else {
        return scenario_unobservable(format!(
            "finalized receipt 未找到于 {}",
            receipts_root.display()
        ));
    };
    let Ok(mut value) =
        serde_json::from_str::<Value>(&std::fs::read_to_string(&receipt_path).unwrap_or_default())
    else {
        return scenario_unobservable("finalized receipt 解析失败".to_string());
    };
    let _restore = Task11FileRestore::capture(&receipt_path);
    value["policy_digest"] = json!("sha256:t11-receipt-chain-broken");
    if let Err(error) = std::fs::write(
        &receipt_path,
        serde_json::to_vec(&value).unwrap_or_default(),
    ) {
        return scenario_unobservable(format!("受控篡改 receipt 失败:{error}"));
    }
    fp(
        "t11_scenario_mutation",
        format_args!(
            "receipt 链断裂(有效 receipt 缺失):{} finalized_at={finalized_at}",
            receipt_path.display()
        ),
    );
    match gateway.validate(task11_coding_request(env)) {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=policy_digest_chain error={error}"),
            );
            scenario_refused("validate(#8 receipt 链三方一致)", error.to_string())
        }
        Ok(_) => scenario_not_refused(
            "validate(#8 receipt 链三方一致)",
            "receipt 链断裂未被 validate 拒绝".to_string(),
        ),
    }
}

/// 场景 4:missing_trust——需要 workspace trust 的 provider(codex/kimi)
/// 在未登记 HOME 上被 spawn 前复验 #4b 真实拒绝;无 trust 面的 provider
/// (claude/pi)按产品过滤语义记 Unknown(不冒充被拒)。
async fn scenario_missing_trust(env: &MatrixEnvironment, sid: &str) -> ScenarioOutcome {
    if !requires_workspace_trust(&env.provider) {
        return scenario_unobservable(format!(
            "provider {:?} 无用户级 workspace trust 面(requires_workspace_trust=false,\
             产品过滤语义真实观测):trust 缺失不可触发,记 Unknown 不冒充被拒",
            env.provider
        ));
    }
    // 受控 HOME(空 tempdir)上的真实 trust source(ReadonlyProviderTrustSource,
    // 与 factory 生产装配同类型):verify 未登记 → spawn 前复验 #4b 拒绝;
    // 不触碰真实 HOME。
    let empty_home = TempDir::new().expect("t11 empty trust home");
    let trust_source = ReadonlyProviderTrustSource::new(
        env.app_paths.clone(),
        Some(env.lc_id.clone()),
        vec![
            Arc::new(CodexTrustAdapter::for_home(empty_home.path())),
            Arc::new(KimiTrustAdapter::for_home(empty_home.path())),
        ],
    );
    let gateway = match task11_assembled_gateway(
        env,
        Some(Arc::new(trust_source) as Arc<dyn ProviderTrustSource>),
    ) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("组装 gateway 失败:{error}")),
    };
    let prepared = match task11_prepare_baseline(
        env,
        &gateway,
        sid,
        task11_streaming_input(env),
        task11_coding_request(env),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    let cancel = CancellationToken::new();
    let started = gateway.start_streaming(prepared, cancel.clone()).await;
    match started {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=trust error={error}"),
            );
            scenario_refused("spawn 前复验(#4b trust source)", error.to_string())
        }
        Ok(session) => {
            drop(session);
            cancel.cancel();
            scenario_not_refused(
                "spawn 前复验(#4b trust source)",
                "trust 未登记未被拒绝".to_string(),
            )
        }
    }
}

/// 场景 5-8:capability 分格门(launch/boundary × Unknown/Denied)——
/// validate 阶段真实拒绝。Unknown 需同时移出旧 allow 列表(过渡桥语义:
/// Unknown+列表内放行等待真实探针,列表外 fail-closed)。
fn scenario_capability_cell(env: &MatrixEnvironment, scenario: &str) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let guard = match task11_mutate_selected_record(env, |record| {
        let mut row = record
            .action_matrix
            .row(&SessionPolicyAction::CodingTargetWrite);
        match scenario {
            "launch_unknown" => {
                row.launch = ProviderCapabilityEvidence::Unknown;
                record
                    .supported_actions
                    .retain(|action| *action != SessionPolicyAction::CodingTargetWrite);
            }
            "launch_denied" => {
                row.launch =
                    ProviderCapabilityEvidence::denied("t11 受控负向:launch 分格携带真实负向证据");
            }
            "boundary_unknown" => {
                row.write_boundary = ProviderCapabilityEvidence::Unknown;
                record
                    .supported_actions
                    .retain(|action| *action != SessionPolicyAction::CodingTargetWrite);
            }
            "boundary_denied" => {
                row.write_boundary = ProviderCapabilityEvidence::denied(
                    "t11 受控负向:write_boundary 分格携带真实负向证据",
                );
            }
            other => unreachable!("capability 分格场景分支外的场景 {other}"),
        }
        record.action_matrix.replace_row(row);
    }) {
        Ok(guard) => guard,
        Err(error) => return scenario_unobservable(format!("capability 突变失败:{error}")),
    };
    let verdict = gateway.validate(task11_coding_request(env));
    drop(guard);
    match verdict {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=capability error={error}"),
            );
            scenario_refused("validate(capability 分格门)", error.to_string())
        }
        Ok(_) => scenario_not_refused(
            "validate(capability 分格门)",
            format!("{scenario} 分格未被 capability 门拒绝"),
        ),
    }
}

/// 场景 9/10/14/17:version/wire 漂移、resume Unknown、D4 漂移——
/// spawn 前复验的真实 TOCTOU 拒绝。
async fn scenario_capability_drift(
    env: &MatrixEnvironment,
    sid: &str,
    scenario: &str,
) -> ScenarioOutcome {
    let dimension = match scenario {
        "version_drift" => "provider_version",
        "wire_drift" => "provider_dialect",
        "resume_unknown" => "resume 分格(ResumeNotSupported)",
        "d4_missing_or_drift" => "write_boundary(D4)分格复验",
        other => unreachable!("drift 场景分支外的场景 {other}"),
    };
    let mutation: Box<dyn FnOnce(&MatrixEnvironment) -> Result<Task11MutationGuard, String>> =
        match scenario {
            "version_drift" => Box::new(|env| {
                task11_mutate_selected_record(env, |record| {
                    record.version = format!("{}-t11-drift", record.version);
                })
                .map(Task11MutationGuard::Capability)
            }),
            "wire_drift" => Box::new(|env| {
                task11_mutate_selected_record(env, |record| {
                    let (dialect, wire) = match record.adapter_dialect {
                        ProviderDialect::ClaudeCodeCliV1 => (
                            ProviderDialect::CodexCliV1,
                            ProviderWireDialect::CodexAppServerRpc,
                        ),
                        ProviderDialect::CodexCliV1 => {
                            (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc)
                        }
                        ProviderDialect::PiRpcV1 => {
                            (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp)
                        }
                        ProviderDialect::KimiAcpV1 => (
                            ProviderDialect::ClaudeCodeCliV1,
                            ProviderWireDialect::ClaudeCodeStreamJson,
                        ),
                    };
                    record.adapter_dialect = dialect;
                    record.wire_dialect = wire;
                })
                .map(Task11MutationGuard::Capability)
            }),
            "resume_unknown" => Box::new(|env| {
                task11_mutate_selected_record(env, |record| {
                    let mut row = record
                        .action_matrix
                        .row(&SessionPolicyAction::CodingTargetWrite);
                    row.resume = ProviderCapabilityEvidence::Unknown;
                    record.action_matrix.replace_row(row);
                })
                .map(Task11MutationGuard::Capability)
            }),
            "d4_missing_or_drift" => Box::new(|env| {
                task11_mutate_selected_record(env, |record| {
                    let mut row = record
                        .action_matrix
                        .row(&SessionPolicyAction::CodingTargetWrite);
                    row.write_boundary = ProviderCapabilityEvidence::Unknown;
                    record.action_matrix.replace_row(row);
                    record
                        .supported_actions
                        .retain(|action| *action != SessionPolicyAction::CodingTargetWrite);
                })
                .map(Task11MutationGuard::Capability)
            }),
            other => unreachable!("drift 场景分支外的场景 {other}"),
        };
    // resume 场景:输入标记 resume(分格在 spawn 前复验施加)。
    let mut input = task11_streaming_input(env);
    if scenario == "resume_unknown" {
        input.resume_provider_session_id = Some("t11-resume-unknown-fixture-id".to_string());
    }
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let prepared = match task11_prepare_baseline(
        env,
        &gateway,
        sid,
        input,
        task11_coding_request(env),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    fp(
        "t11_scenario_mutation",
        format_args!("dimension={dimension}"),
    );
    let guard = match mutation(env) {
        Ok(guard) => guard,
        Err(problem) => return scenario_unobservable(format!("受控突变失败:{problem}")),
    };
    let cancel = CancellationToken::new();
    let started = gateway.start_streaming(prepared, cancel.clone()).await;
    drop(guard);
    match started {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("dimension={dimension} error={error}"),
            );
            scenario_refused(&format!("spawn 前复验({dimension})"), error.to_string())
        }
        Ok(session) => {
            drop(session);
            cancel.cancel();
            fp(
                "t11_scenario_unexpected_spawn",
                format_args!("dimension={dimension}(已 cancel+drop 收口)"),
            );
            scenario_not_refused(
                &format!("spawn 前复验({dimension})"),
                format!("维度 {dimension} 漂移未被复验拒绝"),
            )
        }
    }
}

/// 场景 11:codex_danger_full_access——danger 形态(非 target-only 写面)
/// 的 codex coding 启动被真实拒绝(validate 写面冻结/codex 投影 danger 门)。
async fn scenario_codex_danger(env: &MatrixEnvironment, sid: &str) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let codex_ref = ProviderRef::codex("cap_managed_snapshot");
    let guard = Task11CapabilityRestore::capture(env, codex_ref.provider_type);
    let record = Task11CapabilityRestore::fixture_record(codex_ref.provider_type);
    if let Err(error) = ProviderCapabilityStore::for_lc(env.app_paths.clone(), env.lc_id.clone())
        .upsert(PROJECT_ID, &record)
    {
        drop(guard);
        return scenario_unobservable(format!("codex 受控 setup 行写入失败:{error}"));
    }
    fp(
        "t11_scenario_mutation",
        "codex 受控 setup 行(Confirmed,只为通过 capability 门;被测面=danger 形态拒绝)",
    );
    // danger 形态:写面非 target-only(writable_roots 含 target 之外的 root)。
    let mut request = task11_coding_request(env);
    request.provider = codex_ref;
    request.writable_roots = vec![env.member_worktree.clone(), env.canonical_root.clone()];
    let mut input = task11_streaming_input(env);
    input.provider_type = ProviderType::Codex;
    let prepared = gateway.prepare_streaming_launch(input, request, task11_audit_context(env, sid));
    let outcome = match prepared {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=danger(validate) error={error}"),
            );
            scenario_refused("validate/prepare(danger 写面冻结)", error.to_string())
        }
        Ok(prepared) => {
            let cancel = CancellationToken::new();
            let started = gateway.start_streaming(prepared, cancel.clone()).await;
            match started {
                Err(error) => {
                    fp(
                        "t11_scenario_refused",
                        format_args!("gate=danger(投影/spawn 前) error={error}"),
                    );
                    scenario_refused("codex danger 门(投影/spawn 前复验)", error.to_string())
                }
                Ok(session) => {
                    drop(session);
                    cancel.cancel();
                    scenario_not_refused(
                        "codex danger 门(投影/spawn 前复验)",
                        "danger 形态未被拒绝".to_string(),
                    )
                }
            }
        }
    };
    drop(guard);
    outcome
}

/// 场景 12:illegal_role——Executor 携带外来 tool policy(非法组合)被
/// prepare 的 role/tool 策略 guard 真实拒绝。
fn scenario_illegal_role(env: &MatrixEnvironment, sid: &str) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let mut input = task11_streaming_input(env);
    input.role = AdapterRole::Executor;
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    fp(
        "t11_scenario_mutation",
        "Executor 携带外来 Some(tool policy)(非法组合)",
    );
    match gateway.prepare_streaming_launch(
        input,
        task11_coding_request(env),
        task11_audit_context(env, sid),
    ) {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=role_guard error={error}"),
            );
            scenario_refused("prepare(role/tool 策略 guard)", error.to_string())
        }
        Ok(_) => scenario_not_refused(
            "prepare(role/tool 策略 guard)",
            "非法 role/policy 组合未被 guard 拒绝".to_string(),
        ),
    }
}

/// 场景 13:false_flag——config artifact 非空 flag=false(envelope 校验
/// 拒绝;托管配置引用为 envelope 冻结的必填 flag)。
fn scenario_false_flag(env: &MatrixEnvironment) -> ScenarioOutcome {
    let gateway = match task11_readonly_gateway(env) {
        Ok(gateway) => gateway,
        Err(error) => return scenario_unobservable(format!("readonly gateway 组装失败:{error}")),
    };
    let mut request = task11_coding_request(env);
    request.config_artifact_ref = String::new();
    fp(
        "t11_scenario_mutation",
        "config_artifact_ref 置空(非空 flag=false)",
    );
    match gateway.validate(request) {
        Err(error) => {
            fp(
                "t11_scenario_refused",
                format_args!("gate=envelope_flag error={error}"),
            );
            scenario_refused(
                "validate(envelope config artifact 非空 flag)",
                error.to_string(),
            )
        }
        Ok(_) => scenario_not_refused(
            "validate(envelope config artifact 非空 flag)",
            "非空 flag=false 未被 envelope 校验拒绝".to_string(),
        ),
    }
}

/// 场景 15:git_refs_mutation_observed_not_identity_drift——prepare 后真实
/// commit 前移 HEAD(k3 终审澄清:产品指纹=canonical git-dir 路径,
/// provider_gateway.rs canonical_target_git_identity,HEAD/refs 突变不改
/// 该路径,复验不拒=设计必然且对正常 coding 自提交流程正确);本场景
/// 如实记录「复验未拒+spawn 后 cancel 收口」形态,不构成 identity 漂移
/// 负向证明(该判定面需 gitdir 路径突变才触发,受控复位)。
async fn scenario_fingerprint_drift(env: &MatrixEnvironment, sid: &str) -> ScenarioOutcome {
    let drift_file = env.member_worktree.join("t11-fingerprint-drift.txt");
    let head_before = git_head(&env.member_worktree);
    let mutation = |env: &MatrixEnvironment| -> Result<Task11MutationGuard, String> {
        if std::fs::write(&drift_file, b"t11 fingerprint drift fixture").is_err() {
            return Err(format!("写入 {} 失败", drift_file.display()));
        }
        let git = |arguments: &[&str]| -> std::io::Result<()> {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&env.member_worktree)
                .args(["-c", "user.email=t11@aria", "-c", "user.name=t11"])
                .args(arguments)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!("git {arguments:?} 失败")))
            }
        };
        git(&["add", "."]).map_err(|error| format!("git add 失败:{error}"))?;
        git(&[
            "commit",
            "-q",
            "-m",
            "t11 fingerprint drift(受控,场景后复位)",
        ])
        .map_err(|error| format!("git commit 失败:{error}"))?;
        fp(
            "t11_scenario_git_mutation",
            "target 真实 commit(受控:git add+commit;场景后复位)",
        );
        Ok(Task11MutationGuard::None)
    };
    let outcome = task11_revalidate_drift(
        env,
        sid,
        "resume_fingerprint(target git identity)",
        mutation,
    )
    .await;
    // 受控复位:硬复位回漂移前 HEAD(仅本场景提交;fixture 仓)。
    if let (Some(head_before), Some(head_after)) = (head_before, git_head(&env.member_worktree)) {
        if head_before != head_after {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&env.member_worktree)
                .args(["reset", "--hard", "-q", &head_before])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status();
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(&env.member_worktree)
                .args(["clean", "-fdq"])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status();
            fp(
                "t11_git_restored",
                format_args!("reset → {head_before}(fingerprint 漂移复位)"),
            );
        }
    }
    outcome
}

/// 场景 16:target_git_pointer_missing_or_drift——prepare 后 target 的
/// `.git`(目录或 linked-worktree 指针)被篡改 → resolver 重解析失败或
/// 不一致(零 spawn;受控恢复)。
async fn scenario_target_git_pointer(env: &MatrixEnvironment, sid: &str) -> ScenarioOutcome {
    let git_path = env.member_worktree.join(".git");
    let pointer_form = std::fs::symlink_metadata(&git_path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false);
    let original_pointer = std::fs::read_to_string(&git_path).ok();
    let mutation = |env: &MatrixEnvironment| -> Result<Task11MutationGuard, String> {
        if pointer_form {
            if std::fs::write(&git_path, b"gitdir: /tmp/t11-drift-pointer").is_err() {
                return Err("改写 .git 指针失败".to_string());
            }
            // 指针形态:恢复信息由场景尾部显式处理(守卫闭包无法跨
            // start_streaming 存活),原字节随事件留证。
            fp(
                "t11_scenario_pointer_tampered",
                format!(".git 指针改写(原内容:{original_pointer:?})"),
            );
        } else {
            let moved = env.member_worktree.join(".git.t11-tamper");
            if std::fs::rename(&git_path, &moved).is_err() {
                return Err("移走 .git 目录失败".to_string());
            }
            fp("t11_scenario_pointer_tampered", ".git 目录移走(受控)");
        }
        Ok(Task11MutationGuard::None)
    };
    let outcome =
        task11_revalidate_drift(env, sid, "target/git identity(resolver 重解析)", mutation).await;
    // 受控恢复:目录形态移回;指针形态按篡改前字节写回(原字节在闭包
    // fp 已留证;此处从 tamper 语义恢复为「gitdir 指回真实 git dir」——
    // 目录形态 git dir 未动,指针重写为原内容)。
    if pointer_form {
        if let Some(original) = original_pointer.clone() {
            let _ = std::fs::write(&git_path, original);
            fp("t11_git_restored", ".git 指针写回(target pointer 场景)");
        }
    } else {
        let moved = env.member_worktree.join(".git.t11-tamper");
        if moved.exists() && !git_path.exists() {
            let _ = std::fs::rename(&moved, &git_path);
            fp("t11_git_restored", ".git 目录移回(target pointer 场景)");
        }
    }
    outcome
}

/// availability `--version` 真实探测(另记维度,不冒充 session child)。
fn task11_availability_probe(env: &MatrixEnvironment) -> Option<Value> {
    let (cli_program, _) = provider_probe_channel(&env.provider)?;
    let output = std::process::Command::new(cli_program)
        .arg("--version")
        .output()
        .ok()?;
    let excerpt: String = String::from_utf8_lossy(&output.stdout)
        .trim()
        .chars()
        .take(160)
        .collect();
    fp(
        "t11_availability_version_probe",
        format_args!(
            "argv=[{cli_program} --version] exit={:?} excerpt={excerpt:?}",
            output.status.code()
        ),
    );
    Some(json!({
        "ts": task11_now_ts(),
        "event": {
            "type": "availability_version_probe",
            "argv": [cli_program, "--version"],
            "exit": output.status.code(),
            "output_excerpt": excerpt,
        },
    }))
}

/// 失败段驱动:逐场景真实探测(逐格失败不中止矩阵;不删格)。
async fn run_task11_failure_scenarios(env: &mut MatrixEnvironment) -> Vec<EvidenceCell> {
    let availability_event = task11_availability_probe(env);
    let mut cells = Vec::new();
    for (index, scenario) in TASK11_FAILURE_SCENARIOS.iter().enumerate() {
        let phase = if matches!(*scenario, "resume_unknown" | "fingerprint_drift") {
            RESUME
        } else {
            FRESH
        };
        fp_enter_phase("t11-failure", scenario, phase);
        fp("t11_scenario_begin", format_args!("scenario={scenario}"));
        let sid = format!("t11-failure-{scenario}");
        // 零 spawn 会计三面之一:场景前后审计扫描(独立会话分区)。
        let audits_before = env.scan_session_audits(&sid).len();
        let outcome = run_task11_scenario(env, scenario, &sid).await;
        let audits_after = env.scan_session_audits(&sid).len();
        fp(
            "t11_scenario_end",
            format_args!(
                "scenario={scenario} refused={} state={} audits_before={audits_before} audits_after={audits_after}",
                outcome.refused, outcome.state
            ),
        );
        fp(
            "t11_zero_spawn_accounting",
            format_args!(
                "scenario={scenario} provider_start_audits={audits_after} pid=none argv=none"
            ),
        );
        let mut events = vec![
            json!({
                "ts": task11_now_ts(),
                "event": {
                    "type": "boundary_matrix_evidence",
                    "failure_scenario": scenario,
                },
            }),
            json!({
                "ts": task11_now_ts(),
                "event": {
                    "type": "launch_rejected_probe",
                    "scenario": scenario,
                    "refusal_gate": outcome.refusal_gate,
                    "refusal_text": outcome.refusal_text,
                    "refused": outcome.refused,
                },
            }),
            json!({
                "ts": task11_now_ts(),
                "event": {
                    "type": "zero_spawn_observation",
                    "provider_start_audits_before": audits_before,
                    "provider_start_audits_after": audits_after,
                    "pid_observed": false,
                    "argv_captured": false,
                    "scan_window": "workspace session 分区 role_run_seq 0..=64",
                },
            }),
        ];
        if index == 0 {
            if let Some(event) = availability_event.clone() {
                events.push(event);
            }
        }
        events.extend(outcome.extra_events);
        let mut cell = task11_cell_base(
            env,
            format!(
                "t11-failure-{scenario}-{}",
                Utc::now().format("%Y%m%dT%H%M%SZ")
            ),
        );
        cell.fresh_or_resume = phase.to_string();
        cell.capability_state = outcome.state.to_string();
        cell.denied_reason = Some(outcome.refusal_text.clone());
        cell.workspace_session_id = sid;
        cell.pid_unavailable_reason = Some(format!(
            "零 spawn 被拒:无 provider 子进程(provider_start audits={audits_after},\
             PID 无观测,argv 无捕获)"
        ));
        cell.provider_events = events;
        cells.push(cell);
    }
    cells
}

/// 单场景分发(全部真实产品面;不可观察=Unknown 格,不冒充被拒)。
async fn run_task11_scenario(
    env: &mut MatrixEnvironment,
    scenario: &str,
    sid: &str,
) -> ScenarioOutcome {
    match scenario {
        "missing_readiness" => scenario_missing_readiness(env, sid).await,
        "missing_body" => scenario_missing_body(env),
        "missing_receipt" => scenario_missing_receipt(env),
        "missing_trust" => scenario_missing_trust(env, sid).await,
        "launch_unknown" | "launch_denied" | "boundary_unknown" | "boundary_denied" => {
            scenario_capability_cell(env, scenario)
        }
        "version_drift" | "wire_drift" | "resume_unknown" | "d4_missing_or_drift" => {
            scenario_capability_drift(env, sid, scenario).await
        }
        "codex_danger_full_access" => scenario_codex_danger(env, sid).await,
        "illegal_role" => scenario_illegal_role(env, sid),
        "false_flag" => scenario_false_flag(env),
        "fingerprint_drift" => scenario_fingerprint_drift(env, sid).await,
        "target_git_pointer_missing_or_drift" => scenario_target_git_pointer(env, sid).await,
        other => scenario_unobservable(format!("未知场景 {other}(清单漂移,须对齐冻结测试面)")),
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
    /// r55:成员仓本地 bare origin 根(每轮 tempdir;持活至 env drop——
    /// coding 交付链 push/ls-remote 需远端在本轮内存续)。
    _bare_origin_root: Option<TempDir>,
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
    /// r52 快照:工作区根固定路径(capture/resume 模式;None=tempdir)。
    workspace_root_path: PathBuf,
    /// r52 快照:聚合根固定路径(capture/resume 模式;None=tempdir)。
    aggregate_root_fixed: Option<PathBuf>,
    /// r52 快照:capture 轮在 plan-confirmed 边界捕获的快照 id。
    snapshot_id: Option<String>,
    /// r52 快照:capture 轮 build 预建的快照目录(run/ 所在;pristine
    /// 落同目录)。
    capture_snapshot_dir: Option<PathBuf>,
    /// r52 快照:resume 轮打开的 manifest(承继格引用)。
    snapshot_manifest: Option<super::snapshot::SnapshotManifest>,
    /// r52 快照:resume 轮=true——coding/review 观测格标 snapshot_boot。
    snapshot_boot: bool,
}

impl MatrixEnvironment {
    /// r52 快照续跑:打开快照(pristine 校验+diff 门禁+再基线计数→
    /// 回灌 run/ 固定路径原地打开),重建全部短生命期状态(WebAppState/
    /// registry/gate/EventHub/run registries 全新;provider health 真实
    /// 重探测),durable preflight(manifest 摘要复核)由 open 完成。
    /// 不做路径重写——run/ 与捕获轮逐字节同路径,指纹自然有效。
    async fn build_from_snapshot(
        provider: ProviderName,
        evidence_root: &Path,
    ) -> Result<Self, LiveMatrixFailure> {
        let snapshot_id = super::snapshot::resolve_snapshot_id(evidence_root)
            .map_err(|message| matrix_failure("snapshot_unresolvable", message, None))?;
        fp(
            "snapshot_open_begin",
            format_args!("snapshot={snapshot_id}"),
        );
        let opened = super::snapshot::open_plan_snapshot(evidence_root, &snapshot_id)
            .map_err(|error| matrix_failure(error.reason_code(), error.message(), None))?;
        let manifest = opened.manifest.clone();
        fp(
            "snapshot_opened",
            format_args!(
                "snapshot={snapshot_id} revision={} created_at={}",
                manifest.harness_revision, manifest.created_at
            ),
        );
        let root = opened.workspace_root;
        let aggregate_root = opened.aggregate_root;
        let runtime = WebRuntime::new_real(root.clone()).map_err(|error| {
            matrix_failure("real_runtime_unavailable", format!("{error:?}"), None)
        })?;
        let state = WebAppState::with_events(root.clone(), runtime, EventHub::new());
        if state.test_provider_enabled {
            return Err(matrix_failure(
                "real_mode_required",
                "ARIA_PROVIDER_MODE=fake 或 test provider 生效:真实矩阵要求真实 provider registry"
                    .to_string(),
                None,
            ));
        }
        let gateway_factory = state
            .logical_gateway_factory
            .clone()
            .expect("生产 logical gateway factory");
        let app_paths = ProductAppPaths::new(root.join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let app = build_web_router(state);
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
            provider: provider.clone(),
            provider_wire,
            _root: None,
            _aggregate_root: None,
            _bare_origin_root: None,
            app: http_app,
            _server: server,
            ws_addr,
            app_paths,
            lifecycle,
            gateway_factory,
            // durable 身份全部来自 manifest(快照点冻结)。
            lc_id: manifest.lc_id.clone(),
            issue_id: manifest.issue_id.clone(),
            canonical_root: PathBuf::new(),
            member_worktree: PathBuf::new(),
            member_physical_repo_id: String::new(),
            member_logical_id: String::new(),
            member_checkout_id: String::new(),
            story_spec_id: None,
            design_spec_id: None,
            work_item_id: Some(manifest.work_item_id.clone()),
            plan_work_item_ids: vec![manifest.work_item_id.clone()],
            plan_id: manifest.plan_id.clone(),
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
            workspace_root_path: root,
            aggregate_root_fixed: Some(aggregate_root),
            snapshot_id: Some(snapshot_id),
            capture_snapshot_dir: None,
            snapshot_manifest: Some(manifest),
            snapshot_boot: true,
        };
        // provider health 真实重探测(不沿用快照;有界超时=BLOCKED 报告)。
        // F2(r58 深掏审计):就绪返回的当前 CLI 版本与捕获时冻结的
        // manifest.cli_version 前置比对——CLI 自动更新会使快照携带的
        // capability 行/投影指纹全失效,coding/review spawn 八维复验必
        // PolicyDrift 拒启(两格白烧 45-75min 且表象像新缺陷);漂移早
        // BLOCKED 明确指示重 capture,同 token 放行(基线快照兼容)。
        let current_cli_version = env.wait_for_provider_health().await?;
        let frozen_cli_version = env
            .snapshot_manifest
            .as_ref()
            .expect("snapshot manifest")
            .cli_version
            .clone();
        if let Some(drift_reason) =
            snapshot_cli_version_drift(&frozen_cli_version, current_cli_version.as_deref())
        {
            fp(
                "snapshot_cli_version_drifted",
                format_args!(
                    "frozen={frozen_cli_version:?} current={:?}",
                    current_cli_version.as_deref().unwrap_or("-")
                ),
            );
            return Err(matrix_failure(
                "snapshot_cli_version_drifted",
                drift_reason,
                None,
            ));
        }
        // F4(r58 深掏审计):resume 轮与 build() 同源幂等重登 provider
        // trust——codex/kimi 的用户级 trust 工件只在 build() 写 HOME
        //(快照不含 HOME 面);HOME 清理/容器重建/换机跑 resume 会撞
        // trust 硬前置门(r3 三家「entry absent」同款)。ensure_before_
        // recipe 幂等:canonical root=固定快照路径,键稳定,工件在场
        // =Replay 重写安全,缺工件=重新登记。
        env.ensure_provider_trust().await?;
        // canonical root/target 从 durable manifest 复原(原地同路径)。
        env.resolve_member_target().await?;
        // 成员 Git 身份 preflight:快照点 HEAD 复核(漂移=BLOCKED)。
        let member_head = super::snapshot::git_head(&env.member_worktree)
            .map_err(|message| matrix_failure("snapshot_member_head_unavailable", message, None))?;
        let expected_head = env
            .snapshot_manifest
            .as_ref()
            .expect("snapshot manifest")
            .member_git_head
            .clone();
        if !expected_head.is_empty() && member_head != expected_head {
            return Err(matrix_failure(
                "snapshot_member_head_mismatch",
                format!("成员仓 HEAD {member_head} != 快照点 {expected_head}"),
                None,
            ));
        }
        // r55 修复:resume 轮 env 准备收尾——回灌后的 run 树成员仓补齐
        // 本地 bare origin(coding 交付链 push/ls-remote 的 remote=origin
        // 契约)。置于 pristine 校验/diff 门禁/成员 HEAD 复核全部通过之后:
        // 只动 run 树 `.git` 配面(pristine 不触;tree_digest 跳过 .git,
        // 后续轮 digest/HEAD 复核不受影响);pristine 可能携带 capture
        // 轮陈旧 origin URL,ensure 以 set-url 收敛到本轮 bare(tempdir
        // 随 env 持活,轮内 push 可达)。承继格(story/design/plan/split)
        // 不涉 push,不受影响。
        let bare_origin_root = TempDir::new().expect("bare origin root");
        for member in ["alpha", "beta"] {
            ensure_member_bare_origin(
                bare_origin_root.path(),
                &env.aggregate_root_path().join(member),
            );
        }
        env._bare_origin_root = Some(bare_origin_root);
        fp(
            "member_bare_origin_ensured",
            format_args!(
                "snapshot={} bare_root={}",
                env.snapshot_id.as_deref().unwrap_or("-"),
                env._bare_origin_root
                    .as_ref()
                    .expect("bare origin root")
                    .path()
                    .display()
            ),
        );
        // r56 修复:resume 轮 spec id 恢复(与 r55 origin ensure 同区挂点:
        // 全部 preflight 通过后、Ok(env) 前)。story/design 为承继格不重跑,
        // manifest schema 冻结不携带 spec id——回灌后的 run 树 durable 态
        // (.aria 内 plan_confirmed 前已 Confirmed 的 spec 记录)经现成只读
        // 端点 GET issue lifecycle 重取(优先产品 API:零 .aria 直读、与
        // harness 全程 HTTP 交互同风格;端点自带 legacy 版本 backfill 幂等,
        // 只写 run 树不触 pristine/snapshots)。review 阶段建载体会话消费
        // story_spec_ids,缺失=产品 500 story_spec_required(r54-r56 三轮
        // 同错)→取不到 fail loudly(snapshot_resume_state_missing,
        // BLOCKED 真实报告),绝不静默空。
        let (status, body) = request_json(
            &env.app,
            Method::GET,
            &format!(
                "/api/issues/{}/lifecycle?project_id={PROJECT_ID}",
                env.issue_id
            ),
            json!({}),
        )
        .await;
        if !status.is_success() {
            return Err(matrix_failure(
                "snapshot_resume_state_missing",
                format!("快照续跑恢复失败:issue lifecycle 读取失败({status}):{body}"),
                None,
            ));
        }
        let (story_spec_id, design_spec_id) = match restored_spec_ids_from_lifecycle(&body) {
            Ok(ids) => ids,
            Err(reason) => {
                return Err(matrix_failure(
                    "snapshot_resume_state_missing",
                    format!("快照续跑恢复失败:回灌后 durable 态缺 Confirmed spec——{reason}"),
                    None,
                ));
            }
        };
        env.story_spec_id = Some(story_spec_id.clone());
        env.design_spec_id = Some(design_spec_id.clone());
        fp(
            "snapshot_spec_ids_restored",
            format_args!("story={story_spec_id} design={design_spec_id}"),
        );
        Ok(env)
    }

    async fn build(
        provider: ProviderName,
        evidence_root: &Path,
    ) -> Result<Self, LiveMatrixFailure> {
        // 1) 真实模式门:fake registry/test provider 下不允许冒充真实现场。
        // r52 快照:capture 模式在固定路径 run/{workspace,aggregate-root}
        // 上构建(resume 轮回灌 pristine→run/ 后路径与捕获轮逐字节同源,
        // 绝对路径嵌入的指纹自然有效;不做路径重写)。
        let capture_mode = matches!(
            super::snapshot::RunMode::from_env(),
            Ok(super::snapshot::RunMode::CapturePlanSnapshot)
        );
        let mut capture_snapshot_dir: Option<PathBuf> = None;
        let (root, aggregate_root, workspace_root_path, aggregate_root_fixed) = if capture_mode {
            let snapshot_id = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            let directory = super::snapshot::snapshots_root(evidence_root).join(&snapshot_id);
            let run_workspace = directory.join("run").join("workspace");
            let run_aggregate = directory.join("run").join("aggregate-root");
            std::fs::create_dir_all(&run_workspace).map_err(|error| {
                matrix_failure(
                    "snapshot_run_dir_create_failed",
                    format!("{}: {error}", run_workspace.display()),
                    None,
                )
            })?;
            std::fs::create_dir_all(&run_aggregate).map_err(|error| {
                matrix_failure(
                    "snapshot_run_dir_create_failed",
                    format!("{}: {error}", run_aggregate.display()),
                    None,
                )
            })?;
            // r53 修复:固定路径 canonicalize——evidence_root 可为相对路径,
            // 而登记 preflight 的 auto-discovery 以 canonical 绝对路径产出
            // candidates;原始相对路径的 confirmed_paths 与其交集为空→
            // 「confirmed preflight must contain at least one candidate」500
            //(tempdir 轮天然绝对路径故不现)。canonicalize 后与发现面同源。
            let run_workspace = run_workspace.canonicalize().map_err(|error| {
                matrix_failure(
                    "snapshot_run_dir_create_failed",
                    format!("canonicalize {}: {error}", run_workspace.display()),
                    None,
                )
            })?;
            let run_aggregate = run_aggregate.canonicalize().map_err(|error| {
                matrix_failure(
                    "snapshot_run_dir_create_failed",
                    format!("canonicalize {}: {error}", run_aggregate.display()),
                    None,
                )
            })?;
            fp(
                "snapshot_capture_run_root",
                format_args!("snapshot={snapshot_id} dir={}", directory.display()),
            );
            capture_snapshot_dir = Some(directory.clone());
            (None, None, run_workspace, Some(run_aggregate))
        } else {
            let root = TempDir::new().expect("workspace root");
            let aggregate_root = TempDir::new().expect("aggregate root");
            let workspace_root_path = root.path().to_path_buf();
            (Some(root), Some(aggregate_root), workspace_root_path, None)
        };
        let runtime = WebRuntime::new_real(workspace_root_path.clone()).map_err(|error| {
            matrix_failure("real_runtime_unavailable", format!("{error:?}"), None)
        })?;
        let state = WebAppState::with_events(workspace_root_path.clone(), runtime, EventHub::new());
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
        let aggregate_root_path = aggregate_root_fixed.clone().unwrap_or_else(|| {
            aggregate_root
                .as_ref()
                .expect("aggregate root")
                .path()
                .to_path_buf()
        });
        let member_a = aggregate_root_path.join("alpha");
        let member_b = aggregate_root_path.join("beta");
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

        // r55 修复:建仓即配本地 bare origin——coding 交付链(commit→
        // push→ls-remote)按真实部署假设消费 remote=origin;夹具此前
        // git init 无 remote,r55 coder 守纪律走完 commit→push 后
        // ls-remote fatal。bare 落 harness 自有 tempdir(不进 run/
        // pristine 树);capture 轮 pristine 会携带本 tempdir URL,
        // resume 轮 ensure 以 set-url 收敛(幂等)。
        let bare_origin_root = TempDir::new().expect("bare origin root");
        ensure_member_bare_origin(bare_origin_root.path(), &member_a);
        ensure_member_bare_origin(bare_origin_root.path(), &member_b);
        fp(
            "member_bare_origin_ensured",
            format_args!("bare_root={}", bare_origin_root.path().display()),
        );

        let app_paths = ProductAppPaths::new(workspace_root_path.join(".aria"));
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
            _root: root,
            _aggregate_root: aggregate_root,
            _bare_origin_root: Some(bare_origin_root),
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
            workspace_root_path,
            aggregate_root_fixed,
            snapshot_id: None,
            capture_snapshot_dir,
            snapshot_manifest: None,
            snapshot_boot: false,
        };

        // 2.5) provider health 就绪等待:new_real 的 ProviderHealthService
        // 需完成探测刷新,否则 spawn 复验报 provider_gateway_unavailable
        //(health state degraded)。有界超时,超时=BLOCKED 真实报告。
        // 全链/capture 轮不做版本比对(无快照基线;版本供 resume 轮用)。
        let _ = env.wait_for_provider_health().await?;
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
        //      「provider trust not established for Codex: entry absent」
        //      ——聚合初始化的 trust 硬前置门按 recipe 固定只评估
        //      ClaudeCode(自身无用户级 trust 面,requires_workspace_trust
        //      过滤后恒空),codex/kimi 的 workspace trust 条目无登记入口。
        //      环境构造与生产 production_provider_trust_precondition 同源
        //      构造 home 背书 registry,把 trust 面 provider 登记为 Ready
        //      (claude/pi 无 trust 工件,registry 内部过滤跳过)。
        env.ensure_provider_trust().await?;
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
                format!(
                    "provider {:?} 无真实探针通道(四家之外不冒充)",
                    self.provider
                ),
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
            // r50(#1,coding fresh 现场):code review reviewer 角色经
            // ReviewReadOnly 启动——review round 2 的 reviewer 原生 resume
            // 被 require_resume_supported 拒(provider_gateway_resume_not_
            // supported:探针未覆盖该 action,resume 分格 Unknown)。探针
            // 与 PlanningReadOnly 同族(只读+Reviewer 角色),补齐后 reviewer
            // resume=Confirmed,round 2 正常续接。
            SessionPolicyAction::ReviewReadOnly,
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
            let resume_state =
                ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref());
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
                        format!(
                            "provider {:?} action {:?} 2d 导入被拒:{error:?}",
                            self.provider, action
                        ),
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
    /// r4:trust 面 provider 的用户级 workspace trust 登记;MaxParallel3
    /// 收窄为仅 selected provider(registry 内 `requires_workspace_trust`
    /// 过滤,pi/claude 天然零写,另一家条目不由本家进程登记)——三家
    /// 并行时 HOME trust 文件每家唯一写者,消除跨进程 RMW TOCTOU。
    /// canonical root 与 gateway authority root 同源(manifest
    /// provider_context_root 的 canonicalize,见 gateway_factory
    /// build_scoped);HOME 与生产 trust 装配同源。Waiting=可重试等待面,
    /// 失败即 BLOCKED 真实报告,绝不伪造 Ready。
    async fn ensure_provider_trust(&mut self) -> Result<(), LiveMatrixFailure> {
        fp_enter_phase("env", "provider_trust_ensure", "-");
        let Some(home) = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .filter(|home| home.is_absolute())
        else {
            let failure = matrix_failure(
                "provider_trust_home_unavailable",
                "HOME/USERPROFILE 缺失:无法构造 home 背书 trust registry".to_string(),
                None,
            );
            return Err(self.fail_with_diagnostics(failure, None).await);
        };
        let store = LogicalCodebaseStore::for_lc(self.app_paths.clone(), self.lc_id.clone());
        let manifest = match store.load_manifest(PROJECT_ID) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => {
                let failure = matrix_failure(
                    "provider_trust_manifest_missing",
                    "LC manifest 缺失:无法定位 provider_context_root".to_string(),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
            Err(error) => {
                let failure = matrix_failure(
                    "provider_trust_manifest_error",
                    format!("读取 LC manifest 失败:{error}"),
                    None,
                );
                return Err(self.fail_with_diagnostics(failure, None).await);
            }
        };
        let canonical_root = std::fs::canonicalize(&manifest.provider_context_root)
            .unwrap_or_else(|_| manifest.provider_context_root.clone());
        let registry = HomeBackedProviderTrustRegistry::new(
            self.app_paths.clone(),
            vec![
                std::sync::Arc::new(CodexTrustAdapter::for_home(&home)),
                std::sync::Arc::new(KimiTrustAdapter::for_home(&home)),
            ],
        );
        let operation_id = format!("lcg-matrix-trust-{}", self.lc_id);
        match registry.ensure_before_recipe(
            PROJECT_ID,
            &operation_id,
            &self.lc_id,
            &canonical_root,
            selected_trust_targets(&self.provider),
        ) {
            cadence_aria::product::logical_codebase::ProviderTrustPreparationResult::Ready {
                registrations,
            } => {
                let summary = registrations
                    .iter()
                    .map(|registration| {
                        format!("{:?}={:?}", registration.provider, registration.result)
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                fp("provider_trust_ready", summary);
                Ok(())
            }
            cadence_aria::product::logical_codebase::ProviderTrustPreparationResult::Waiting {
                waiting,
            } => {
                let failure = matrix_failure(
                    "provider_trust_waiting",
                    format!(
                        "trust 登记未 Ready(provider={:?} reason={} message={};环境不可运行)",
                        waiting.provider, waiting.reason_code, waiting.message
                    ),
                    None,
                );
                Err(self.fail_with_diagnostics(failure, None).await)
            }
        }
    }

    fn workspace_root_path(&self) -> &Path {
        if !self.workspace_root_path.as_os_str().is_empty() {
            return &self.workspace_root_path;
        }
        self._root.as_ref().expect("workspace root").path()
    }

    fn aggregate_root_path(&self) -> &Path {
        self.aggregate_root_fixed.as_deref().unwrap_or_else(|| {
            self._aggregate_root
                .as_ref()
                .expect("aggregate root")
                .path()
        })
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
    /// F2:就绪时返回该 provider 的当前 CLI 版本(与 recheck 的 --version
    /// 探测同源,经 /api/providers/status 透出),供 resume 轮与快照
    /// manifest.cli_version 前置比对。
    async fn wait_for_provider_health(&mut self) -> Result<Option<String>, LiveMatrixFailure> {
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
                let current_version = body["providers"].as_array().and_then(|entries| {
                    entries
                        .iter()
                        .find(|entry| entry["provider"] == self.provider_wire)
                        .and_then(|entry| entry["version"].as_str().map(str::to_string))
                });
                return Ok(current_version);
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
    /// 真实聚合初始化(固定 Claude recipe):对概率性失败(receipt
    /// Rejected/CLI 写错 scope,产品标 retryable)按新产品 operation
    /// 有界重试——每轮失败先抄录该轮 receipts 到 diagnostics/attempt-N/,
    /// 拒绝轮证据全留;最终失败才保留 tempdir。
    ///
    /// r57:失败 attempt 可能把 root recipe 工件写进聚合根
    /// (AGENTS.md/CLAUDE.md/.claude/…),下一次新 operation 的严格
    /// preflight 会按 reject_owned_root_files 永拒(recipe replay 豁免
    /// 只认 Completed operation 的 receipt);CLI 也可能把成员仓写脏
    /// (r57 attempt1 把整套单仓布局写进 alpha/beta,114 个 DENY)。所以
    /// 首次 attempt 前冻结根级 pristine 条目名,失败之后、下一次 attempt
    /// 之前按 [`reset_aggregate_root_for_retry`] 复位。只有
    /// `initialization_failed`(operation 已 durable 终态、worker 已退
    /// 出)才会进入下一轮,超时轮(worker 可能仍活着)不复位不重试。
    async fn run_real_aggregate_initialization(&mut self) -> Result<(), LiveMatrixFailure> {
        fp_enter_phase("env", "aggregate_init", "-");
        const MAX_ATTEMPTS: usize = 3;
        let pristine_root_entries: Vec<String> = std::fs::read_dir(self.aggregate_root_path())
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        let mut last_failure = None;
        for attempt in 1..=MAX_ATTEMPTS {
            if attempt > 1 {
                reset_aggregate_root_for_retry(self.aggregate_root_path(), &pristine_root_entries);
            }
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
                        fp(
                            "init_poll",
                            format_args!("status={} 继续轮询(30s 心跳)", snapshot["status"]),
                        );
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
                fp(
                    "index_poll",
                    format_args!("state={} 继续轮询(30s 心跳)", body["state"]),
                );
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
                observation.denied_cell(format!("生成 {stage} 响应缺 workspace_session:{body}")),
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
            return observation
                .denied_cell("fresh 轮未达人工门(无干净的同连接会话可供门上修订)".to_string());
        };
        observation.workspace_session_id = session_id.clone();
        fp(
            "resume_audit_scan_begin",
            format_args!("session={session_id}"),
        );
        // 请求恢复的 native id = fresh 轮审计里的 provider_session_id
        //(必须在 revision 重驱前捕获)。
        observation.requested_resume_id =
            self.latest_audit_native_id(&session_id, &self.provider, None);
        observation.frozen_digest =
            self.latest_audit_projection_digest(&session_id, &self.provider);
        fp(
            "resume_audit_scan_end",
            format_args!(
                "requested_id={:?} frozen={:?}",
                observation.requested_resume_id, observation.frozen_digest
            ),
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
        self.drive_workspace_session_ws_with_reviewer(
            session_id,
            observation,
            confirm_rounds,
            timeout,
            None,
        )
        .await
    }

    /// r45 根修:reviewer 可配——plan 会话的 reviewer 经 start_generation
    /// provider_config 透传(r44 现场:plan fresh 发 reviewer:null 覆盖
    /// prepare 所设 reviewer→SC compile 子会话 reviewer None→coding 组
    /// attempt provider 快照 code_reviewer=null→code_review 门 blocked
    /// reviewer_configuration_missing,重试动作救不回)。实体阶段
    /// (story/design)保持 None 语义不变。
    async fn drive_workspace_session_ws_with_reviewer(
        &self,
        session_id: &str,
        observation: &mut StageObservation,
        confirm_rounds: usize,
        timeout: Duration,
        reviewer: Option<ProviderName>,
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
                "reviewer": reviewer,
                "review_rounds": 1
            },
            "reviewer_enabled": reviewer.is_some()
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
        fp(
            "pump_begin",
            format_args!("timeout={timeout:?} confirms={confirm_rounds}"),
        );
        loop {
            if tokio::time::Instant::now() >= deadline {
                // r21 C1 同族盲区:该分支此前不落 run_failure——story resume
                // 空转 5400s 后仍以"通过"建格,正是 r21 误判「story resume 过」
                // 的另一半原因。阶段超时=驱动未达终态,必须落格失败原因。
                observation.push_event(json!({"type": "matrix_stage_timeout"}));
                observation.run_failure =
                    Some("workspace 会话阶段超时未达终态(对端零响应或挂死)".to_string());
                fp(
                    "pump_stage_timeout",
                    format_args!("elapsed={:?} events={events_seen}", pump_started.elapsed()),
                );
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
                    fp(
                        "pump_ws_closed",
                        format_args!("elapsed={:?} events={events_seen}", pump_started.elapsed()),
                    );
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
                        format_args!(
                            "elapsed={:?} events={events_seen}(30s 心跳:泵存活/对端静默)",
                            pump_started.elapsed()
                        ),
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
                    fp(
                        "pump_stage_timeout",
                        format_args!("elapsed={:?} events={events_seen}", pump_started.elapsed()),
                    );
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
                        fp(
                            "pump_session_state",
                            format_args!("status={status} events={events_seen}"),
                        );
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
                        fp(
                            "pump_choice_request",
                            format_args!("id={choice_id} options={}", top_selected.join(",")),
                        );
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
                    fp(
                        "pump_confirmed",
                        format_args!("elapsed={:?}", pump_started.elapsed()),
                    );
                    return outcome;
                }
                PumpFrameDisposition::Terminal(status) => {
                    observation.terminal_status = Some(status.to_string());
                    fp(
                        "pump_terminal",
                        format_args!("status={status} elapsed={:?}", pump_started.elapsed()),
                    );
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
            .drive_workspace_session_ws_with_reviewer(
                &session_id,
                &mut observation,
                0,
                self.entity_stage_timeout,
                // r45:plan 会话 reviewer 经 start_generation 透传(SC
                // compile 子会话继承→coding 组快照 code_reviewer 非空;
                // r44 现场 null→code_review 门 blocked 不可达)。
                Some(self.provider.clone()),
            )
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

        // ---- resume:SC 门 typed feedback 修订 → 复评/修复循环回门(门帧
        // 驱动,r58)→ typed confirm 定稿(work item 落库),同连接(与实体
        // 阶段同模型)----
        let resume = self.drive_plan_sc_gate_resume(&session_id, gate_ws).await;
        cells.push(resume);
        // Plan confirmed 后解析 work item(coding 前置;r26:从 resume
        // confirm 前移出——fresh 不再 confirm,work item 定稿在 resume 轮)。
        self.resolve_first_work_item().await;
        // r52 快照:capture 模式在此静止边界落快照(typed confirm 已
        // confirmed、work item 已解析、无 running run/attempt)。
        if matches!(
            super::snapshot::RunMode::from_env(),
            Ok(super::snapshot::RunMode::CapturePlanSnapshot)
        ) && self.snapshot_id.is_none()
        {
            let lc_root = self
                .app_paths
                .logical_codebase_record_root(PROJECT_ID, &self.lc_id);
            let audits =
                self.scan_session_audits(&self.prior_plan_session_id.clone().unwrap_or_default());
            let cli_version = audits
                .first()
                .map(|(_, record)| record.provider_version.clone())
                .unwrap_or_default();
            let input = super::snapshot::CaptureInput {
                provider_wire: &self.provider_wire,
                cli_version: &cli_version,
                project_id: PROJECT_ID,
                issue_id: &self.issue_id,
                lc_id: &self.lc_id,
                plan_id: &self.plan_id,
                work_item_id: self.work_item_id.as_deref().unwrap_or_default(),
                workspace_root: self.workspace_root_path(),
                aggregate_root: self.aggregate_root_path(),
                lc_root: &lc_root,
                policy_root: &self.canonical_root,
                source_evidence_root: &self.evidence_root,
            };
            match super::snapshot::capture_plan_snapshot(
                &self.evidence_root,
                &input,
                self.capture_snapshot_dir.as_deref(),
            ) {
                Ok(captured) => {
                    self.snapshot_id = Some(captured.snapshot_id.clone());
                    fp(
                        "snapshot_captured",
                        format_args!(
                            "snapshot={} dir={}",
                            captured.snapshot_id,
                            captured.directory.display()
                        ),
                    );
                }
                Err(message) => {
                    // 快照失败不伪造、不阻断全链证据:fp 留痕,本轮无快照
                    //(capture 是运营加速面,矩阵证据语义不受影响)。
                    fp("snapshot_capture_failed", format_args!("{message}"));
                }
            }
        }
        cells
    }

    /// r26 问题2:SC plan 会话的门上修订重驱。SC human_confirm 门的消息集是
    /// typed 三命令:request_revision 不放行(r25 现场
    /// WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID)。修订入口 =
    /// `human_gate_feedback`(开门 turn→HumanGateScManualRevision run→
    /// `human_gate_turn_completed` 回门→r58 起复评/修复循环后门重开),
    /// 定稿入口 = typed `confirm`(approve→compile→durable Confirmed+
    /// 子 WorkItem 落库;confirm 只在 human_confirm 放行,门帧驱动)。
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
            return observation.denied_cell(
                "plan fresh 未达 SC 人工门(无干净的同连接会话可供门上修订)".to_string(),
            );
        };
        observation.workspace_session_id = session_id.to_string();
        // r52(#2 收口):请求基准必须与修订 turn 实际携带的 resume id 同源
        //——author 面(work_item_splitter)最新审计。role 不过滤会读到
        // fresh 轮 cross-review reviewer 的 id(r52 现场:请求 7d13=
        // reviewer,修订实际 --resume 0e97=author,native 同 0e97)。
        observation.requested_resume_id =
            self.latest_audit_native_id(session_id, &self.provider, Some("work_item_splitter"));
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
        // ---- 阶段 2(r58 根修):等 SC 门重开(stage_change human_confirm)
        // 才发 confirm。修订回门 ≠ 门开:产品合法走复评/修复循环(r58 现场
        // reviewer 判修订版仍有缺陷→repairs_used=1/1 自动返修→再复评,全程
        // stage=cross_review),confirm 仅在 human_confirm 放行(protocol.rs
        // SC 矩阵)。旧定时发送=赌复评恰好在 confirm 被处理前回门:r57b
        // 赌赢(confirm 排队 3min13s 后门开才被处理),r58 赌输(INVALID_
        // MESSAGE_FOR_STAGE: confirm not allowed in stage cross_review)。
        // 门帧驱动:期间 choice 语义应答/心跳照常,确定终态秒收口,阶段
        // 超时兜底(有界,不空转)。
        let gate_open = self
            .pump_plan_sc_phase(&mut ws, &mut observation, PlanScPumpPhase::AwaitConfirmGate)
            .await;
        if !gate_open {
            return observation.build_cell(self);
        }
        // ---- 阶段 3:typed confirm → 泵至终态(stage completed/confirmed)。
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
            if recovery_actions_left > 0 && plan_confirm_failure_is_recoverable(&failure_text) {
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
                    history.transitions_used, history.manual_repairs_used, history.repairs_used
                );
                fp("plan_sc_budget_account", format_args!("{account}"));
                observation.run_failure = Some(format!(
                    "{}/{}",
                    observation.run_failure.clone().unwrap_or_default(),
                    account
                ));
            }
        }
        observation.completed_product_artifact_exists = confirmed;
        // r50(#2 残根):plan resume 轮以 cross-review reviewer 驱动收尾
        //——role 不过滤的「最新 audit」取到 reviewer 的 native id(r50 现场
        // 57d8≠请求 4ddb,实测值实为 reviewer 泄漏)。原生恢复确认的比对
        // 面必须是 Author(work_item_splitter)驱动;story/design resume
        // Confirmed 证明 CLI streaming resume 保持同 id,无 fork。
        observation.native_confirmed_id =
            self.latest_audit_native_id(session_id, &self.provider, Some("work_item_splitter"));
        // r51(#2 残根取证):修订轮 Author(work_item_splitter)面最新审计的
        // argv 全文——argv 带 --resume<fresh id> 而 native 变化 = CLI fork;
        // argv 无 --resume = adapter 9b resume 审计门(GC9/drift,mod.rs
        // 758-807 清 resume)在 spawn 前清除。两案修法不同,以本足迹裁决。
        if let Some((_, record)) = self
            .scan_session_audits(session_id)
            .into_iter()
            .find(|(_, record)| record.role == "work_item_splitter")
        {
            fp(
                "plan_resume_author_audit",
                format_args!(
                    "native={} argv_resume={:?} argv={:?}",
                    record.provider_session_id,
                    record
                        .argv
                        .iter()
                        .position(|token| token == "--resume")
                        .map(|index| record.argv.get(index + 1).cloned()),
                    record.argv
                ),
            );
        }
        observation.build_cell(self)
    }

    /// SC 门三相位泵(r58 起门帧驱动):AwaitRevisionComplete 等
    /// `human_gate_turn_completed`(修订轮回门信号);AwaitConfirmGate 等
    /// `stage_change human_confirm`(复评/修复循环后门重开,confirm 合法
    /// 窗口);AwaitConfirmTerminal 等 stage_change completed /
    /// session_state confirmed(定稿终态)。通用面:30s 心跳保活、choice
    /// 语义应答、错误帧与确定终态秒收口(r21/r58 处置面)、阶段超时落
    /// run_failure。
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
                observation.run_failure =
                    Some(format!("plan SC 门阶段超时(phase={phase:?},未达预期信号)"));
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
                    observation.run_failure = Some("plan SC 门会话 WS 在终态前关闭".to_string());
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
                        fp(
                            "plan_sc_pump_choice_request",
                            format_args!("id={choice_id}"),
                        );
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
            &format!(
                "/api/issues/{}/lifecycle?project_id={PROJECT_ID}",
                self.issue_id
            ),
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

    /// r33/r41 A1:已登记需求编号清单(供 plan SC 修订反馈纠偏 AI 编造
    /// 编号)。r41 起复用产品公共提取器 extract_registered_requirement_ids
    /// (story+design 双源,消除 harness/产品双实现漂移);story 与 design
    /// 的最新版本正文都扫。空/缺失返回空串=反馈退回基础文案。
    fn design_requirement_ids_digest(&self) -> String {
        let mut contexts = Vec::new();
        for entity_id in [
            self.story_spec_id.as_deref(),
            self.design_spec_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Ok(mut versions) =
                self.lifecycle
                    .list_versions(PROJECT_ID, &self.issue_id, entity_id)
            {
                versions.sort_by_key(|version| version.version);
                if let Some(latest) = versions.last() {
                    contexts.push(latest.markdown.clone());
                }
            }
        }
        cadence_aria::product::work_item_split_engine::context::extract_registered_requirement_ids(
            &contexts,
        )
        .into_iter()
        .map(|id| format!("- {id}"))
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
        fp(
            "split_sync_begin_handle",
            format_args!("session={workspace_session_id}(同步段:持审计互斥锁)"),
        );
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
        fp(
            "split_sync_handle_ok",
            format_args!("run_ref={}", handle.run_ref),
        );
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
                    format_args!(
                        "spawn_blocking 桥启动,stage 预算={}s(桥线程 lc-gateway-sync-bridge)",
                        self.stage_timeout.as_secs()
                    ),
                );
                let bounded = tokio::time::timeout(
                    self.stage_timeout,
                    tokio::task::spawn_blocking(move || gateway.run_sync(launch)),
                )
                .await;
                fp(
                    "split_sync_run_sync_end",
                    format_args!("bounded={}", bounded.is_ok()),
                );
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
                            let reason = format!("split_sync 结构化产物缺失/收口失败:{output:?}");
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
        let snapshot_boot = self.snapshot_boot;
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
        if snapshot_boot {
            observation.execution_origin = "snapshot_boot".to_string();
        }
        fp_enter_phase("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
        // F5:fresh/resume 观测格与 native 断言扫描共用同一 executor 角色
        // 常量(审计 record.role 为 AdapterRole 序列值,不得漂移)。
        observation.role = CODING_EXECUTOR_ROLE.to_string();
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
        if snapshot_boot {
            resume.execution_origin = "snapshot_boot".to_string();
        }
        fp_enter_phase("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
        resume.role = CODING_EXECUTOR_ROLE.to_string();
        resume.action = "coding_workspace_write".to_string();
        resume.force_resume = true;
        resume.workspace_session_id = attempt_id.clone();
        // F5(r58 深掏审计):两次 native 断言扫描必须按 executor 角色过滤
        //(与观测格 role 对齐,r50/r52 已修 plan author 与 review reviewer,
        // coding 未跟进)——attempt 分区内 review round 的 reviewer
        // ReviewReadOnly 启动同计入;None 读到 reviewer id,resume attach
        // 期间新 reviewer spawn 落在两次扫描之间即 requested≠confirmed
        // 假阴性(r56 Confirmed=时序幸运)。
        resume.requested_resume_id =
            self.latest_audit_native_id(&attempt_id, &self.provider, Some(CODING_EXECUTOR_ROLE));
        resume.frozen_digest = self.latest_audit_projection_digest(&attempt_id, &self.provider);
        let drive = self
            .drive_coding_attempt_ws_inner(&attempt_id, &mut resume, true)
            .await;
        resume.completed_product_artifact_exists = drive.artifact_confirmed;
        resume.observed_pid = self.scan_attempt_stream_log_pid(&attempt_id);
        resume.native_confirmed_id =
            self.latest_audit_native_id(&attempt_id, &self.provider, Some(CODING_EXECUTOR_ROLE));
        cells.push(resume.build_cell(self));
        cells
    }

    async fn drive_coding_attempt_ws(
        &self,
        attempt_id: &str,
        observation: &mut StageObservation,
    ) -> DriveOutcome {
        self.drive_coding_attempt_ws_inner(attempt_id, observation, false)
            .await
    }

    /// r45:resume 形态——重连同 attempt 且不重发 start_coding(blocked/
    /// 中断 attempt 的恢复语义由产品在 attach 后自行裁决;对非 created 态
    /// 重发 start_coding 会被产品拒收,r44 resume 白等 3600s 即此形态)。
    async fn drive_coding_attempt_ws_inner(
        &self,
        attempt_id: &str,
        observation: &mut StageObservation,
        resume: bool,
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
        if !resume && let Err(error) = ws.send_json(&json!({"type": "start_coding"})).await {
            observation.push_event(json!({"type": "matrix_error", "message": error}));
            observation.run_failure = Some(format!("start_coding 发送失败:{error}"));
            return DriveOutcome::default();
        }
        let mut outcome = DriveOutcome::default();
        // r45:blocked 门重试计数(跨帧保持——r44 现场 retry 后同门重开,
        // 第 2 次出现即判重试无效有界落格)。
        let mut blocked_gate_retries: std::collections::BTreeMap<String, u32> =
            std::collections::BTreeMap::new();
        // r50(#2,coding fresh 现场):gate_response 失败计数——retry_review
        // 应答撞 stale-read 竞态(coding_failed_review_recovery_requires_
        // reservation)时补一次延迟重试(恢复 admission 需 durable Blocked
        // 态落定);第 2 次失败即有界落格,不烧满 stage_timeout(r49 现场
        // 34min 空转到阶段超时)。
        let mut gate_response_failures: std::collections::BTreeMap<String, (u32, String)> =
            std::collections::BTreeMap::new();
        let mut last_gate_response: Option<(String, String)> = None;
        // r52(#2):awaiting_manual_recovery 有界恢复(一次 recover_coding,
        // 同态重现即落格)与最近协议错误留痕。
        let mut recovery_seen = 0u32;
        let mut last_protocol_error: Option<String> = None;
        let deadline = tokio::time::Instant::now() + self.stage_timeout;
        let mut idle_deadline = tokio::time::Instant::now() + IDLE_PING_SECS;
        let coding_started = std::time::Instant::now();
        // kimi-9:resume 窗口在途 coder run 重挂检测(见
        // InflightCoderRunTracker;fresh 窗口内启动的 run 不触发)。
        let pump_started_at = Utc::now();
        let mut inflight_tracker = InflightCoderRunTracker::default();
        let mut events_seen = 0u64;
        fp(
            "coding_pump_begin",
            format_args!("attempt={attempt_id} timeout={:?}", self.stage_timeout),
        );
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
                    fp(
                        "coding_pump_closed",
                        format_args!(
                            "elapsed={:?} events={events_seen}",
                            coding_started.elapsed()
                        ),
                    );
                    return outcome;
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    // 空闲保活:防 coding server idle 断连。同 workspace 泵:
                    // ping 后必须重置 idle_deadline,否则过期 deadline 使
                    // timeout_at 立即 Err 退化成 ping 风暴。
                    fp(
                        "coding_pump_idle_ping",
                        format_args!(
                            "elapsed={:?} events={events_seen}(30s 心跳:泵存活/对端静默)",
                            coding_started.elapsed()
                        ),
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
                    // 真实阶段门:按门形状选语义动作应答(默认 [0];
                    // r58 审计 F3:rework 上限门 [0]=provide_context 楔死,
                    // 选 send_to_coder),不旁路产品决策面。
                    let gate_id = message
                        .pointer("/gate/id")
                        .or_else(|| message.pointer("/gate/gate_id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let gate_kind = message
                        .pointer("/gate/kind")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let reason_code = message
                        .pointer("/gate/reason_code")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let action_id = coding_gate_action_id(&message);
                    // r45:blocked 门有界处理——blocked 态的重试动作救不回
                    // 配置类缺失(如 reviewer_configuration_missing,r44 现场
                    // retry_review 后门原样重开,泵空转 3600s×2)。同一
                    // blocked 门第 2 次出现=重试无效,落格如实带门详情。
                    if gate_kind == "blocked" {
                        let seen = blocked_gate_retries.entry(gate_id.to_string()).or_default();
                        *seen += 1;
                        if *seen >= 2 {
                            observation.run_failure = Some(format!(
                                "coding blocked 门重试无效(gate={gate_id} reason={reason_code});真人工恢复门,不空转"
                            ));
                            fp(
                                "coding_pump_blocked_unrecoverable",
                                format_args!("gate={gate_id} reason={reason_code}"),
                            );
                            return outcome;
                        }
                    }
                    if !gate_id.is_empty() && !action_id.is_empty() {
                        last_gate_response = Some((gate_id.to_string(), action_id.to_string()));
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
                "coding_protocol_error" => {
                    // r50(#2):gate_response 失败(retry_review 恢复协议撞
                    // stale-read 竞态)→ 2s 后补一次重试(恢复 admission 需
                    // durable Blocked 态与 role run 落定);同门第 2 次失败
                    // 即有界落格,不空转到阶段超时。
                    let code = message
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    // r52(#2):恢复态落格的最近错误留痕。
                    last_protocol_error = Some(
                        message
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or(code)
                            .to_string(),
                    );
                    if code != "coding_gate_response_failed" {
                        continue;
                    }
                    let Some((gate_id, action_id)) = last_gate_response.clone() else {
                        continue;
                    };
                    let entry = gate_response_failures
                        .entry(gate_id.clone())
                        .or_insert((0u32, String::new()));
                    entry.0 += 1;
                    entry.1 = message
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if entry.0 >= 2 {
                        observation.run_failure = Some(format!(
                            "coding gate_response 重试仍失败(gate={gate_id} action={action_id} error={});恢复协议不可达,不空转",
                            entry.1
                        ));
                        fp(
                            "coding_pump_gate_response_unrecoverable",
                            format_args!("gate={gate_id} action={action_id}"),
                        );
                        return outcome;
                    }
                    fp(
                        "coding_pump_gate_response_retry",
                        format_args!("gate={gate_id} action={action_id} attempt={}", entry.0),
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let _ = ws
                        .send_json(&json!({
                            "type": "gate_response",
                            "gate_id": gate_id,
                            "action_id": action_id,
                            "extra_context": null
                        }))
                        .await;
                }
                "coding_session_state" => {
                    // kimi-9:在途 coder run 重挂检测(resume 格原生恢复
                    // 证据;见 InflightCoderRunTracker)。
                    inflight_tracker.observe(&message, pump_started_at);
                    if inflight_tracker.reattached() {
                        observation.reattached_inflight_run = true;
                    }
                    if let Some(status) = message.get("status").and_then(Value::as_str) {
                        observation.record_status(status);
                        fp(
                            "coding_pump_state",
                            format_args!("status={status} events={events_seen}"),
                        );
                        match status {
                            // r58 审计 F3:waiting_for_human 只有在终门
                            //(stage=final_confirm 且无未决 blocked 门)才是
                            // 确认成功——中途门(rework 上限楔死/choice 挂起/
                            // 共享 worktree 脏)也落本态,一律记 Confirmed=
                            // 终门未达也算过(假阳性 PASS);非终门继续泵
                            //(门应答/终态/阶段超时兜底,有界)。
                            "waiting_for_human" => {
                                if coding_waiting_human_confirms(&message) {
                                    outcome.artifact_confirmed = true;
                                    return outcome;
                                }
                                let stage = message
                                    .get("stage")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                fp(
                                    "coding_pump_waiting_non_final",
                                    format_args!("stage={stage}(终门未达,继续泵)"),
                                );
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
                            // r52(#2 新层):awaiting_manual_recovery(F-16 恢复态,
                            // 唯一非 Abort 放行动作=recover_coding)。r52 现场
                            // 22:54 review round 2 完成后 cross_target_delivery_
                            // blocked(cross_target_violation_detected)进入本态,
                            // 无门帧。按 v1.2 §5.2 先例:发一次 recover_coding
                            // (admission CAS 回 Running+重启 runner);同态第 2
                            // 次出现=恢复无效,有界落格(真实报告,不空转)。
                            "awaiting_manual_recovery" => {
                                recovery_seen += 1;
                                if recovery_seen >= 2 {
                                    observation.terminal_status = Some(status.to_string());
                                    observation.run_failure = Some(format!(
                                        "coding awaiting_manual_recovery 恢复无效(第 {recovery_seen} 次;上次错误={})",
                                        last_protocol_error.clone().unwrap_or_default()
                                    ));
                                    fp(
                                        "coding_pump_recovery_unrecoverable",
                                        format_args!(
                                            "last_error={}",
                                            last_protocol_error.clone().unwrap_or_default()
                                        ),
                                    );
                                    return outcome;
                                }
                                fp("coding_pump_recovery_recover_coding", "attempt=1");
                                let _ = ws.send_json(&json!({"type": "recover_coding"})).await;
                            }
                            _ => {}
                        }
                    }
                }
                // r58 审计 F2:coding choice 帧(顶层 id/prompt/options 形态)
                // ——coder/reviewer 提问即落 WaitingForHuman(gates.rs
                // 321-335),无应答=provider 悬等烧满 stage_timeout。与
                // workspace 泵同构:semantic_choice_answers 单题顶层形态
                // 语义应答(coding 入站 choice_response 同名字段,其余
                // 缺省),不旁路产品决策面。
                "coding_choice_request" => {
                    if let Some(choice_id) = message.get("id").and_then(Value::as_str) {
                        let (answers, top_selected) = semantic_choice_answers(&message);
                        fp(
                            "coding_pump_choice_request",
                            format_args!("id={choice_id} options={}", top_selected.join(",")),
                        );
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
        if self.snapshot_boot {
            observation.execution_origin = "snapshot_boot".to_string();
        }
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
        if self.snapshot_boot {
            resume.execution_origin = "snapshot_boot".to_string();
        }
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
    /// F5:过滤谓词抽纯函数 `latest_native_id_in_audits`(角色过滤语义由
    /// lcg_coding_resume_native_scan_filters_by_executor_role 钉死)。
    fn latest_audit_native_id(
        &self,
        workspace_session_id: &str,
        provider: &ProviderName,
        role: Option<&str>,
    ) -> Option<String> {
        latest_native_id_in_audits(
            &self.scan_session_audits(workspace_session_id),
            provider,
            role,
        )
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
            format_args!(
                "{}/{}/{}(同步段)",
                cell.stage, cell.entrypoint, cell.fresh_or_resume
            ),
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
    /// r52:本观测格的执行来源(full_chain/snapshot_boot;承继格不走本结构)。
    execution_origin: String,
    completed_product_artifact_exists: bool,
    /// r27 问题1:产物 markdown 在场(artifact_update/session_state 携带
    /// 非空 markdown)——plan fresh 的产物判定用(停门模型:候选产物已
    /// 生成,confirmed plan 归 resume 格 confirm 后)。
    artifact_markdown_seen: bool,
    /// kimi-9:resume 窗口内观察到重连前启动的 coder role run 从 running
    /// 走到 completed(在途 run 重挂;attempt 级原生恢复证据之一)。
    reattached_inflight_run: bool,
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
            execution_origin: "full_chain".to_string(),
            completed_product_artifact_exists: false,
            artifact_markdown_seen: false,
            reattached_inflight_run: false,
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
            native_resume_reattached: false,
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
            execution_origin: self.execution_origin.clone(),
        }
    }

    /// 由真实观测(审计 + 事件流 + 产品门)组装证据格。
    fn build_cell(&mut self, env: &MatrixEnvironment) -> EvidenceCell {
        fp(
            "build_cell",
            format_args!(
                "{}/{}(同步段:审计扫描+git 快照)",
                self.stage, self.force_resume
            ),
        );
        let mut cell = self.placeholder_cell();
        cell.process_cwd = env.canonical_root.clone();
        cell.target = env.member_worktree.clone();
        cell.requested_resume_id = self.requested_resume_id.clone();
        cell.native_resume_confirmed_id = self.native_confirmed_id.clone();
        cell.native_resume_reattached = self.reattached_inflight_run;
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
            // resume 格:kimi-9 修订——确认=(请求 id==实测 id)或在途
            // coder run 重挂(判据语义见 coding_resume_native_confirmed)。
            if coding_resume_native_confirmed(
                &self.requested_resume_id,
                &self.native_confirmed_id,
                self.reattached_inflight_run,
            ) {
                cell.capability_state = "confirmed".to_string();
            } else {
                cell.capability_state = "unknown".to_string();
                cell.denied_reason = Some(format!(
                    "原生恢复未确认:请求 {:?} 实测 {:?}(在途重挂={})",
                    self.requested_resume_id,
                    self.native_confirmed_id,
                    self.reattached_inflight_run
                ));
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
        "artifact_update" => message.get("markdown").and_then(Value::as_str).or_else(|| {
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

/// r58 审计 F3-A:coding 门动作选择(纯函数,决策表可单测)。默认取
/// available_actions[0]——triage/配置缺失/interrupted 门的 [0]=retry_*
/// 即正确语义动作。唯一例外=rework 上限门(reason_code=
/// reviewer_rework_limit_reached,动作序 [provide_context, send_to_coder,
/// abort]):provide_context+null 上下文不建 note、不 resolve 门、不唤
/// runner(blocked_gate.inc.rs 42-43/144-163,续跑白名单无 provide_
/// context),恒取 [0]=attempt 楔死在开着的门+假阳性 PASS;选
/// send_to_coder(findings 交 coder 真实重驱)。
fn coding_gate_action_id(message: &Value) -> String {
    let actions = message
        .pointer("/gate/available_actions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let action_id = |action: &Value| -> String {
        action
            .get("action_id")
            .or_else(|| action.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let reason_code = message
        .pointer("/gate/reason_code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason_code == "reviewer_rework_limit_reached"
        && let Some(action) = actions
            .iter()
            .find(|action| action_id(action) == "send_to_coder")
    {
        return action_id(action);
    }
    actions.first().map(action_id).unwrap_or_default()
}

/// r58 审计 F3-B:coding waiting_for_human 收口判据(纯函数,可单测)。
/// coding 的确认成功语义=流程真实走完到终门(stage=final_confirm 的
/// group 终门,确认入口=入站 final_confirm,socket.rs 613-649;矩阵
/// 停门模型不发、停在门上)。中途门(rework 上限/choice 挂起/共享
/// worktree 脏)同样落 waiting_for_human——一律记 Confirmed=终门未达
/// 也算过(假阳性 PASS);终门上还压着未决 blocked 门(如 FinalConfirm
/// 阶段开的 shared_worktree_dirty manual 门,gates.rs 362-392)同样
/// 不可确认,继续泵走门应答/有界失败路径。
fn coding_waiting_human_confirms(frame: &Value) -> bool {
    let stage = frame
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if stage != "final_confirm" {
        return false;
    }
    !frame
        .get("pending_gates")
        .and_then(Value::as_array)
        .is_some_and(|gates| {
            gates
                .iter()
                .any(|gate| gate.get("kind").and_then(Value::as_str) == Some("blocked"))
        })
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

/// r26:SC plan 门 resume 的泵相位;r58 起三相位(confirm 时序门帧驱动)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanScPumpPhase {
    /// 阶段 1:`human_gate_feedback` 修订轮,等 `human_gate_turn_completed`。
    AwaitRevisionComplete,
    /// 阶段 2(r58 根修):修订回门后等 SC 门重开(`stage_change
    /// human_confirm`)。回门≠门开:产品合法走复评/修复循环(reviewer
    /// 判缺陷→repair 预算内自动返修→再复评,r58 现场 repairs_used=1/1),
    /// 全程 stage=cross_review——confirm 仅在 human_confirm 放行
    /// (protocol.rs SC 矩阵),r57b 成功轮证明定时发送=赌复评恰好在
    /// confirm 被处理前回门(排队 3min13s 赌赢;r58 赌输被矩阵拒)。
    /// 门帧才是唯一可靠时序信号。
    AwaitConfirmGate,
    /// 阶段 3:typed `confirm` 定稿,等 stage_change completed /
    /// session_state confirmed。
    AwaitConfirmTerminal,
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
            match (phase, stage) {
                // r58 根修:门重开帧=confirm 合法窗口打开(r57b 09:34:23 帧
                // 序:cross_review 复评完回 human_confirm,排队的 confirm 才
                // 被处理;定时发送在 r58 撞 cross_review 被矩阵拒)。
                (PlanScPumpPhase::AwaitConfirmGate, "human_confirm") => {
                    PlanScFrameSignal::PhaseDone
                }
                (PlanScPumpPhase::AwaitConfirmTerminal, "completed") => {
                    PlanScFrameSignal::PhaseDone
                }
                _ => PlanScFrameSignal::Listen,
            }
        }
        "session_state" => {
            let status = message.get("status").and_then(Value::as_str).unwrap_or("");
            if phase == PlanScPumpPhase::AwaitConfirmTerminal && status == "confirmed" {
                return PlanScFrameSignal::PhaseDone;
            }
            // r58 同族收口:确定失败终态任意相位秒收口——门可能永不再开
            //(AbortFatal/预算尽=failed;StopNeedsHuman=stopped_needs_human
            // 且 stage=completed 只放行 SC Advance),等下去只会空转到阶段
            // 超时(r51 死门教训),终态帧原文落格。编译失败不在此列:
            // recovery/门保持路径落 waiting_for_human(compile.rs 598-617)。
            if matches!(
                status,
                "failed" | "terminated" | "blocked_provider_unavailable" | "stopped_needs_human"
            ) {
                return PlanScFrameSignal::ServerError;
            }
            PlanScFrameSignal::Listen
        }
        _ => PlanScFrameSignal::Listen,
    }
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
fn pinned_story_generate_body(title: &str, member_logical_id: &str, provider_wire: &str) -> Value {
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

/// MaxParallel3(OracleParallel3 硬前提⑤):trust 登记收窄为 selected
/// provider 单元素切片——三家并行时 Codex `~/.codex/config.toml` 仅由
/// Codex 进程写、Kimi workspace-trust 仅由 Kimi 进程写、Pi/Claude 零写,
/// 结构性消除跨进程 read-modify-write 的 TOCTOU(两进程同读旧内容各自
/// rename 会静默丢对方异 root 条目)。
fn selected_trust_targets(provider: &ProviderName) -> &[ProviderName] {
    std::slice::from_ref(provider)
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
            fp(
                "http_end",
                format_args!("{http_label} -> {}", result.0.as_u16()),
            );
            result
        }
        Err(_) => {
            fp(
                "http_timeout",
                format_args!("{http_label}(180s 超时:服务端任务疑似挂起)"),
            );
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
/// r52 快照:承继格(引用来源轮证据链;不重新计票,不参与 Confirmed
/// 断言)。capability_state=carried;denied_reason 携带来源引用说明。
fn carried_cell(
    stage: &str,
    provider: &ProviderName,
    manifest: &super::snapshot::SnapshotManifest,
    snapshot_id: &str,
) -> EvidenceCell {
    EvidenceCell {
        provider: provider.clone(),
        exact_version: manifest.cli_version.clone(),
        stage: stage.to_string(),
        entrypoint: if stage == "split" {
            ENTRYPOINT_SPLIT_SYNC.to_string()
        } else {
            ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string()
        },
        fresh_or_resume: FRESH.to_string(),
        process_cwd: PathBuf::new(),
        target: PathBuf::new(),
        audit_projection_digest: String::new(),
        frozen_projection_digest: String::new(),
        native_resume_confirmed_id: None,
        native_resume_reattached: false,
        requested_resume_id: None,
        argv_or_wire_capture_exists: false,
        approval_and_tool_events_exist: false,
        completed_product_artifact_exists: false,
        run_ref: format!("snapshot-{snapshot_id}-carried-{stage}"),
        run_ref_is_unique_within_entrypoint: true,
        action: String::new(),
        role: String::new(),
        gateway_dialect: String::new(),
        wire_dialect: String::new(),
        native_session_id: String::new(),
        workspace_session_id: String::new(),
        argv: Vec::new(),
        capability_state: "carried".to_string(),
        denied_reason: Some(format!(
            "承继格:来源轮证据链 {}/matrix(快照 {snapshot_id}@{},构建 {});不重新计票",
            manifest.source_evidence_root, manifest.created_at, manifest.harness_revision
        )),
        provider_pid: None,
        pid_unavailable_reason: Some("承继格:证据在来源轮,本轮不重复观测".to_string()),
        provider_spawn_count: 0,
        session_projection_digest: String::new(),
        provider_events: Vec::new(),
        execution_origin: format!("carried:{snapshot_id}"),
    }
}

/// r56 第三类恢复缺口修复:从 issue lifecycle GET 响应恢复 resume 轮
/// env 的 story/design spec id。resume 轮 story/design 为承继格(不
/// 重跑),capture 轮经 generate 响应写入 env 的 spec id 不在快照
/// manifest(schema 冻结,基线快照无该字段)→review 阶段建载体会话
/// 消费 story_spec_ids([] 在产品 500 story_spec_required,r54-r56
/// 三轮同错,前两轮被 coding 失败掩盖)。取数=回灌后 durable 态经
/// 现成只读端点 GET /api/issues/{id}/lifecycle(harness 全程 HTTP
/// 形态与产品交互,保持同风格);快照点=plan_confirmed 前置两 spec
/// Confirmed,故各取「最新 Confirmed 条目」(修订流 Draft/InReview
/// 中间态不参与);任一缺失→Err(BLOCKED 真实报告,不静默空)。
/// 纯函数,两臂由 snapshot_spec_restore_tests 钉死。
fn restored_spec_ids_from_lifecycle(body: &Value) -> Result<(String, String), String> {
    let latest_confirmed = |entries: Option<&Vec<Value>>, id_field: &str| -> Option<String> {
        entries?
            .iter()
            .filter(|entry| {
                entry
                    .get("confirmation_status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status == "confirmed")
            })
            .filter_map(|entry| {
                entry
                    .get(id_field)
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .next_back()
    };
    let story = latest_confirmed(
        body.get("story_specs").and_then(Value::as_array),
        "story_spec_id",
    );
    let design = latest_confirmed(
        body.get("design_specs").and_then(Value::as_array),
        "design_spec_id",
    );
    match (story, design) {
        (Some(story), Some(design)) => Ok((story, design)),
        (None, Some(_)) => Err(
            "story_specs 无 Confirmed 条目(快照点 plan_confirmed 前置 story Confirmed;\
             回灌后 durable 态异常)"
                .to_string(),
        ),
        (Some(_), None) => Err(
            "design_specs 无 Confirmed 条目(快照点 plan_confirmed 前置 design Confirmed;\
             回灌后 durable 态异常)"
                .to_string(),
        ),
        (None, None) => Err(
            "story_specs/design_specs 均无 Confirmed 条目(快照点 plan_confirmed 前置\
             两 spec Confirmed;回灌后 durable 态异常)"
                .to_string(),
        ),
    }
}

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

/// r55:成员仓 `<bare_root>/<member>.git` 本地 bare 路径。
fn member_bare_path(bare_root: &Path, member: &str) -> PathBuf {
    bare_root.join(format!("{member}.git"))
}

/// 仓库 remote URL(None=remote 未配置/命令失败同形)。
fn git_remote_url(path: &Path, remote: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", remote])
        .current_dir(path)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// r55 夹具缺口修复:为成员仓收敛配置本地 bare origin——coding 交付链
///(commit→push→ls-remote)按真实部署假设消费 remote=origin
///(execute_review_request 入参/reconcile.git_remote_branch_head);无
/// origin 时 push/ls-remote fatal,产品 fail-closed 符合设计,夹具侧
/// 补齐真实远端。幂等收敛:remote 缺失→add;URL 漂移→set-url(resume
/// 轮 pristine 回灌携带 capture 轮已销毁 tempdir 的陈旧 URL);已一致
/// →不动。bare 缺失才 init(重复 init 亦无害,但保持确定性)。
/// 返回 bare 仓库路径。只动成员仓 `.git` 配面(tree_digest 跳过 .git,
/// 不影响 pristine digest/HEAD 复核)。
fn ensure_member_bare_origin(bare_root: &Path, member_checkout: &Path) -> PathBuf {
    let member = member_checkout
        .file_name()
        .and_then(|name| name.to_str())
        .expect("member checkout dir name");
    let bare = member_bare_path(bare_root, member);
    if !bare.join("HEAD").exists() {
        std::fs::create_dir_all(&bare).expect("create member bare dir");
        run_git(&bare, &["init", "-q", "--bare"]);
    }
    let desired = bare
        .canonicalize()
        .expect("canonicalize member bare")
        .to_string_lossy()
        .to_string();
    match git_remote_url(member_checkout, "origin") {
        Some(url) if url == desired => {}
        Some(_) => run_git(member_checkout, &["remote", "set-url", "origin", &desired]),
        None => run_git(member_checkout, &["remote", "add", "origin", &desired]),
    }
    bare
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

/// r57:聚合初始化有界重试之间的聚合根复位计划。失败 attempt 可能已把
/// root recipe 工件写进聚合根(AGENTS.md/CLAUDE.md/.claude/…),下一次
/// 新 operation 的严格 preflight 会按 reject_owned_root_files 拒绝
/// (recipe replay 豁免只认 Completed operation 的 receipt 证明);CLI
/// 也可能把成员仓写脏(r57 attempt1 把整套单仓布局写进 alpha/beta,
/// 114 个 DENY)。计划 = 根级「新增于 pristine 之外」的条目(排序稳定
/// 便于足迹比对);pristine 条目本身不删,git 仓复位由
/// [`reset_aggregate_root_for_retry`] 用 git 硬复位完成。
fn aggregate_root_retry_reset_plan(pristine: &[String], current: &[String]) -> Vec<String> {
    let mut extras: Vec<String> = current
        .iter()
        .filter(|name| !pristine.iter().any(|kept| kept == *name))
        .cloned()
        .collect();
    extras.sort();
    extras
}

/// r57:执行 [`aggregate_root_retry_reset_plan`] 的复位——删除根级非
/// pristine 条目;对 pristine 内每个 git 仓(reset --hard + clean -fdx)
/// 从 HEAD 恢复成员 checkout。仅由失败终态(`initialization_failed`,
/// worker 已退出)后的下一轮 attempt 调用,绝不与在跑 worker 并发。
/// 每个动作打一行足迹;git 失败按 `run_git` 断言风格直接 panic(夹具
/// 环境损坏须显式失败,不得静默带脏重试)。
fn reset_aggregate_root_for_retry(canonical_root: &Path, pristine: &[String]) {
    let current: Vec<String> = std::fs::read_dir(canonical_root)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    for name in aggregate_root_retry_reset_plan(pristine, &current) {
        let path = canonical_root.join(&name);
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        fp(
            "agg_root_retry_reset",
            format_args!(
                "remove {name}: {}",
                if removed.is_ok() { "ok" } else { "failed" }
            ),
        );
    }
    for name in pristine {
        let member = canonical_root.join(name);
        if member.join(".git").is_dir() {
            run_git(&member, &["reset", "--hard", "-q", "HEAD"]);
            run_git(&member, &["clean", "-fdxq"]);
            fp(
                "agg_root_retry_reset",
                format_args!("git restore member {name}"),
            );
        }
    }
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
///
/// r61(codex 现场)provider 事件形态映射:codex `commandExecution` item 与
/// pi bash/read 工具执行经产品桥(parse.rs `parse_execution_event`/
/// mapping.rs `ws_execution_event_kind`)映射为 `execution_event`/
/// `coding_execution_event` 帧,inner `event.kind="command"`——协议实名
/// 事件(counted);其余 kind(output/turn/usage/provider)仍不计。
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
        "execution_event" | "coding_execution_event" => {
            // provider 工具执行桥帧:inner event.kind == "command"
            // (codex commandExecution / pi bash-read 工具)。
            if event.pointer("/event/kind").and_then(Value::as_str) == Some("command") {
                return true;
            }
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
///
/// r61(codex 现场):codex app-server 的 `default_mode_request_user_input`
/// 是其 requestUserInput 审批通道的实名启用 flag(069ab065 先例:codex
/// 结构化交互工具=requestUserInput,argv 冻结 `app-server --enable
/// default_mode_request_user_input`)——与 --allowedTools 族同为等价 token。
///
/// r62(kimi-6 split_sync 现场):kimi 的权限/审批投影不走 argv token 族——
/// ACP 协议内 initialize clientCapabilities(fs/terminal 布尔方言)+
/// ClientServicePolicy 决策表(24 格,由 action 派生;66e0d59d 先例:kimi
/// 例外映射,DenyFileWriteBuiltins→None)承担,argv 仅冻结 `acp` 子命令。
/// `acp` 是该控制面的唯一 argv 实名段(非 ACP 模式无 fs/terminal/permission
/// 通道),与 codex 的 requestUserInput flag 同构计入;claude/codex/pi 的
/// argv 形态(-p/app-server/--mode rpc)不含裸 `acp`,无误伤面。
fn argv_carries_permission_wire(argv: &[String]) -> bool {
    argv.iter().any(|argument| {
        argument.starts_with("--allowedTools")
            || argument.starts_with("--disallowedTools")
            || argument.starts_with("--permission-prompt-tool")
            || argument.starts_with("--allowed-tools")
            || argument.starts_with("--exclude-tools")
            || argument.starts_with("--tools")
            || argument == "default_mode_request_user_input"
            || argument == "acp"
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

/// F5(r58 深掏审计):审计分区(最新在前)内该 provider 可选 role 过滤
/// 的最新 provider_start 原生 id(纯函数,过滤语义由
/// lcg_coding_resume_native_scan_filters_by_executor_role 钉死)。
fn latest_native_id_in_audits(
    audits: &[(u64, ProviderStartAudit)],
    provider: &ProviderName,
    role: Option<&str>,
) -> Option<String> {
    audits
        .iter()
        .find(|(_, record)| {
            provider_matches_record(provider, record) && role.is_none_or(|role| record.role == role)
        })
        .map(|(_, record)| record.provider_session_id.clone())
}

/// kimi-9 现场(r10 2026-10-09)coding resume 原生恢复确认判据(纯函数,
/// `coding_resume_native_reattach_tests` 钉死):确认 = (a) 请求 id ==
/// 实测 id(真 native resume:kimi 方言 session/load 不回显、session/update
/// 携带同 id;或零 spawn 空转形——重连即终态,两次扫描读同一条旧审计,
/// kimi-8 形),或 (b) 重连窗口内观察到「重连前启动的 coder role run 从
/// running 走到 completed」(在途 run 重挂:provider 子进程跨 WS 重连存活,
/// 事件继续流到新连接;不产生新 provider_start 审计)。blocked 门
/// retry_coding 按产品冻结设计清除 coder 会话引用后开全新 native 会话
/// (gates_parts/blocked_gate.inc.rs RetryCoding→clear_attempt_provider_
/// conversation),重挂之后的 retry spawn 拿新 id 不构成「未恢复」;两条件
/// 皆缺才是真 fork/未恢复(denied)。
fn coding_resume_native_confirmed(
    requested: &Option<String>,
    confirmed: &Option<String>,
    reattached_inflight_run: bool,
) -> bool {
    match (requested, confirmed) {
        (Some(requested), Some(confirmed)) if requested == confirmed => true,
        (Some(_), Some(_)) => reattached_inflight_run,
        _ => false,
    }
}

/// kimi-9:resume 窗口在途 coder run 重挂检测(coding_session_state 帧流
/// 累积态;`Default` 构造,`observe` 逐帧喂入,纯数据累积可单测)。
#[derive(Default)]
struct InflightCoderRunTracker {
    /// 首见即 running 且 started_at 早于泵启动时刻的 coder run id 集合。
    started_before_window: std::collections::BTreeSet<String>,
    reattached: bool,
}

impl InflightCoderRunTracker {
    fn observe(&mut self, frame: &Value, pump_started_at: chrono::DateTime<chrono::Utc>) {
        let Some(runs) = frame.get("role_runs").and_then(Value::as_array) else {
            return;
        };
        for run in runs {
            let (Some(id), Some(role), status) = (
                run.get("id").and_then(Value::as_str),
                run.get("role").and_then(Value::as_str),
                run.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ) else {
                continue;
            };
            if role != "coder" {
                continue;
            }
            match status {
                "running" => {
                    let started_before = run
                        .get("started_at")
                        .and_then(Value::as_str)
                        .and_then(|started_at| {
                            chrono::DateTime::parse_from_rfc3339(started_at).ok()
                        })
                        .is_some_and(|started| {
                            started.with_timezone(&chrono::Utc) < pump_started_at
                        });
                    if started_before {
                        self.started_before_window.insert(id.to_string());
                    }
                }
                "completed" => {
                    if self.started_before_window.contains(id) {
                        self.reattached = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn reattached(&self) -> bool {
        self.reattached
    }
}

/// F2(r58 深掏审计):快照 CLI 版本漂移裁决(纯函数,三分支由
/// snapshot_cli_version_drift_tests 钉死)。capture 侧 manifest.cli_version
/// 存的是 plan 会话审计 provider_version 原样(CLI `--version` 全串,如
/// "2.1.283 (Claude Code)"),health 侧经 /api/providers/status 透出的是
/// token(如 "2.1.283")——两侧过同一 token 提取后比对:
/// - token 不等 → Some(reason)(早 BLOCKED,明确指示重 capture);
/// - token 相等 → None 放行(不同书写形态不误报,基线快照兼容);
/// - 任一侧提不出 token(空串/无数字词)→ None(无法比对不无谓
///   BLOCKED;spawn 期八维复验仍 fail-closed 兜底)。
fn snapshot_cli_version_drift(frozen: &str, current: Option<&str>) -> Option<String> {
    let frozen_token = cli_version_token(frozen)?;
    let current_token = current.and_then(cli_version_token)?;
    (frozen_token != current_token).then(|| {
        format!(
            "快照 CLI 版本 {frozen_token} != 当前 {current_token}\
             (CLI 已升级/降级:快照冻结的 capability 行与投影指纹全失效,\
             coding/review spawn 复验必 PolicyDrift 拒启;请重 capture 建新基线)"
        )
    })
}

/// CLI 版本串的数字 token 提取(镜像 provider_health::parse_version_token:
/// 首个含数字的空白分词,去除非 [A-Za-z0-9._+-] 的边界字符)。
fn cli_version_token(version: &str) -> Option<String> {
    version.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+'))
        });
        token
            .chars()
            .any(|character| character.is_ascii_digit())
            .then(|| token.to_string())
    })
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
    // r62(pi-6 review fresh 现场):自由文本宣告类选项(「其他(我会在回答
    // 中说明…)」)语义上是 free-text 通道——自动应答选它等于给 provider
    // 一个空洞应答(现场 Q5 的「C. 其他…兼容保持方式」被 confirm_cjk
    // 「保持」误中选中,pi 收到空洞「其他」后无产物收束,artifact gate
    // 拒)。存在非宣告类候选时把宣告类挤出选择(按 Negative 同款排除
    // 语义);全为宣告类时保持原分级(维持 fallback,不空转)。
    let any_non_declaring = options.iter().any(|option| {
        !is_free_text_declaring_option(option.get("label").and_then(Value::as_str).unwrap_or(""))
    });
    let classified: Vec<(usize, SemanticTier)> = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            // r13 复盘:只按 label 分级——description 常含范围界定语
            //("不含自动续期/不含主动登出清理")会误触负向口径,把
            //「(推荐)」正解挤出选择。
            let label = option.get("label").and_then(Value::as_str).unwrap_or("");
            let mut tier = semantic_tier(label);
            if any_non_declaring && is_free_text_declaring_option(label) {
                tier = SemanticTier::Negative;
            }
            (index, tier)
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

/// r62(pi-6 review fresh 现场):自由文本宣告类选项判定——「其他」开头
/// (剥掉 A./B./1. 类拉丁前缀后)或含「我会在回答中说明」宣告语。这类
/// 选项的语义是把答案留给 free-text 通道,自动应答无法为其提供实质
/// 文本;仅做窄匹配(不认英文 "other",防误伤含该词的实义选项)。
fn is_free_text_declaring_option(label: &str) -> bool {
    let normalized = label.trim().to_lowercase();
    let stripped = normalized
        .trim_start_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | ')' | '、'))
        .trim_start();
    stripped.starts_with("其他") || normalized.contains("我会在回答中说明")
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
    // a′(OraclePiSplit,r61 pi split 现场):pi 探索 14 轮的动机全部是
    // 「I need to find the actual schema」——prompt 原来只写「按输出
    // schema」+字段名清单,而生产 split_output_contract 内嵌
    // WORK_ITEM_SPLIT_OUTPUT_SCHEMA 全文;现镜像生产「严格按以下 JSON
    // schema 输出{schema}」形态内嵌冻结 schema(include_str! 与生产源
    // 零漂移),schema 在手后模型无需再翻工作区探索。
    let nonce = "lcg-matrix-split";
    let schema = work_item_split_output_schema();
    format!(
        "你是跨仓逻辑代码库的 work item 拆分引擎。阅读聚合根下成员仓的公开接口与分层,\n\
         按输出 schema 把本次修复拆为 1-3 个可独立交付的 work item(含 title/kind/\n\
         sequence_hint/depends_on/exclusive_write_scopes)。\n\
         只读规划:不写任何文件。\n\n\
         [output]\n\
         使用 nonce `{nonce}` 包裹唯一 JSON:开始标签 \"<ARIA_STRUCTURED_OUTPUT nonce=\\\"{nonce}\\\">\",\n\
         结束标签 \"</ARIA_STRUCTURED_OUTPUT>\"。JSON 顶层必须先含 \"nonce\":\"{nonce}\",\n\
         再含 repository_profile/plan/work_items(不用 Markdown code fence)。\n\
         严格按以下 JSON schema 输出:\n\
         {schema}\n\n\
         nonce 纪律:开始标签必须逐字回显 \"<ARIA_STRUCTURED_OUTPUT nonce=\\\"{nonce}\\\">\"——属性名恰为 `nonce`,属性值两侧必须是英文直双引号(禁止中文弯引号),nonce 值 {nonce} 逐字照抄。"
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
            fp(
                "ws_connect_timeout",
                format_args!("{url}(180s 超时:服务端任务疑似冻结)"),
            );
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

    /// r57b run8 回归锚点:split_sync prompt 是生产 split 输出契约的镜像,
    /// 必须与 `split_output_contract` 同款钉 nonce 引号字形纪律——run8 现场
    /// AI 把起始标签属性值回显为弯/直混排(nonce=“lcg-matrix-split"),解析
    /// 楔死 missing_start_tag,split fresh 落 denied。
    #[test]
    fn lcg_split_sync_prompt_pins_nonce_attribute_glyph_discipline() {
        let prompt = split_sync_prompt();

        assert!(
            prompt.contains("nonce 纪律"),
            "split_sync prompt must pin the nonce discipline line: {prompt}"
        );
        assert!(
            prompt.contains("英文直双引号"),
            "split_sync prompt must require straight ASCII double quotes: {prompt}"
        );
        assert!(
            prompt.contains("禁止中文弯引号"),
            "split_sync prompt must forbid curly quote glyphs: {prompt}"
        );
        assert!(
            prompt.contains("逐字回显"),
            "split_sync prompt must demand verbatim echo of the start tag: {prompt}"
        );
    }

    /// a′(OraclePiSplit,r61 pi split 现场):pi 探索 14 轮的动机全部是
    /// 「I need to find the actual schema」——prompt 只写「按输出 schema」
    /// +字段名清单,而生产 split_output_contract 内嵌
    /// WORK_ITEM_SPLIT_OUTPUT_SCHEMA 全文;镜像生产「严格按以下 JSON
    /// schema 输出{schema}」形态内嵌冻结 schema(harness 已有
    /// include_str! 冻结源,与生产零漂移),模型无需再翻工作区找 schema。
    #[test]
    fn lcg_split_sync_prompt_embeds_output_schema_mirror() {
        let prompt = split_sync_prompt();

        assert!(
            prompt.contains("严格按以下 JSON schema 输出"),
            "split_sync prompt must mirror the production schema directive: {prompt}"
        );
        assert!(
            prompt.contains(&work_item_split_output_schema()),
            "split_sync prompt must embed the frozen WORK_ITEM_SPLIT_OUTPUT_SCHEMA verbatim"
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
    /// r58 回归锚点:SC 门 confirm 时序必须门帧驱动——修订轮回门后产品
    /// 合法走复评/修复循环(r58 现场 repairs_used=1/1,全程 stage=
    /// cross_review),confirm 只在 human_confirm 门重开帧后发。成功轮
    /// r57b 帧序(09:31 confirm 发出排队 → cross_review 3min13s 复评 →
    /// 09:34:23 human_confirm 门开 confirm 才被处理→running→completed)
    /// 证明定时发送=赌复评恰好在 confirm 被处理前回门:r57b 赌赢,r58
    /// 赌输被协议矩阵拒(INVALID_MESSAGE_FOR_STAGE)。
    #[test]
    fn lcg_plan_sc_gate_wait_signal_decision_table() {
        let gate_wait = PlanScPumpPhase::AwaitConfirmGate;
        // r57b 09:34:23.719 帧:门重开=门等相位完成,confirm 此后才合法。
        let gate_open = json!({"type": "stage_change", "stage": "human_confirm"});
        assert_eq!(
            plan_sc_frame_signal("stage_change", &gate_open, gate_wait),
            PlanScFrameSignal::PhaseDone
        );
        // r58 12:16:16.527 帧:复评/修复循环中的 cross_review 继续听。
        let cross_review = json!({"type": "stage_change", "stage": "cross_review"});
        assert_eq!(
            plan_sc_frame_signal("stage_change", &cross_review, gate_wait),
            PlanScFrameSignal::Listen
        );
        // 门未重开前任何其它阶段帧都不驱动 confirm。
        let running = json!({"type": "stage_change", "stage": "running"});
        assert_eq!(
            plan_sc_frame_signal("stage_change", &running, gate_wait),
            PlanScFrameSignal::Listen
        );
        let completed = json!({"type": "stage_change", "stage": "completed"});
        assert_eq!(
            plan_sc_frame_signal("stage_change", &completed, gate_wait),
            PlanScFrameSignal::Listen
        );
        // 已消费过的回门信号在门等相位不重放。
        let turn_completed = json!({"type": "human_gate_turn_completed", "turn_id": "t1"});
        assert_eq!(
            plan_sc_frame_signal("human_gate_turn_completed", &turn_completed, gate_wait),
            PlanScFrameSignal::Listen
        );
        // 门开伴随的 waiting_for_human 不是门帧本身(r13:门以 stage_change
        // 为准),继续听。
        let waiting = json!({"type": "session_state", "status": "waiting_for_human"});
        assert_eq!(
            plan_sc_frame_signal("session_state", &waiting, gate_wait),
            PlanScFrameSignal::Listen
        );
        // 门永不再开的确定终态秒收口(防 r51 死门式空转到阶段超时):
        // AbortFatal/致命失败=session failed;StopNeedsHuman=stopped_needs_
        // human 且 stage=completed(confirm 只放行 SC Advance)。repair
        // 预算尽回 human_confirm 门(routing.rs 346-354)——门等相位正常
        // 收口,不在终态列。
        for status in [
            "failed",
            "terminated",
            "blocked_provider_unavailable",
            "stopped_needs_human",
        ] {
            let terminal = json!({"type": "session_state", "status": status});
            assert_eq!(
                plan_sc_frame_signal("session_state", &terminal, gate_wait),
                PlanScFrameSignal::ServerError,
                "status={status} 必须秒收口,不得空转"
            );
        }
    }
    /// r58 审计 F2 回归锚点:coding choice 帧(coding_choice_request,
    /// protocol.rs 顶层 id/prompt/options 形态,无 questions)必须得到
    /// 语义应答——semantic_choice_answers 的单题顶层形态天然兼容;
    /// coder/reviewer 以 choice 提问即落 WaitingForHuman(gates.rs
    /// 321-335),无应答=provider 悬等烧满 stage_timeout。
    #[test]
    fn lcg_coding_choice_frame_gets_semantic_answers() {
        let frame = json!({
            "type": "coding_choice_request",
            "id": "choice_1",
            "prompt": "review findings 如何处置",
            "source": "reviewer",
            "options": [
                {"id": "opt_continue", "label": "继续修复", "description": "再跑一轮修复"},
                {"id": "opt_stop", "label": "停止修复", "description": "放弃"}
            ],
            "allow_multiple": false,
            "allow_free_text": false,
        });
        let (answers, top_selected) = semantic_choice_answers(&frame);
        assert_eq!(
            top_selected,
            vec!["opt_continue".to_string()],
            "语义应答选正向动作项(继续),规避负向项(停止)"
        );
        assert_eq!(answers.len(), 1, "单题顶层形态构造一个伪题答案");
        assert_eq!(answers[0]["question_id"], json!("default"));
    }

    /// r58 审计 F3-A:rework 上限门动作选择——[0]=provide_context+null
    /// 上下文不建 note、不 resolve 门、不唤 runner(blocked_gate.inc.rs
    /// 42-43/144-163,runner 续跑白名单无 provide_context),恒取 [0]=
    /// attempt 楔死在开着的门;按 reason_code 选 send_to_coder(真实
    /// 续跑:findings 交 coder 重驱,rework.rs 动作序 [provide_context,
    /// send_to_coder, abort])。其余门保持 [0](triage/配置缺失/
    /// interrupted 门的 [0]=retry_* 即正确语义动作)。
    #[test]
    fn lcg_coding_gate_action_prefers_send_to_coder_on_rework_limit() {
        let rework_gate = json!({
            "type": "coding_gate_required",
            "gate": {
                "gate_id": "gate_1",
                "kind": "blocked",
                "reason_code": "reviewer_rework_limit_reached",
                "available_actions": [
                    {"action_id": "provide_context", "label": "补充上下文"},
                    {"action_id": "send_to_coder", "label": "交 coder 修复"},
                    {"action_id": "abort", "label": "中止"}
                ]
            }
        });
        assert_eq!(coding_gate_action_id(&rework_gate), "send_to_coder");
        let triage_gate = json!({
            "type": "coding_gate_required",
            "gate": {
                "gate_id": "gate_2",
                "kind": "blocked",
                "reason_code": "plan_defect_triage",
                "available_actions": [
                    {"action_id": "retry_coding", "label": "重试"},
                    {"action_id": "abort", "label": "中止"}
                ]
            }
        });
        assert_eq!(coding_gate_action_id(&triage_gate), "retry_coding");
        // 空动作表:空串(泵不发送,不旁路产品决策面)。
        assert_eq!(coding_gate_action_id(&json!({"gate": {}})), "");
    }

    /// r58 审计 F3-B:waiting_for_human 收口判据——coding 确认成功语义=
    /// 流程真实走完到终门(stage=final_confirm);中途门(rework 上限/
    /// choice 挂起/共享 worktree 脏)也落 waiting_for_human,一律记
    /// Confirmed=终门未达也算过(假阳性 PASS,r58 审计)。终门上还压着
    /// 未决 blocked 门(如 FinalConfirm 阶段开的 shared_worktree_dirty
    /// manual 门,gates.rs 362-392)同样不可记 Confirmed。
    #[test]
    fn lcg_coding_waiting_human_confirms_only_at_final_gate() {
        let final_gate = json!({
            "type": "coding_session_state",
            "status": "waiting_for_human",
            "stage": "final_confirm",
        });
        assert!(coding_waiting_human_confirms(&final_gate));
        let rework_wedge = json!({
            "type": "coding_session_state",
            "status": "waiting_for_human",
            "stage": "code_review",
        });
        assert!(
            !coding_waiting_human_confirms(&rework_wedge),
            "rework 上限门楔死态不可记 Confirmed"
        );
        let choice_pending = json!({
            "type": "coding_session_state",
            "status": "waiting_for_human",
            "stage": "coding",
        });
        assert!(
            !coding_waiting_human_confirms(&choice_pending),
            "choice 挂起态不可记 Confirmed"
        );
        let dirty_final = json!({
            "type": "coding_session_state",
            "status": "waiting_for_human",
            "stage": "final_confirm",
            "pending_gates": [
                {
                    "gate_id": "gate_dirty",
                    "kind": "blocked",
                    "reason_code": "shared_worktree_dirty_manual_gate",
                    "available_actions": [],
                }
            ],
        });
        assert!(
            !coding_waiting_human_confirms(&dirty_final),
            "终门上未决 blocked 门不可记 Confirmed"
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
        assert!(frame_carries_artifact_markdown(
            "artifact_update",
            &artifact_update
        ));
        // 嵌套 payload 形态兜底同样命中。
        let nested_update = json!({
            "type": "artifact_update",
            "payload": {"markdown": "# 会话过期提示 Story Spec\n内容…", "version": 2},
        });
        assert!(frame_carries_artifact_markdown(
            "artifact_update",
            &nested_update
        ));
        let snapshot = json!({
            "type": "session_state",
            "artifact": {"markdown": "# plan 候选"},
        });
        assert!(frame_carries_artifact_markdown("session_state", &snapshot));
        // 空 markdown/缺失/其他帧不算(plan fresh 产物判定不得被空帧误置)。
        let empty_update = json!({"type": "artifact_update", "payload": {"markdown": "   "}});
        assert!(!frame_carries_artifact_markdown(
            "artifact_update",
            &empty_update
        ));
        let no_artifact = json!({"type": "session_state", "artifact": null});
        assert!(!frame_carries_artifact_markdown(
            "session_state",
            &no_artifact
        ));
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
        assert_eq!(
            plain,
            vec!["REQ-ENV-01", "NFR-PERF-2"],
            "编号保序去重: {ids:?}"
        );
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

    /// r61(codex 现场):tool 事件判据按 provider 协议事件形态映射——
    /// codex `commandExecution` item 经产品桥映射为 `execution_event`/
    /// `coding_execution_event` 帧(inner `event.kind="command"`,
    /// title=Command started/completed/failed);pi 的 bash/read 工具执行
    /// 同走 kind="command"(title=工具名)。这些是 provider 协议实名事件,
    /// 必须计入;非 command kind(output/turn/usage/provider)仍不计——
    /// 映射实名对应,不降低判据。
    #[test]
    fn lcg_tool_event_counts_provider_command_execution_frames() {
        // codex-5 design fresh 现场帧(event_seq 38 原形)。
        let codex_command = json!({
            "type": "execution_event",
            "event_seq": 38,
            "event": {
                "agent": "codex",
                "command": "/usr/bin/zsh -lc 'pwd && ls -la'",
                "event_id": "command_call_00_eiVTyMSaA73Hzk6KZAFG3349",
                "kind": "command",
                "status": "started",
                "title": "Command started"
            }
        });
        assert!(
            is_tool_event(&codex_command),
            "codex commandExecution 桥帧必须计入"
        );
        // codex-5 coding fresh 现场帧(coding_ 前缀,358 帧)。
        let codex_coding_command = json!({
            "type": "coding_execution_event",
            "event": {
                "agent": "codex",
                "command": "cargo test --locked",
                "kind": "command",
                "status": "completed",
                "title": "Command completed"
            }
        });
        assert!(
            is_tool_event(&codex_coding_command),
            "coding 栈 codex commandExecution 桥帧必须计入"
        );
        // pi 现场帧(bash/read 工具执行,kind=command,title=工具名)。
        let pi_bash = json!({
            "type": "execution_event",
            "event": {
                "agent": "pi",
                "kind": "command",
                "title": "bash"
            }
        });
        assert!(is_tool_event(&pi_bash), "pi bash 工具执行帧必须计入");
        // 非 command kind 不得计入(判据不降低)。
        let prompt_echo = json!({
            "type": "execution_event",
            "event": {
                "agent": "codex",
                "kind": "output",
                "title": "Provider Prompt"
            }
        });
        assert!(
            !is_tool_event(&prompt_echo),
            "output/prompt 回显不是 tool 事件"
        );
        let turn_event = json!({
            "type": "execution_event",
            "event": {"kind": "turn", "title": "Turn started"}
        });
        assert!(!is_tool_event(&turn_event), "turn 生命周期不是 tool 事件");
        let usage_event = json!({
            "type": "coding_execution_event",
            "event": {"kind": "usage", "title": "author token usage"}
        });
        assert!(!is_tool_event(&usage_event), "usage 统计不是 tool 事件");
    }

    /// r61(codex 现场):sync 栈无 WS 事件流,argv 权限投影 wire 段即投放
    /// 证据——codex app-server 的 `default_mode_request_user_input` 是其
    /// requestUserInput 审批通道的实名启用 flag(069ab065 先例:codex 结构化
    /// 交互工具=requestUserInput),与 --allowedTools 族同为权限 wire 段。
    #[test]
    fn lcg_argv_carries_permission_wire_recognizes_codex_request_user_input_channel() {
        // codex-5 现场 argv(审计 provider_start 原形)。
        let codex_argv = vec![
            "app-server".to_string(),
            "--enable".to_string(),
            "default_mode_request_user_input".to_string(),
        ];
        assert!(
            argv_carries_permission_wire(&codex_argv),
            "codex requestUserInput 通道 flag 是权限 wire 实名段"
        );
        // 既有 token 族不回归。
        let pi_argv = vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "--exclude-tools".to_string(),
            "edit,write".to_string(),
        ];
        assert!(argv_carries_permission_wire(&pi_argv));
        // 无权限语义的普通 flag 不计入(判据不降低)。
        let plain = vec!["--mode".to_string(), "rpc".to_string()];
        assert!(!argv_carries_permission_wire(&plain));
        let unrelated_enable = vec!["--enable".to_string(), "some_other_feature".to_string()];
        assert!(
            !argv_carries_permission_wire(&unrelated_enable),
            "非审批通道的 --enable 值不得计入"
        );
    }

    /// r62(kimi-6 split_sync 现场):kimi 的权限/审批投影在 ACP 协议内
    /// (initialize clientCapabilities + ClientServicePolicy 决策表,66e0d59d
    /// 例外映射先例),argv 仅冻结 `acp` 子命令——`acp` 是该控制面的唯一
    /// argv 实名段,与 codex requestUserInput flag 同构计入;claude/codex/pi
    /// argv 形态不含裸 `acp`,无误伤面。
    #[test]
    fn lcg_argv_carries_permission_wire_recognizes_kimi_acp_client_services_channel() {
        // kimi-6 split_sync 现场 argv(审计 provider_start 原形)。
        let kimi_argv = vec!["acp".to_string()];
        assert!(
            argv_carries_permission_wire(&kimi_argv),
            "kimi ACP clientServices 控制面子命令是权限 wire 实名段"
        );
        // 非子命令位置的同名词不计入(判据不降低:实名段是子命令形态)。
        let embedded = vec!["--mode".to_string(), "acp-like".to_string()];
        assert!(!argv_carries_permission_wire(&embedded));
    }

    /// r62(pi-6 review fresh 现场):语义应答不得选中自由文本宣告类选项
    /// ——Q5 的「C. 其他(我会在回答中说明调整形态与兼容保持方式)」被
    /// confirm_cjk「保持」误中选中,pi 收到空洞「其他」应答后无产物收束
    /// (artifact gate 拒,pi 无 retry 既有契约);「A. 不调整」含负向「不」
    /// 同样被排除,正确落点是实义选项 B。
    #[test]
    fn lcg_semantic_selection_skips_free_text_declaring_options() {
        // pi-6 review fresh 现场 Q5 选项原形(id=label 全文,pi 方言)。
        let option_a = "A. 不调整：`pub fn cross_repo_greeting() -> &'static str` 保持签名与返回值语义完全不变，新能力全部走新增入口【推荐：[REQ-006] 是「带约束的许可」而非必须调整，零调整即零兼容风险，[AC-007] 自动满足】";
        let option_b = "B. 调整：改变签名或返回值语义（如返回结构体/版本信息），同时保证 beta 侧既有消费路径仍可用";
        let option_c = "C. 其他（我会在回答中说明调整形态与兼容保持方式）";
        let options = json!([
            { "id": option_a, "label": option_a },
            { "id": option_b, "label": option_b },
            { "id": option_c, "label": option_c },
        ]);
        let selected = semantic_option_selection(
            "cross_repo_greeting 调整口径？",
            options.as_array().unwrap(),
            false,
        );
        assert_eq!(
            selected,
            vec![option_b.to_string()],
            "自由文本宣告类(C)与负向「不调整」(A)都必须让位给实义选项 B"
        );
        // 全为宣告类时保持 fallback(不空转,维持首选项语义)。
        let all_declaring = json!([
            { "id": "opt_x", "label": "其他（自由说明）" },
            { "id": "opt_y", "label": "其他（另一形态）" },
        ]);
        let selected =
            semantic_option_selection("任意问题", all_declaring.as_array().unwrap(), false);
        assert_eq!(selected, vec!["opt_x".to_string()]);
    }
}

/// r55 夹具缺口修复面:成员仓本地 bare origin。coding 交付链
///(commit→push→ls-remote)的 remote=origin 硬契约
///(internal_pr_review execute_review_request/reconcile.rs git_remote_branch_head)
/// 需要真实可推送远端;无 origin 时 push/ls-remote fatal(r55 死点)。
mod member_bare_origin_tests {
    use super::*;

    fn member_repo(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        git_repo_at(&path);
        commit(&path, "seed");
        path
    }

    fn canonical_bare_url(bare_root: &Path, member: &str) -> String {
        member_bare_path(bare_root, member)
            .canonicalize()
            .expect("canonicalize bare")
            .to_string_lossy()
            .to_string()
    }

    /// 幂等:首次 ensure 配 origin 指向本地 bare;重复 ensure 不改 URL。
    #[test]
    fn lcg_member_bare_origin_ensure_is_idempotent() {
        let aggregate = tempfile::tempdir().expect("aggregate dir");
        let alpha = member_repo(aggregate.path(), "alpha");
        let bare_root = tempfile::tempdir().expect("bare root");

        assert!(
            git_remote_url(&alpha, "origin").is_none(),
            "建仓时无 origin(git_repo_at 不配 remote)"
        );
        ensure_member_bare_origin(bare_root.path(), &alpha);
        assert_eq!(
            git_remote_url(&alpha, "origin").as_deref(),
            Some(canonical_bare_url(bare_root.path(), "alpha").as_str())
        );

        // 幂等:同 bare 重复 ensure 后 URL 原样(bare 不重建、remote 不重配)。
        ensure_member_bare_origin(bare_root.path(), &alpha);
        assert_eq!(
            git_remote_url(&alpha, "origin").as_deref(),
            Some(canonical_bare_url(bare_root.path(), "alpha").as_str())
        );
    }

    /// resume 轮形态:pristine 回灌携带 capture 轮陈旧 origin URL(tempdir
    /// 已销毁)→ensure 以 set-url 收敛到本轮 bare,而非「存在即跳过」。
    #[test]
    fn lcg_member_bare_origin_ensure_converges_stale_url() {
        let aggregate = tempfile::tempdir().expect("aggregate dir");
        let alpha = member_repo(aggregate.path(), "alpha");
        let capture_bare = tempfile::tempdir().expect("capture bare");
        ensure_member_bare_origin(capture_bare.path(), &alpha);
        drop(capture_bare);

        let resume_bare = tempfile::tempdir().expect("resume bare");
        ensure_member_bare_origin(resume_bare.path(), &alpha);
        assert_eq!(
            git_remote_url(&alpha, "origin").as_deref(),
            Some(canonical_bare_url(resume_bare.path(), "alpha").as_str()),
            "陈旧 URL 必须被收敛到本轮 bare(resume 可续跑)"
        );
    }

    /// r55 死点端到端复原:产品在主 checkout 挂隔离 worktree
    ///(SAFE_WORKTREE_PREFIXES 形态)→coder 在 worktree commit→push
    /// origin <branch>→产品同形 ls-remote 读到远端头=本地提交
    ///(finish_nonzero_review_push 的 Pushed 判定输入)。
    #[test]
    fn lcg_member_bare_origin_worktree_push_then_ls_remote_reads_head() {
        let aggregate = tempfile::tempdir().expect("aggregate dir");
        let alpha = member_repo(aggregate.path(), "alpha");
        let bare_root = tempfile::tempdir().expect("bare root");
        ensure_member_bare_origin(bare_root.path(), &alpha);

        // 产品同形嵌套 worktree:主 checkout 下 .worktrees/aria-issues/<issue>。
        let worktree = alpha
            .join(".worktrees")
            .join("aria-issues")
            .join("issue_0001");
        run_git(
            &alpha,
            &[
                "worktree",
                "add",
                "-q",
                worktree.to_string_lossy().as_ref(),
                "-b",
                "aria/issues/issue_0001",
            ],
        );
        assert_eq!(
            git_remote_url(&worktree, "origin"),
            git_remote_url(&alpha, "origin"),
            "隔离 worktree 必须共享主 checkout 的 origin 配置(remotes 在公共 config)"
        );

        // 推送前:产品 ls-remote 同形命令输出为空(Ok(None)=未推送,
        // 不再是 r55 的 fatal)。
        let ls_remote = |cwd: &Path| {
            let output = std::process::Command::new("git")
                .args([
                    "ls-remote",
                    "--heads",
                    "origin",
                    "refs/heads/aria/issues/issue_0001",
                ])
                .current_dir(cwd)
                .output()
                .expect("ls-remote spawn");
            assert!(
                output.status.success(),
                "ls-remote 必须可用:{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        assert_eq!(ls_remote(&worktree), "", "推送前远端分支不存在");

        // coder 交付形态:worktree 内提交并 push origin <branch>。
        std::fs::write(worktree.join("lib.rs"), "pub fn delivered() -> u8 { 1 }")
            .expect("write worktree change");
        run_git(&worktree, &["add", "."]);
        run_git(
            &worktree,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "feat: deliver",
            ],
        );
        run_git(
            &worktree,
            &["push", "-q", "origin", "aria/issues/issue_0001"],
        );

        let head = git_head(&worktree).expect("worktree head");
        let remote_line = ls_remote(&worktree);
        assert_eq!(
            remote_line.split_whitespace().next().unwrap_or_default(),
            head,
            "push 后 ls-remote 远端头必须等于本地提交(Pushed 判定可达成)"
        );
    }
}

/// r56 第三类恢复缺口回归锚点:resume 轮 story/design 为承继格(不重跑),
/// capture 轮经 generate 响应写入 env 的 spec id 不在快照 manifest(schema
/// 冻结)——env 必须从回灌后 durable 态重取。「有 Confirmed spec→回填」与
/// 「无 Confirmed→BLOCKED(不静默空)」两臂钉死(r54-r56 三轮 review 同错
/// 500 story_spec_required:建会话 POST story_spec_ids:[])。
mod snapshot_spec_restore_tests {
    use super::*;

    fn story_entry(id: &str, status: &str) -> Value {
        json!({
            "story_spec_id": id,
            "confirmation_status": status,
        })
    }

    fn design_entry(id: &str, status: &str) -> Value {
        json!({
            "design_spec_id": id,
            "confirmation_status": status,
        })
    }

    fn lifecycle_body(story_specs: Vec<Value>, design_specs: Vec<Value>) -> Value {
        json!({
            "issue": {"issue_id": "issue_0001"},
            "story_specs": story_specs,
            "design_specs": design_specs,
        })
    }

    /// 快照点形态:story/design 各一条 Confirmed → 两 id 回填。
    #[test]
    fn lcg_restored_spec_ids_confirmed_pair_fills_both() {
        let body = lifecycle_body(
            vec![story_entry("story_spec_0001", "confirmed")],
            vec![design_entry("design_spec_0001", "confirmed")],
        );
        assert_eq!(
            restored_spec_ids_from_lifecycle(&body).expect("Confirmed 成对必须恢复"),
            (
                "story_spec_0001".to_string(),
                "design_spec_0001".to_string()
            )
        );
    }

    /// 多条目取最新 Confirmed(列表尾);非 Confirmed 条目不参与——
    /// 修订流会产生 Draft/InReview 中间态,静默取首条会拿错版本。
    #[test]
    fn lcg_restored_spec_ids_picks_latest_confirmed_ignoring_drafts() {
        let body = lifecycle_body(
            vec![
                story_entry("story_spec_0001", "draft"),
                story_entry("story_spec_0002", "confirmed"),
                story_entry("story_spec_0003", "in_review"),
                story_entry("story_spec_0004", "confirmed"),
            ],
            vec![design_entry("design_spec_0001", "confirmed")],
        );
        assert_eq!(
            restored_spec_ids_from_lifecycle(&body).expect("存在 Confirmed 条目必须恢复"),
            (
                "story_spec_0004".to_string(),
                "design_spec_0001".to_string()
            )
        );
    }

    /// 无 Confirmed story → BLOCKED(Err),不静默空——review 阶段消费
    /// story_spec_ids,空数组在产品 500 story_spec_required。
    #[test]
    fn lcg_restored_spec_ids_blocked_without_confirmed_story() {
        let body = lifecycle_body(
            vec![story_entry("story_spec_0001", "draft")],
            vec![design_entry("design_spec_0001", "confirmed")],
        );
        let reason =
            restored_spec_ids_from_lifecycle(&body).expect_err("无 Confirmed story 必须 BLOCKED");
        assert!(reason.contains("story"), "原因须点名 story:{reason}");
    }

    /// 无 Confirmed design → 同口径 BLOCKED(快照点 plan_confirmed 前置
    /// 两 spec Confirmed;缺失=durable 态异常,真实报告)。
    #[test]
    fn lcg_restored_spec_ids_blocked_without_confirmed_design() {
        let body = lifecycle_body(vec![story_entry("story_spec_0001", "confirmed")], vec![]);
        let reason =
            restored_spec_ids_from_lifecycle(&body).expect_err("无 Confirmed design 必须 BLOCKED");
        assert!(reason.contains("design"), "原因须点名 design:{reason}");
    }

    /// 字段缺失/空数组(旧 durable 态或半恢复现场)→ BLOCKED,不 panic。
    #[test]
    fn lcg_restored_spec_ids_blocked_on_missing_fields() {
        let body = json!({"issue": {"issue_id": "issue_0001"}});
        assert!(restored_spec_ids_from_lifecycle(&body).is_err());
    }
}

/// F2(r58 深掏审计)回归锚:快照 CLI 版本漂移裁决三分支——token 漂移
/// BLOCKED(原因点名两侧版本+重 capture 指示)/同 token 不同书写形态
/// 放行(r57 基线 manifest 存全串、health 透 token,不得误报)/任一侧
/// 不可解析跳过(不无谓 BLOCKED,spawn 复验兜底)。
mod snapshot_cli_version_drift_tests {
    use super::*;

    #[test]
    fn lcg_snapshot_cli_version_drift_blocks_on_token_drift() {
        let reason = snapshot_cli_version_drift("2.1.283 (Claude Code)", Some("2.1.300"))
            .expect("token 不等必须 BLOCKED");
        assert!(reason.contains("2.1.283"), "原因须点名冻结版本:{reason}");
        assert!(reason.contains("2.1.300"), "原因须点名当前版本:{reason}");
        assert!(reason.contains("capture"), "原因须指示重 capture:{reason}");
    }

    #[test]
    fn lcg_snapshot_cli_version_passes_on_token_equal_across_forms() {
        // r57 基线形态:manifest 冻结 CLI --version 全串,health 侧 token。
        assert_eq!(
            snapshot_cli_version_drift("2.1.283 (Claude Code)", Some("2.1.283")),
            None,
            "同 token 不同书写形态必须放行(基线快照兼容,不无谓 BLOCKED)"
        );
    }

    #[test]
    fn lcg_snapshot_cli_version_skips_when_token_unparseable() {
        assert_eq!(
            snapshot_cli_version_drift("", Some("2.1.283")),
            None,
            "冻结侧空串(capture 边缘形态)无比对基准,不 BLOCKED"
        );
        assert_eq!(
            snapshot_cli_version_drift("2.1.283", None),
            None,
            "当前版本不可得不无谓 BLOCKED,spawn 期复验兜底"
        );
        assert_eq!(
            snapshot_cli_version_drift("no-digits", Some("2.1.283")),
            None,
            "冻结侧提不出数字 token 不构成漂移证据"
        );
    }
}

/// F4(r58 深掏审计)前提钉子:HOME trust 工件被清理(容器重建/换机/
/// HOME 清理)后幂等重调 ensure_before_recipe 必须重登成功——resume 轮
/// build_from_snapshot 已与 build() 同源重登(工件在场=Replay 幂等,
/// 缺工件=重新登记),本测试钉死「缺工件→重登成功」臂。
/// MaxParallel3 改钉:登记口径与 harness 同为 selected-only——每家
/// 只通过自己的单元素切片重登,另一家工件不因本家恢复被触碰。
mod snapshot_resume_trust_tests {
    use super::*;

    fn selected_registry(
        home: &std::path::Path,
        aria: &std::path::Path,
    ) -> HomeBackedProviderTrustRegistry {
        HomeBackedProviderTrustRegistry::new(
            ProductAppPaths::new(aria.to_path_buf()),
            vec![
                Arc::new(CodexTrustAdapter::for_home(home)),
                Arc::new(KimiTrustAdapter::for_home(home)),
            ],
        )
    }

    #[test]
    fn lcg_resume_trust_reensure_recovers_wiped_home_artifacts() {
        let home = TempDir::new().expect("home");
        let aria = TempDir::new().expect("aria root");
        let canonical_root = home.path().join("lc-root");
        std::fs::create_dir_all(&canonical_root).expect("canonical root");
        let registry = selected_registry(home.path(), aria.path());
        let ensure = |selected: &ProviderName| {
            matches!(
                registry.ensure_before_recipe(
                    PROJECT_ID,
                    "lcg-matrix-trust-resume",
                    "logical_codebase_resume",
                    &canonical_root,
                    selected_trust_targets(selected),
                ),
                cadence_aria::product::logical_codebase::ProviderTrustPreparationResult::Ready { .. }
            )
        };
        assert!(
            ensure(&ProviderName::Codex) && ensure(&ProviderName::KimiCode),
            "首轮各家 selected 登记必须 Ready"
        );
        // HOME 工件清理(capture→resume 之间换机/容器重建形态):codex
        // config.toml 与 kimi workspace-trust 全部抹掉(.aria 登记记录
        // 仍在——快照回灌形态)。
        let _ = std::fs::remove_file(home.path().join(".codex").join("config.toml"));
        let _ = std::fs::remove_dir_all(home.path().join(".kimi-code").join("workspace-trust"));
        assert!(
            ensure(&ProviderName::Codex),
            "codex selected 重登必须重新 Ready(resume 轮 F4 前提)"
        );
        assert!(
            ensure(&ProviderName::KimiCode),
            "kimi selected 重登必须重新 Ready(resume 轮 F4 前提)"
        );
        // HOME 工件确已重建(codex config 携带 trusted 条目)。
        let config = std::fs::read_to_string(home.path().join(".codex").join("config.toml"))
            .expect("codex config rebuilt");
        assert!(
            config.contains("trust_level = \"trusted\""),
            "重登后 HOME 工件必须重建:\n{config}"
        );
    }
}

/// MaxParallel3 trust 收窄(OracleParallel3 硬前提⑤)钉子:harness 的
/// trust 登记面必须按 selected provider 单元素切片驱动 registry——旧
/// 全量 `[Codex, Pi, KimiCode]` 形态会让 kimi 进程并发写 codex 的
/// `~/.codex/config.toml`(跨进程 RMW→rename 静默丢异 root 条目),
/// F4 清 HOME 恢复轮同样触发。本组以 harness 同一 helper 驱动
/// registry,钉死零跨家写。
mod harness_trust_selection_tests {
    use super::*;

    fn run_selected_trust_pass(
        home: &std::path::Path,
        aria: &std::path::Path,
        canonical_root: &std::path::Path,
        lc_id: &str,
        selected: &ProviderName,
    ) -> bool {
        let registry = HomeBackedProviderTrustRegistry::new(
            ProductAppPaths::new(aria.to_path_buf()),
            vec![
                Arc::new(CodexTrustAdapter::for_home(home)),
                Arc::new(KimiTrustAdapter::for_home(home)),
            ],
        );
        matches!(
            registry.ensure_before_recipe(
                PROJECT_ID,
                "lcg-matrix-trust-selected",
                lc_id,
                canonical_root,
                selected_trust_targets(selected),
            ),
            cadence_aria::product::logical_codebase::ProviderTrustPreparationResult::Ready { .. }
        )
    }

    #[test]
    fn lcg_trust_targets_pin_selected_only_slice() {
        // 切片形状钉子:恒单元素、按引用借入(无 clone/无分配);
        // pi/claude 等零 trust 家原样透传,由 registry 过滤。
        assert_eq!(
            selected_trust_targets(&ProviderName::Codex),
            &[ProviderName::Codex]
        );
        assert_eq!(
            selected_trust_targets(&ProviderName::KimiCode),
            &[ProviderName::KimiCode]
        );
        assert_eq!(
            selected_trust_targets(&ProviderName::Pi),
            &[ProviderName::Pi]
        );
    }

    #[test]
    fn lcg_selected_trust_kimi_run_writes_no_codex_config() {
        let home = TempDir::new().expect("home");
        let aria = TempDir::new().expect("aria root");
        let canonical_root = home.path().join("lc-root");
        std::fs::create_dir_all(&canonical_root).expect("canonical root");
        assert!(
            run_selected_trust_pass(
                home.path(),
                aria.path(),
                &canonical_root,
                "logical_codebase_kimi",
                &ProviderName::KimiCode,
            ),
            "kimi selected 登记必须 Ready"
        );
        assert!(
            home.path()
                .join(".kimi-code")
                .join("workspace-trust")
                .exists(),
            "kimi 自家 trust 工件必须生成"
        );
        assert!(
            !home.path().join(".codex").join("config.toml").exists(),
            "kimi 运行不得写 codex 的 ~/.codex/config.toml(旧全量数组行为)"
        );
    }

    #[test]
    fn lcg_selected_trust_codex_run_writes_no_kimi_artifacts() {
        let home = TempDir::new().expect("home");
        let aria = TempDir::new().expect("aria root");
        let canonical_root = home.path().join("lc-root");
        std::fs::create_dir_all(&canonical_root).expect("canonical root");
        assert!(
            run_selected_trust_pass(
                home.path(),
                aria.path(),
                &canonical_root,
                "logical_codebase_codex",
                &ProviderName::Codex,
            ),
            "codex selected 登记必须 Ready"
        );
        let config = std::fs::read_to_string(home.path().join(".codex").join("config.toml"))
            .expect("codex config written");
        assert!(
            config.contains("trust_level = \"trusted\""),
            "codex 自家 config 必须携带 trusted 条目:\n{config}"
        );
        assert!(
            !home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .exists(),
            "codex 运行不得生成 kimi workspace-trust(旧全量数组行为)"
        );
    }

    #[test]
    fn lcg_selected_trust_zero_write_providers_touch_no_home() {
        // pi/claude 无 workspace trust 需要:registry 过滤后零 HOME 写,
        // 也不需要另一家条目(设计验证矩阵 selected Pi/Claude 行)。
        for provider in [ProviderName::Pi, ProviderName::ClaudeCode] {
            let home = TempDir::new().expect("home");
            let aria = TempDir::new().expect("aria root");
            let canonical_root = home.path().join("lc-root");
            std::fs::create_dir_all(&canonical_root).expect("canonical root");
            assert!(
                run_selected_trust_pass(
                    home.path(),
                    aria.path(),
                    &canonical_root,
                    "logical_codebase_zero_write",
                    &provider,
                ),
                "{provider:?} 零 trust 家必须直接 Ready"
            );
            assert!(
                !home.path().join(".codex").join("config.toml").exists(),
                "{provider:?} 运行不得写 codex config"
            );
            assert!(
                !home
                    .path()
                    .join(".kimi-code")
                    .join("workspace-trust")
                    .exists(),
                "{provider:?} 运行不得写 kimi workspace-trust"
            );
        }
    }
}

/// F5(r58 深掏审计)回归锚:coding resume 两次 native 断言扫描的
/// executor 角色过滤语义——attempt 分区(最新在前)内 review round 的
/// reviewer ReviewReadOnly 启动同计入,None 读到 reviewer id(r56
/// Confirmed=时序幸运);Some("executor") 恒取 executor 最新 id。
mod coding_resume_role_filter_tests {
    use super::*;

    fn start(provider: &str, role: &str, native_id: &str) -> ProviderStartAudit {
        ProviderStartAudit {
            provider: provider.to_string(),
            role: role.to_string(),
            provider_session_id: native_id.to_string(),
            ..ProviderStartAudit::default()
        }
    }

    #[test]
    fn lcg_coding_resume_native_scan_filters_by_executor_role() {
        let provider = ProviderName::ClaudeCode;
        // 最新在前(seq 降序):reviewer spawn 晚于 executor spawn。
        let audits = vec![
            (2u64, start("claude-code", "reviewer", "reviewer-native-2")),
            (1u64, start("claude-code", "executor", "exec-native-1")),
        ];
        // 旧形态(None):读到 reviewer id——resume attach 期间新 reviewer
        // spawn 落在两次扫描之间即 requested≠confirmed 假阴性(F5 机制)。
        assert_eq!(
            latest_native_id_in_audits(&audits, &provider, None),
            Some("reviewer-native-2".to_string()),
            "None=分区最新(含 reviewer 污染),钉住 F5 缺陷形态"
        );
        // 修复形态:与观测格 role 对齐的 executor 过滤恒取 executor id。
        assert_eq!(
            latest_native_id_in_audits(&audits, &provider, Some(CODING_EXECUTOR_ROLE)),
            Some("exec-native-1".to_string()),
            "executor 过滤必须命中 executor 最新启动"
        );
        // provider 不匹配的记录不参与(过滤谓词另一维)。
        let mixed = vec![
            (3u64, start("codex", "executor", "codex-native-3")),
            (2u64, start("claude-code", "executor", "exec-native-2")),
        ];
        assert_eq!(
            latest_native_id_in_audits(&mixed, &provider, Some(CODING_EXECUTOR_ROLE)),
            Some("exec-native-2".to_string()),
            "provider 维过滤不得被他家 provider 启动污染"
        );
    }
}

/// kimi-9 现场(r10 2026-10-09)coding resume 判据修订:coding attempt 的
/// 原生恢复有两条合法形态——(a)executor native id 前后一致(真 session
/// 复用,或零 spawn 空转形,kimi-8 形);(b)在途 coder run 重挂(WS 重连后
/// 重连前启动的 coder role run 继续流出事件至 completed,不产生新
/// provider_start 审计;kimi-9 现场 run 0019:03:10:18 启动、03:13:26 完成,
/// 跨 fresh/resume 边界)。blocked 门 retry_coding 按产品冻结设计清除 coder
/// 会话引用后开全新 native 会话(gates_parts/blocked_gate.inc.rs
/// RetryCoding→clear_attempt_provider_conversation),重挂后的 retry spawn
/// 拿新 id 不构成「未恢复」。判据纯函数与重挂检测由本模块钉死。
mod coding_resume_native_reattach_tests {
    use super::*;

    #[test]
    fn lcg_coding_resume_native_confirmed_accepts_id_equality_or_reattachment() {
        // (a) 请求==实测:真 native resume(kimi 方言 session/load→
        // session/update 同 id)或零 spawn 空转形(kimi-8:重连即终态,
        // 两次扫描读同一条旧审计)。
        assert!(coding_resume_native_confirmed(
            &Some("session-a".to_string()),
            &Some("session-a".to_string()),
            false
        ));
        // (b) kimi-9 形:实测 id 变(retry 门清引用后全新会话)+ 在途
        // coder run 重挂证据 → 恢复成立。
        assert!(coding_resume_native_confirmed(
            &Some("session-ddde50e2".to_string()),
            &Some("session-8e4c3334".to_string()),
            true
        ));
        // 真 fork/未恢复:实测 id 变且无重挂证据 → 不确认。
        assert!(!coding_resume_native_confirmed(
            &Some("session-a".to_string()),
            &Some("session-b".to_string()),
            false
        ));
        // 双缺(扫描不到 executor 启动)不因重挂标志放行。
        assert!(!coding_resume_native_confirmed(&None, &None, true));
    }

    fn state_frame(runs: &[(&str, &str, &str, &str)]) -> Value {
        json!({
            "type": "coding_session_state",
            "status": "running",
            "role_runs": runs
                .iter()
                .map(|(id, role, status, started_at)| {
                    json!({
                        "id": id,
                        "role": role,
                        "status": status,
                        "started_at": started_at,
                    })
                })
                .collect::<Vec<_>>(),
        })
    }

    #[test]
    fn lcg_inflight_coder_run_tracker_detects_pre_window_run_completion() {
        // kimi-9 现场:resume 窗口 03:12:18 起;run 0019 03:10:18 启动
        //(窗口前)、首帧 running、后续帧 completed → 重挂证据成立。
        let pump_started = chrono::DateTime::parse_from_rfc3339("2026-10-09T03:12:18Z")
            .expect("ts")
            .with_timezone(&chrono::Utc);
        let mut tracker = InflightCoderRunTracker::default();
        tracker.observe(
            &state_frame(&[(
                "coding_role_run_0019",
                "coder",
                "running",
                "2026-10-09T03:10:18.933682317+00:00",
            )]),
            pump_started,
        );
        assert!(!tracker.reattached(), "首帧 running 只登记候选,不判定");
        tracker.observe(
            &state_frame(&[(
                "coding_role_run_0019",
                "coder",
                "completed",
                "2026-10-09T03:10:18.933682317+00:00",
            )]),
            pump_started,
        );
        assert!(
            tracker.reattached(),
            "窗口前启动的 coder run 在窗口内完成=重挂"
        );
    }

    #[test]
    fn lcg_inflight_coder_run_tracker_ignores_window_started_and_non_coder_runs() {
        let pump_started = chrono::DateTime::parse_from_rfc3339("2026-10-09T03:12:18Z")
            .expect("ts")
            .with_timezone(&chrono::Utc);
        let mut tracker = InflightCoderRunTracker::default();
        // 窗口内启动的 run(重连后新 spawn)不是重挂;reviewer run 不算。
        tracker.observe(
            &state_frame(&[
                (
                    "coding_role_run_0020",
                    "coder",
                    "running",
                    "2026-10-09T03:13:32.316936864+00:00",
                ),
                (
                    "coding_role_run_0008",
                    "code_reviewer",
                    "running",
                    "2026-10-09T02:36:47.435235214+00:00",
                ),
            ]),
            pump_started,
        );
        tracker.observe(
            &state_frame(&[
                (
                    "coding_role_run_0020",
                    "coder",
                    "completed",
                    "2026-10-09T03:13:32.316936864+00:00",
                ),
                (
                    "coding_role_run_0008",
                    "code_reviewer",
                    "completed",
                    "2026-10-09T02:36:47.435235214+00:00",
                ),
            ]),
            pump_started,
        );
        assert!(
            !tracker.reattached(),
            "窗口内启动的 coder run 与 reviewer run 的完成都不是重挂证据"
        );
    }
}

#[cfg(test)]
mod aggregate_root_retry_reset_tests {
    use super::*;

    /// r57 钉死:复位计划只挑 pristine 之外的新增条目,排序稳定。
    #[test]
    fn lcg_retry_reset_plan_picks_only_non_pristine_entries_sorted() {
        let pristine = vec![
            "alpha".to_string(),
            "beta".to_string(),
            "seed.md".to_string(),
        ];
        // 失败 attempt 后的根:成员 + 预置文件 + CLI 写入的 recipe 工件。
        let current = vec![
            "CLAUDE.md".to_string(),
            "beta".to_string(),
            "cadence".to_string(),
            "alpha".to_string(),
            "AGENTS.md".to_string(),
            "seed.md".to_string(),
        ];
        assert_eq!(
            aggregate_root_retry_reset_plan(&pristine, &current),
            vec![
                "AGENTS.md".to_string(),
                "CLAUDE.md".to_string(),
                "cadence".to_string(),
            ],
            "r57 attempt2 把 AGENTS.md/CLAUDE.md 写进聚合根后,attempt3 的\
             严格 preflight 永拒——重试必须先按计划删掉这些新增条目"
        );
        // 干净根(与 pristine 一致)零动作。
        assert!(aggregate_root_retry_reset_plan(&pristine, &pristine).is_empty());
    }

    /// r57 钉死(真 git):复位删除根级 recipe 工件、成员仓硬复位回
    /// HEAD、pristine 非 git 条目原样保留。
    #[test]
    fn lcg_retry_reset_removes_recipe_artifacts_and_restores_members() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(root.join("alpha")).expect("create member");
        git_repo_at(&root.join("alpha"));
        std::fs::write(root.join("alpha").join("lib.rs"), "committed").expect("seed");
        run_git(&root.join("alpha"), &["add", "."]);
        run_git(
            &root.join("alpha"),
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "seed",
            ],
        );
        std::fs::write(root.join("seed.md"), "pristine").expect("pristine file");
        let pristine = vec![
            "alpha".to_string(),
            "seed.md".to_string(),
            // 根级 CLI 工件在 pristine 之外。
        ];

        // 失败 attempt 的现场:根级 recipe 工件 + 成员仓脏写(r57
        // attempt1 把 rules/.mcp.json 写进成员仓的形态)。
        std::fs::write(root.join("AGENTS.md"), "cli wrote").expect("artifact");
        std::fs::create_dir_all(root.join(".claude").join("rules")).expect("rules dir");
        std::fs::write(
            root.join(".claude").join("rules").join("language.md"),
            "cli wrote",
        )
        .expect("rule");
        std::fs::write(root.join("alpha").join("dirty-rules.md"), "cli wrote").expect("dirty");

        reset_aggregate_root_for_retry(&root, &pristine);

        assert!(!root.join("AGENTS.md").exists(), "根级 recipe 工件必须删除");
        assert!(
            !root.join(".claude").exists(),
            "根级 recipe 目录必须整体删除"
        );
        assert!(
            root.join("seed.md").exists(),
            "pristine 非 git 条目必须原样保留"
        );
        assert!(
            !root.join("alpha").join("dirty-rules.md").exists(),
            "成员仓脏写必须被 git 复位清掉"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("alpha").join("lib.rs")).expect("lib"),
            "committed",
            "成员仓已提交内容必须从 HEAD 恢复"
        );
    }
}

#[cfg(test)]
mod d4_observability_tests {
    use super::*;
    use serde_json::json;

    /// r63 kimi 现场回归锚点:root/metadata 面必须以逐条目清单暴露
    /// (成员工作树 alpha/beta 与 .codegraph 排除;其余实名入清单)——
    /// 聚合 digest 漂移时漂移物可自证,不再只有不可拆解的 digest。
    #[test]
    fn lcg_d4_root_metadata_face_lists_entries_and_excludes_members() {
        let root = TempDir::new().expect("aggregate root");
        let base = root.path();
        std::fs::create_dir_all(base.join("alpha/.git")).expect("alpha git");
        std::fs::create_dir_all(base.join("alpha/src")).expect("alpha src");
        std::fs::write(base.join("alpha/src/lib.rs"), "alpha").expect("alpha file");
        std::fs::create_dir_all(base.join("beta")).expect("beta");
        std::fs::write(base.join("beta/README.md"), "beta").expect("beta file");
        std::fs::create_dir_all(base.join(".codegraph")).expect("codegraph");
        std::fs::write(base.join(".codegraph/state.db"), "watcher").expect("codegraph file");
        std::fs::create_dir_all(base.join(".provider-session-cache/66f7c0dc6712e775"))
            .expect("provider session cache");
        std::fs::write(
            base.join(".provider-session-cache/66f7c0dc6712e775/writable_git_paths.json"),
            b"[]",
        )
        .expect("frozen face cache");
        std::fs::create_dir_all(base.join("policy/project_0001/x")).expect("policy dir");
        std::fs::write(base.join("policy/project_0001/x/2"), b"policy-body").expect("policy");
        std::fs::write(base.join("AGENTS.md"), b"# root rules\n").expect("root rule");

        let face = task11_root_metadata_face(base).expect("root/metadata face");
        let rels: Vec<&str> = face.entries.iter().map(|(rel, _)| rel.as_str()).collect();
        assert_eq!(
            rels,
            vec!["AGENTS.md", "policy/project_0001/x/2"],
            "成员工作树(alpha/beta)、.codegraph 与 .provider-session-cache\
             (产品 F-17 冻结面自管缓存,r63 kimi 现场漂移物)必须排除,其余逐条目实名"
        );
        assert!(face.digest.starts_with("sha256:"), "面聚合 digest 形态");
        let policy_line = &face.entries[1].1;
        assert!(
            policy_line.starts_with("policy/project_0001/x/2:11:"),
            "文件条目必须携带 len 与内容摘要:{policy_line}"
        );
    }

    /// r63 kimi 现场回归锚点:kimi LC Executor 会话构造时产品 F-17 冻结面
    /// 落盘 `.provider-session-cache/<key>/writable_git_paths.json`(D4 窗口内
    /// fingerprint_drift 场景的真实 spawn 触发)——排除语义生效后,该文件
    /// 出现前后 root/metadata 面 digest 必须不变(漂移物已按产品自管面排除)。
    #[test]
    fn lcg_d4_root_metadata_face_stable_across_provider_session_cache_creation() {
        let root = TempDir::new().expect("aggregate root");
        let base = root.path();
        std::fs::create_dir_all(base.join("policy")).expect("policy dir");
        std::fs::write(base.join("policy/body"), b"fixture").expect("policy body");
        let before = task11_root_metadata_face(base).expect("before");
        let cache = base.join(".provider-session-cache/66f7c0dc6712e775");
        std::fs::create_dir_all(&cache).expect("frozen face cache dir");
        std::fs::write(
            cache.join("writable_git_paths.json"),
            br#"["/tmp/.tmppTFuiU/alpha/.git"]"#,
        )
        .expect("frozen face cache file");
        let after = task11_root_metadata_face(base).expect("after");
        assert_eq!(
            before.digest, after.digest,
            "产品 F-17 冻结缓存落盘不得漂移 root/metadata 面(r63 kimi 现场)"
        );
        assert!(
            after
                .entries
                .iter()
                .all(|(rel, _)| !rel.starts_with(".provider-session-cache")),
            "冻结缓存条目不得进入面清单:{:?}",
            after.entries
        );
    }

    /// mtime-only 变化(同内容重写)不得改变面 digest——「恢复」语义的
    /// 判定基础:失败场景受控突变+字节级恢复后,root/metadata 面必须收敛。
    #[test]
    fn lcg_d4_root_metadata_face_digest_stable_across_mtime_rewrite() {
        let root = TempDir::new().expect("aggregate root");
        let file = root.path().join("policy/body");
        std::fs::create_dir_all(root.path().join("policy")).expect("policy dir");
        std::fs::write(&file, b"stable-bytes").expect("policy body");
        let before = task11_root_metadata_face(root.path()).expect("before");
        std::fs::write(&file, b"stable-bytes").expect("rewrite same bytes (mtime drift)");
        let after = task11_root_metadata_face(root.path()).expect("after");
        assert_eq!(
            before.digest, after.digest,
            "同内容重写(mtime-only)不得漂移 root/metadata 面"
        );
        std::fs::write(&file, b"mutated-bytes").expect("real content drift");
        let drifted = task11_root_metadata_face(root.path()).expect("drifted");
        assert_ne!(
            before.digest, drifted.digest,
            "真实内容变化必须改变面 digest"
        );
    }

    /// 漂移差分必须三列实名(kimi r63 现场:combined 漂移无实径)——
    /// added/removed/changed 逐路径,changed 携带前后条目原文。
    #[test]
    fn lcg_d4_root_metadata_entry_diff_names_added_removed_changed() {
        let before = vec![
            ("policy/a".to_string(), "policy/a:1:aaa".to_string()),
            ("policy/b".to_string(), "policy/b:2:bbb".to_string()),
        ];
        let after = vec![
            ("policy/a".to_string(), "policy/a:9:zzz".to_string()),
            ("policy/c".to_string(), "policy/c:3:ccc".to_string()),
        ];
        let diff = task11_root_metadata_entry_diff(&before, &after);
        assert_eq!(diff["added"], json!(["policy/c"]), "新增实名");
        assert_eq!(diff["removed"], json!(["policy/b"]), "移除实名");
        assert_eq!(
            diff["changed"],
            json!([{
                "path": "policy/a",
                "before": "policy/a:1:aaa",
                "after": "policy/a:9:zzz",
            }]),
            "变更实名并携带前后条目原文"
        );
    }

    /// denied_reason 摘要有界:每列至多列 8 条实名+总数,防清单爆破。
    #[test]
    fn lcg_d4_drift_summary_bounds_listed_paths() {
        let added: Vec<Value> = (0..10)
            .map(|index| json!(format!("rogue/extra-{index}")))
            .collect();
        let diff = json!({"added": added, "removed": [], "changed": []});
        let summary = task11_root_metadata_drift_summary(&diff);
        assert!(
            summary.contains("added=10[\"rogue/extra-0\""),
            "摘要必须带总数与实名前缀:{summary}"
        );
        assert!(
            summary.contains("rogue/extra-7"),
            "摘要列至多 8 条(0..=7):{summary}"
        );
        assert!(
            !summary.contains("rogue/extra-8") && !summary.contains("rogue/extra-9"),
            "超过 8 条不列名(只留总数):{summary}"
        );
        assert!(
            summary.contains("removed=0[]") && summary.contains("changed=0[]"),
            "空列零计数:{summary}"
        );
    }
}
