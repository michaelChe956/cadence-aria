use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::{JsonRpcPeer, OutboundIdNamespace};
use crate::cross_cutting::process_manager::{
    PROVIDER_CHILD_EXIT_STREAM_DRAIN, ProcessManager, SessionTailExit, bounded_child_wait,
    bounded_task_join, provider_child_exit_error, run_session_tail_with_exit_watch,
};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    LcVersionCache, LcVersionObservation, LcVersionProbe, LcVersionProbeKey, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderSession, ProviderStatus, ProviderVersionSupplier, StreamingProviderAdapter,
    StreamingProviderInput, canonical_tool_policy, validate_tool_policy_for_role,
};
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, LcProjectionAudit, ProviderStartAudit,
};
use crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector as _;
use crate::product::models::ProviderName;

mod parse;
mod projection;
mod response;
pub(crate) mod session;
mod support;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;
pub use projection::CodexPolicyProjector;
pub(crate) use response::*;
pub(crate) use support::*;
pub(crate) const CODEX_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Codex app-server 默认 sandbox 模式(T12,REQ-LCG-04 改版):
/// `danger-full-access` 是 direct Coder 与 LC 全相位的当前执行基线
/// (LC 投影的 `CodexSandboxMode::DangerFullAccess` 同值;旧 gateway 路由
/// 阻断与受限档已退役)。
pub const CODEX_DEFAULT_SANDBOX_MODE: &str = "danger-full-access";
pub(crate) const CODEX_RESUME_STALL_ERROR: &str = "Codex resume stalled before provider progress";

/// supplier seam 的 TTL key 实例范围序列(从 1 起;0 保留给真实 CLI 面)。
static LC_SUPPLIER_SCOPE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Task 3.4(B0c):LC validated start 的 spawn 前 facts 复验——冻结 full 投影
/// 的 exact version 与当前 TTL 观测不一致时的稳定判别码(child 前拒绝,
/// 零 adapter start 足迹/零 child)。
pub(crate) const CODEX_LC_PREPARED_VERSION_DRIFT: &str = "codex_lc_prepared_version_drift";

pub(crate) fn is_resume_stall_failure(message: &str) -> bool {
    message.contains(CODEX_RESUME_STALL_ERROR)
}

#[cfg(not(test))]
pub(crate) const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(crate) const CODEX_RESUME_STALL_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub struct CodexProvider {
    command: PathBuf,
    /// 策略会话 provider 版本 supplier（测试 seam；3.3 接线真实 CLI 探测后保留）。
    version_supplier: Option<ProviderVersionSupplier>,
    /// Task 3.4(B0c):LC 权威版本观测缓存(60 秒 TTL/single-flight,设计
    /// §5.4;与 direct 路径的无过期 `cached_cli_version` 严格分离)。
    lc_version_cache: LcVersionCache,
    /// TTL key 的 supplier 实例范围(测试 supplier 与真实 CLI 不串值;真实
    /// CLI 恒 0,supplier 每实例唯一——同 CLI 不同 supplier 实例互不命中)。
    lc_supplier_scope: u64,
}

impl std::fmt::Debug for CodexProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexProvider")
            .field("command", &self.command)
            .field(
                "version_supplier",
                &self
                    .version_supplier
                    .as_ref()
                    .map(|_| "<provider-version-supplier>"),
            )
            .finish()
    }
}

/// codex CLI `--version` 有界探测超时（GC9）。
pub const CODEX_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// codex CLI `--version` 有界探测（Task 3.3）：成功非空返回精确字符串；
/// 空输出/命令失败 → `Unavailable`；超时 → `Timeout`。
pub async fn probe_codex_version(
    command: &std::path::Path,
    timeout: Duration,
) -> Result<String, crate::cross_cutting::streaming_provider::VersionProbeError> {
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandRequest, TokioBoundedCommandRunner,
    };
    use crate::cross_cutting::streaming_provider::VersionProbeError;
    use tokio_util::sync::CancellationToken;

    let request = BoundedCommandRequest {
        executable: command.to_string_lossy().into_owned(),
        argv: vec!["--version".to_string()],
        working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        timeout,
        cancellation: CancellationToken::new(),
        // 批2 Task 3 修复轮(F2):探测环境显式携带宿主 PATH(平台观察
        // 语义,provider_health::probe_provider 同款)——裸名 CLI 不因
        // env_clear 后缺 PATH 而恒 Unavailable。
        environment: crate::cross_cutting::provider_environment::platform_observation_environment(),
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
    };
    match TokioBoundedCommandRunner.run_inherited(request).await {
        Ok(result) if result.timed_out => Err(VersionProbeError::Timeout),
        Ok(result) if result.exit_code == Some(0) => {
            let version = result.stdout.trim();
            if version.is_empty() {
                Err(VersionProbeError::Unavailable)
            } else {
                Ok(version.to_string())
            }
        }
        Ok(_) | Err(_) => Err(VersionProbeError::Unavailable),
    }
}

impl CodexProvider {
    pub fn new(command: PathBuf) -> Self {
        Self {
            command,
            version_supplier: None,
            lc_version_cache: LcVersionCache::new(),
            lc_supplier_scope: 0,
        }
    }

    /// 注入策略会话 provider 版本 supplier（fixture/测试 seam）。每次注入
    /// 领取唯一 supplier scope,确保不同 supplier 实例的 TTL 观测互不串值。
    pub fn with_version_supplier(mut self, supplier: ProviderVersionSupplier) -> Self {
        self.version_supplier = Some(supplier);
        self.lc_supplier_scope = LC_SUPPLIER_SCOPE_SEQ.fetch_add(1, Ordering::SeqCst);
        self
    }

    fn build_args(&self) -> Vec<String> {
        vec![
            "app-server".to_string(),
            "--enable".to_string(),
            "default_mode_request_user_input".to_string(),
        ]
    }

    /// LC 版本观测 key(Task 3.4/B0c):真实 CLI 以命令路径+可执行身份
    /// (size+mtime,二进制替换即换 key)区分;supplier seam 以稳定标记+
    /// 实例 scope 区分——测试 supplier 与真实 CLI 不串值(设计 §5.4)。
    fn lc_version_probe_key(&self) -> LcVersionProbeKey {
        LcVersionProbeKey {
            provider: ProviderName::Codex,
            canonical_executable: self.command.clone(),
            executable_identity: self.lc_executable_identity(),
            argv: vec!["--version".to_string()],
            supplier_scope: self.lc_supplier_scope,
        }
    }

    fn lc_executable_identity(&self) -> String {
        if self.version_supplier.is_some() {
            return "codex-version-supplier-seam".to_string();
        }
        match std::fs::metadata(&self.command) {
            Ok(metadata) => {
                let modified_ns = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0);
                format!("len={},mtime_ns={}", metadata.len(), modified_ns)
            }
            Err(_) => "unresolved".to_string(),
        }
    }

    /// LC 权威版本观测(Task 3.4/B0c):走 [`LcVersionCache`] 60 秒
    /// TTL/single-flight 合同(supplier seam 优先,默认真实 `--version` 有界
    /// 探测)。prepare facts 与 LC validated start 的 spawn 前复验共用本
    /// 入口(不双矩阵);direct 路径的无过期 `cached_cli_version` 不动。
    async fn observe_lc_exact_version(
        &self,
        cancel: CancellationToken,
    ) -> Result<LcVersionObservation, crate::cross_cutting::streaming_provider::VersionProbeError>
    {
        let key = self.lc_version_probe_key();
        let command = self.command.clone();
        let supplier = self.version_supplier.clone();
        let probe: LcVersionProbe = std::sync::Arc::new(move |_key| match supplier.clone() {
            Some(supplier) => Box::pin(async move { supplier() }),
            None => {
                let command = command.clone();
                Box::pin(
                    async move { probe_codex_version(&command, CODEX_VERSION_PROBE_TIMEOUT).await },
                )
            }
        });
        self.lc_version_cache
            .cached_lc_cli_version(key, probe, cancel)
            .await
    }

    /// LC 版本观测不可得的 fail-closed 映射(T12):danger 是当前合法基线,
    /// 版本不可得是普通准入失败——沿 LC validated 错误前缀,不再映射到废止
    /// 的 `codex_danger_full_access_unsupported` 稳定码(缺版本=0 会话)。
    fn lc_version_unavailable(
        error: crate::cross_cutting::streaming_provider::VersionProbeError,
    ) -> ProviderAdapterError {
        ProviderAdapterError::parse_error(
            format!("codex lc validated start: {error}"),
            String::new(),
            String::new(),
        )
    }
}

impl CodexProvider {
    /// `start_validated` 的真实执行体(gateway 路由级 Codex 阻断迁移到 LC
    /// projection 判定归 Task 3/8;此前真实 gateway validate 无法为 Codex
    /// 产出 validated policy,tests 子模块直接驱动本私有方法锁定受限分支)。
    ///
    /// 顺序契约(T12,REQ-LCG-04 改版):角色守卫 → adapter 匹配 → audit
    /// sink → exact version → danger 投影 → 以 canonical root 为进程
    /// cwd spawn → LC 握手(协议 params 来自投影)→ provider_start 统一审计
    /// (`lc_projection` 必填)→ 会话循环。
    #[cfg(test)]
    async fn start_lc_validated(
        &self,
        input: StreamingProviderInput,
        envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
        capability_snapshot_ref: &str,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 既有测试驱动的 Normal 相位入口(Task 7 保持签名零变化;裸 validate
        // 无 gateway 冻结候选投影,frozen=None 走 adapter 侧自建投影)。
        self.start_lc_validated_with_phase(
            input,
            envelope,
            capability_snapshot_ref,
            false,
            None,
            cancel,
        )
        .await
    }

    /// `start_validated` 的相位感知执行体(Task 7):`root_recipe_phase`
    /// 来自 validated policy 的冻结相位,供共用 LC guard 消费 marker 语义。
    /// Task 3.4(B0c):`frozen_lc_projection` 为 gateway full 上下文路径
    /// (`resume_or_start_with_projection` Resume / prepared candidate)冻结的
    /// 候选投影——`Some` 时直接消费(spawn 前复验冻结版本,audit 与 prepared
    /// full 投影同源);`None`(裸 validate/既有测试)沿 adapter 侧自建投影。
    async fn start_lc_validated_with_phase(
        &self,
        mut input: StreamingProviderInput,
        envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
        capability_snapshot_ref: &str,
        root_recipe_phase: bool,
        frozen_lc_projection: Option<
            crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
        >,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let lc_error = |details: String| {
            ProviderAdapterError::parse_error(details, String::new(), String::new())
        };

        // 1) 共用 LC run-context guard(Task 7):projection/input role 一致 +
        //    root recipe marker 相位语义 + 双向守卫,早于版本/session child
        //    (与 direct `start` 的纯双向守卫同源)。
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            root_recipe_phase,
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;

        // 2) adapter 匹配:validated input 必须是 Codex app-server(不匹配即拒,
        //    不回退其它 provider/dialect)。
        if input.provider_type != crate::protocol::contracts::ProviderType::Codex
            || envelope.provider_dialect
                != crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1
        {
            return Err(lc_error(
                "codex lc validated start: only Codex app-server launches are accepted by this adapter"
                    .to_string(),
            ));
        }

        // (T12,REQ-LCG-04 改版):旧 danger 危险门(受限写面未冻结即永久
        //  拒绝)已废止——danger 是当前合法执行基线,准入由 capability 行/
        //  trust/版本观测与角色合同裁决。

        // 3) 统一 launch audit sink:LC 会话必须绑定 run-bound sink,缺失
        //    fail-closed(1b 的 prepare 契约之前由本路径强制)。
        let sink = input.audit_sink.clone().ok_or_else(|| {
            lc_error("codex lc validated start: audit sink is required for LC launches".to_string())
        })?;

        // 4) exact version(Task 3.4/B0c:LC 权威观测走 `LcVersionCache` 60 秒
        //    TTL/single-flight,supplier seam 优先,默认真实 `--version` 有界
        //    探测;不可得 fail-closed,普通准入失败零 child)。
        let provider_version = self
            .observe_lc_exact_version(cancel.clone())
            .await
            .map_err(Self::lc_version_unavailable)?
            .exact_version()
            .to_string();

        // 5) danger 投影(T12,REQ-LCG-04 改版):gateway full 上下文路径冻结的
        //    候选投影直接消费——spawn 前复验冻结 exact version 与当前 TTL
        //    观测一致(不一致以 `CODEX_LC_PREPARED_VERSION_DRIFT` 稳定码
        //    child 前拒绝,零 ②③),sandbox/audit 与 prepared full 投影同源;
        //    裸 validate(None)沿 adapter 侧自建投影(trust/MCP 空串同样纳入
        //    session digest,装配后任一漂移都会改变 projection_digest)。
        let (lc_projection, sandbox) = match frozen_lc_projection {
            Some(frozen) => {
                if frozen.exact_version() != provider_version {
                    return Err(lc_error(format!(
                        "codex lc validated start: {}: frozen full projection expects exact version {}, observed {}; refusing to spawn",
                        CODEX_LC_PREPARED_VERSION_DRIFT,
                        frozen.exact_version(),
                        provider_version
                    )));
                }
                let projector = CodexPolicyProjector::new(frozen.exact_version().to_string());
                let sandbox = projector.sandbox_projection(&frozen);
                (frozen, sandbox)
            }
            None => {
                let boundary = projection::lc_boundary_plan(envelope);
                let projection_input =
                    crate::product::logical_codebase::provider_projection::ProviderProjectionInput::new(
                        envelope.clone(),
                        crate::product::logical_codebase::provider_gateway::ProviderRef::codex(
                            capability_snapshot_ref,
                        ),
                        envelope.action,
                        input.role.clone(),
                        input.permission_mode.clone(),
                        input.tool_policy.clone(),
                        String::new(),
                        String::new(),
                        envelope.config_artifact_ref.clone(),
                        String::new(),
                        Some(boundary),
                    );
                let projector = CodexPolicyProjector::new(provider_version.clone());
                let lc_projection = projector
                    .project(&projection_input)
                    .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;
                let sandbox = projector.sandbox_projection(&lc_projection);
                (lc_projection, sandbox)
            }
        };

        // 派生一致性防线:投影冻结的 target_root 必须等于 envelope 冻结
        // target(不一致即内部错误,fail-closed,不静默采纳)。
        if sandbox.target_root() != envelope.target.worktree.as_path() {
            return Err(lc_error(
                "codex lc validated start: projection target root disagrees with the frozen envelope target"
                    .to_string(),
            ));
        }

        // Task 9a:LC 显式 resume 在 child 前比较全 LC audit(完整字面量含
        // 投影)。存档缺 lc_projection(旧/direct 形态)或任一投影摘要漂移
        // → superseded 终止审计写被取代旧 run、零 child 拒绝(不清 resume
        // id 静默 fresh);无存档记录时依赖 native 握手确认(9b 的
        // thread/resume 应答同 id 收口),本层不清 id。
        if let Some(resume_id) = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let stored = sink.find_provider_start(resume_id).map_err(|error| {
                lc_error(format!(
                    "codex lc validated start: resume lookup failed: {error}"
                ))
            })?;
            if let Some(stored) = stored.as_ref() {
                let current = lc_start_audit_literal(envelope, &lc_projection);
                if matches!(
                    crate::cross_cutting::tool_policy_audit::resume_with_lc_start_record(
                        &stored.record,
                        &current
                    ),
                    crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                ) {
                    // P1-4 同源裁决:superseded 终止审计写入被取代旧 run。
                    crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                        sink.as_ref(),
                        stored,
                    )
                    .map_err(|error| {
                        lc_error(format!(
                            "codex lc validated start: superseded audit append failed: {error}"
                        ))
                    })?;
                    return Err(lc_error(
                        "codex lc validated start: resume audit drifted or legacy record lacks lc projection; refusing to spawn a fresh child for an explicit LC resume"
                            .to_string(),
                    ));
                }
            }
        }

        // 6) spawn:进程 cwd=投影冻结的 canonical LC root(双 cwd 合同,协议
        //    target 只进 wire params,不进进程 cwd)。
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process_cwd = sandbox.process_cwd().to_path_buf();
        let process = ProcessManager::spawn(
            &command,
            &arg_refs,
            &process_cwd,
            &input.env_vars,
            cancel.clone(),
        )
        .await?;

        // GC7:与 direct 路径同源的出站 request id typed namespace。
        let peer = JsonRpcPeer::new(process.stdout, process.stdin)
            .with_outbound_id_namespace(OutboundIdNamespace::Aria);
        let stderr = process.stderr;
        let mut child = process.child;

        // 7) LC 握手:thread/start|thread/resume 共用 danger 投影 params(协议
        //    cwd=投影冻结面,不以 raw 输入覆盖);失败终止子进程 fail-closed;
        //    LC 路径必得非空 thread id(与 direct 策略路径同源兜底)。
        //    Task 9b:resume 请求的握手失败(thread/resume 应答缺/错 id)
        //    属于已启动 child 后的 runtime 失败——kill/reap 并在错误中记录
        //    「未恢复」(显式声明 child 已启动并被终止,不伪称零 spawn),
        //    不回填请求 id、不清 id 转 fresh。
        let lc_resume_requested = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|session_id| !session_id.is_empty());
        let handshake = match session::codex_lc_session_handshake(&peer, &input, &sandbox).await {
            Ok(handshake) => handshake,
            Err(error) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(if lc_resume_requested {
                    ProviderAdapterError::parse_error(
                        format!(
                            "codex lc validated start: native resume not confirmed (session NOT resumed; the started child was killed and reaped, not a zero-spawn refusal): {}",
                            error.details
                        ),
                        String::new(),
                        String::new(),
                    )
                } else {
                    error
                });
            }
        };
        let Some(thread_id) = handshake.thread_id.clone() else {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(lc_error(
                "codex lc validated start: thread/start response missing or blank thread id"
                    .to_string(),
            ));
        };

        // 8) provider_start 统一审计:sandbox/approvalPolicy 与 wire 同源;
        //    lc_projection 必填(LC 会话不因 tool_policy=None 跳过统一 audit);
        //    boundary 引用同样从 sandbox 投影单一来源取值。
        let tool_policy_digest =
            projection::lc_tool_policy_canonical_digest(input.tool_policy.as_ref())
                .map_err(|error| lc_error(format!("codex lc validated start: {error}")))?;
        let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
            provider: session::TOOL_POLICY_PROVIDER_NAME.to_string(),
            role: crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                .to_string(),
            workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
            provider_session_id: thread_id.clone(),
            tool_policy_canonical_digest: tool_policy_digest.clone(),
            argv: args.clone(),
            sandbox: Some(sandbox.mode().wire_text().to_string()),
            approval_policy: Some(sandbox.approval_policy().to_string()),
            provider_version,
            adapter_dialect: session::CODEX_POLICY_DIALECT.to_string(),
            lc_projection: Some(LcProjectionAudit {
                action: projection::action_text(envelope.action).to_string(),
                wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect())
                    .to_string(),
                capability_projection_digest: lc_projection
                    .capability_projection_digest()
                    .to_string(),
                projection_digest: lc_projection.projection_digest().to_string(),
                boundary_evidence_ref: sandbox.boundary_evidence_ref().to_string(),
            }),
        });
        if let Err(error) = sink.append_bound(audit_event) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(lc_error(format!(
                "codex lc validated start: provider_start audit append failed: {error}"
            )));
        }

        // 10) 事件通道 + 会话循环(与 direct 策略路径同构):会话事件的 cwd
        //     报告进程 cwd=canonical root,raw working_dir 不泄漏;审批行为
        //     沿既有循环(策略会话 exec/fileChange 即时拒绝,Coder 沿
        //     ApprovalBridge 映射 Auto/Supervised)。
        input.working_dir = process_cwd.clone();
        let (event_tx, event_rx) = mpsc::channel(32);
        let bridge = ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone());
        let commands = bridge.command_sender();
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Codex provider started".to_string(),
                detail: None,
                command: None,
                cwd: Some(process_cwd.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        tokio::spawn(async move {
            let stderr_output = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
            let stderr_output_for_task = std::sync::Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            // r19 续修(fix 轮 2):同 direct 路径——exit-watch 竞速+终态 join 有界。
            let result = match run_session_tail_with_exit_watch(
                session::run_codex_session_loop(
                    peer,
                    bridge,
                    event_tx.clone(),
                    input,
                    cancel.clone(),
                    handshake,
                    Some(tool_policy_digest.clone()),
                ),
                &mut child,
                PROVIDER_CHILD_EXIT_STREAM_DRAIN,
                provider_child_exit_error,
            )
            .await
            {
                SessionTailExit::Stream(result) => result,
                SessionTailExit::ChildExitBeforeTerminal { error } => Err(error),
            };
            if result.is_err() {
                let _ = child.start_kill();
            }
            let status =
                match bounded_child_wait(&mut child, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await {
                    Some(status) => Ok(status),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "bounded child join elapsed before exit status",
                    )),
                };
            let _ = bounded_task_join(stderr_task, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await;
            if let Err(error) = result {
                let stderr =
                    support::combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Codex provider failed".to_string(),
                        detail: Some(error.details.clone()),
                        command: None,
                        cwd: None,
                        output: if stderr.trim().is_empty() {
                            None
                        } else {
                            Some(stderr.clone())
                        },
                        exit_code: None,
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: support::format_codex_failure(error.details, status, stderr),
                    })
                    .await;
            }
        });

        Ok(ProviderSession {
            native_session_id: Some(thread_id),
            events: event_rx,
            commands,
        })
    }
}

/// Task 9a:LC 启动的完整 resume 审计字面量(投影摘要 + v2 resume 材料)。
/// action evidence 摘要与 `capability_projection_digest` 同源;resume 指纹为
/// 五参 `from_envelope` 全上下文计算。
fn lc_start_audit_literal(
    envelope: &crate::product::logical_codebase::policy::SessionPolicyEnvelope,
    lc_projection: &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
) -> crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
    let git_identity =
        crate::product::logical_codebase::provider_gateway::canonical_target_git_identity(
            &envelope.target.worktree,
        );
    let action_evidence = lc_projection.capability_projection_digest().to_string();
    crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
        action: projection::action_text(envelope.action).to_string(),
        wire_dialect: projection::wire_dialect_text(lc_projection.wire_dialect()).to_string(),
        capability_projection_digest: action_evidence.clone(),
        projection_digest: lc_projection.projection_digest().to_string(),
        boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
        resume_fingerprint:
            crate::product::logical_codebase::provider_gateway::SessionResumeFingerprint::from_envelope(
                envelope,
                lc_projection,
                &action_evidence,
                &git_identity,
            )
            .digest,
        action_evidence_digest: action_evidence,
        target_git_identity: git_identity,
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CodexProvider {
    async fn start(
        &self,
        mut input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 双向 spawn 前守卫（Task 3.1）：非法角色×策略组合在创建子进程之前拒绝
        //（位于 ProcessManager::spawn 之前；策略会话的握手/审计在 Task 3.2 同样
        // 位于本守卫之后、spawn 之前/之后按契约分层）。
        validate_tool_policy_for_role(&input.role, input.tool_policy.as_ref()).map_err(
            |error| {
                ProviderAdapterError::parse_error(error.to_string(), String::new(), String::new())
            },
        )?;
        // T13 合同驱动 durable 分区（B2T11 裁定缺口补齐）：run-bound audit
        // sink 在位即统一 launch 审计（spawn 前握手 + provider_start 首行），
        // 不再以 deny intent 为门——去 deny 后的 Reviewer direct run 同样
        // 落 provider_start（ReviewerVerify 真窗/审批 durable 面归 T20）。
        // 岗位纪律会话（Author，`DenyFileWriteBuiltins` intent）必须携带
        // sink（缺失 fail-closed 保持），并叠加 canonical digest 与 resume
        // 冻结三元组；无策略合同 run 的 digest 为空串（无策略指纹）。
        // BOOT-04：`BootstrapExecutorMarker` 不进本合同（已在上方守卫完成
        // 结构复核；自举执行器的 durable 审计由 root recipe receipt 承担）。
        let mut superseded_record_missing = false;
        let discipline_policy = input.tool_policy.as_ref().is_some_and(|policy| {
            matches!(
                policy.intent,
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::DenyFileWriteBuiltins
            )
        });
        let audit_contract_sink = if discipline_policy {
            Some(input.audit_sink.clone().ok_or_else(|| {
                ProviderAdapterError::parse_error(
                    "codex policy session: audit sink is required for policy sessions",
                    String::new(),
                    String::new(),
                )
            })?)
        } else {
            input.audit_sink.clone()
        };
        let mut policy_context: Option<(
            std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
            String,
            String,
        )> = None;
        if let Some(sink) = audit_contract_sink {
            let context_label = if discipline_policy {
                "codex policy session"
            } else {
                "codex contract session"
            };
            // 版本解析（supplier seam 优先，默认真实 `--version` 探测+进程内
            // 缓存，不可得 fail-closed）：provider_start 的冻结字段。
            let provider_version = match self.version_supplier.clone() {
                Some(supplier) => supplier().map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("{context_label}: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
                None => crate::cross_cutting::streaming_provider::cached_cli_version(
                    &self.command,
                    probe_codex_version(&self.command, CODEX_VERSION_PROBE_TIMEOUT),
                )
                .await
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("{context_label}: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?,
            };
            let tool_policy_digest = if discipline_policy {
                let policy = input
                    .tool_policy
                    .as_ref()
                    .expect("discipline implies policy");
                canonical_tool_policy(session::TOOL_POLICY_PROVIDER_NAME, policy)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("{context_label}: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?
                    .digest
            } else {
                // 决策项②(T20b):非纪律合同 run 的 digest 按策略形态——
                // ReviewerVerify 携窗口域 canonical digest(与 provider_start/
                // claude/pi/kimi LC 同源可 join,换窗必失配);无策略(纯
                // sink 合同,如 T13 中间态形态)保持空串口径。
                match input.tool_policy.as_ref() {
                    Some(
                        policy @ crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                            intent:
                                crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify { .. },
                        },
                    ) => canonical_tool_policy(session::TOOL_POLICY_PROVIDER_NAME, policy)
                        .map_err(|error| {
                            ProviderAdapterError::parse_error(
                                format!("{context_label}: {error}"),
                                String::new(),
                                String::new(),
                            )
                        })?
                        .digest,
                    _ => String::new(),
                }
            };
            // resume 冻结三元组比对（digest/version/dialect）：记录缺失或任一
            // 不一致 → 追加 superseded 终止审计并新建会话，丢弃 resume id
            //（GC9：缺失与 drift 同路径处置；「标记 superseded」仅带内
            // ToolPolicyWarning——无旧文件可写，不伪造 durable 文件）。
            // 批2 Task 20（RevB2T13 ⚠4）：三元组门只服务岗位纪律会话
            //（Author deny 投影）——非策略合同 run（ReviewerVerify 直连
            // 合同）的连续性由 native thread 承担，不按策略记录门控
            //（否则 reviewer 续接被误清+误导告警，连续性丢失）。
            let resume_id = if discipline_policy {
                input
                    .resume_provider_session_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(ToString::to_string)
            } else {
                None
            };
            if let Some(resume_id) = resume_id.as_ref() {
                let stored = sink.find_provider_start(resume_id).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("{context_label}: resume lookup failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                let current = ProviderStartAudit {
                    workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                    provider_session_id: resume_id.clone(),
                    tool_policy_canonical_digest: tool_policy_digest.clone(),
                    provider_version: provider_version.clone(),
                    adapter_dialect: session::CODEX_POLICY_DIALECT.to_string(),
                    ..ProviderStartAudit::default()
                };
                if let Some(stored) = stored.as_ref()
                    && matches!(
                        crate::cross_cutting::tool_policy_audit::resume_with_audit_record(
                            Some(stored.record.clone()),
                            &current
                        ),
                        crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                    )
                {
                    // P1-4 裁决：superseded 终止审计写入被取代旧 run 的文件（其
                    // provider_start 已是首行；被终止的是旧会话），新 run 照常从
                    // provider_start 开始。
                    crate::cross_cutting::tool_policy_audit::append_superseded_policy_drift(
                        sink.as_ref(),
                        stored,
                    )
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("{context_label}: superseded audit append failed: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    input.resume_provider_session_id = None;
                } else if stored.is_none() {
                    // GC9：记录缺失与 drift 同路径处置——清除 resume id、以全新会话
                    // 启动；「标记 superseded」仅带内 ToolPolicyWarning（事件通道
                    // 建立后送出），🔴 不伪造无 provider_start 首行的 durable 文件。
                    input.resume_provider_session_id = None;
                    superseded_record_missing = true;
                }
            }
            policy_context = Some((sink, provider_version, tool_policy_digest));
        }
        let args = self.build_args();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let command = self.command.to_string_lossy().to_string();
        let process = ProcessManager::spawn(
            &command,
            &arg_refs,
            &input.working_dir,
            &input.env_vars,
            cancel.clone(),
        )
        .await?;

        // GC7：codex peer 出站 request id 使用 typed namespace `aria-<seq>`，
        // 与 server→client 入站原生数字 id 值域隔离；pi/kimi peer 保持默认 Numeric。
        let peer = JsonRpcPeer::new(process.stdout, process.stdin)
            .with_outbound_id_namespace(OutboundIdNamespace::Aria);
        let stderr = process.stderr;
        let mut child = process.child;
        // 审计合同会话（T13 合同驱动：岗位纪律/Author 或合同 bound 的无策略
        // run）：握手（initialize/initialized + thread/start|resume）从后台任务
        // 前置到 start 内有界完成；成功后、返回前写 `provider_start` 首行，
        // 握手/写失败终止子进程并 fail-closed。无合同的 Coder 路径零变化。
        let mut native_session_id = None;
        let policy_handshake = if let Some((sink, provider_version, tool_policy_digest)) =
            policy_context
        {
            let handshake = match session::codex_session_handshake(&peer, &input).await {
                Ok(handshake) => handshake,
                Err(error) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(error);
                }
            };
            let Some(thread_id) = handshake.thread_id.clone() else {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ProviderAdapterError::parse_error(
                    "codex audit contract session: thread/start response missing or blank thread id",
                    String::new(),
                    String::new(),
                ));
            };
            let launch_params = session::codex_launch_params(&input);
            let audit_event = DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: session::TOOL_POLICY_PROVIDER_NAME.to_string(),
                role: crate::cross_cutting::streaming_provider::adapter_role_text(&input.role)
                    .to_string(),
                workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                provider_session_id: thread_id.clone(),
                tool_policy_canonical_digest: tool_policy_digest,
                argv: args.clone(),
                sandbox: launch_params
                    .get("sandbox")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                approval_policy: launch_params
                    .get("approvalPolicy")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                provider_version,
                adapter_dialect: session::CODEX_POLICY_DIALECT.to_string(),
                // direct 合同会话:无 LC 投影(仅 LC validated 启动落盘)。
                lc_projection: None,
            });
            if let Err(error) = sink.append_bound(audit_event) {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ProviderAdapterError::parse_error(
                    format!(
                        "codex audit contract session: provider_start audit append failed: {error}"
                    ),
                    String::new(),
                    String::new(),
                ));
            }
            native_session_id = Some(thread_id);
            Some(handshake)
        } else {
            None
        };
        let (event_tx, event_rx) = mpsc::channel(32);
        let bridge = ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone());
        let commands = bridge.command_sender();
        let _ = event_tx
            .send(ProviderEvent::StatusChanged(ProviderStatus::Starting))
            .await;
        let _ = event_tx
            .send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider".to_string(),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Started,
                title: "Codex provider started".to_string(),
                detail: None,
                command: None,
                cwd: Some(input.working_dir.display().to_string()),
                output: None,
                exit_code: None,
            }))
            .await;

        // GC9：resume 记录缺失的带内 superseded 标记（事件通道在握手/append 之后
        // 建立，故在此送出；新会话 provider_start 已先落盘）。
        if superseded_record_missing {
            let _ = event_tx
                .send(crate::cross_cutting::streaming_provider::superseded_policy_record_missing_warning())
                .await;
        }

        tokio::spawn(async move {
            let stderr_output = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
            let stderr_output_for_task = std::sync::Arc::clone(&stderr_output);
            let stderr_task = tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut output = stderr_output_for_task.lock().await;
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&line);
                }
            });

            // r19 续修(fix 轮 2):泵与会话循环经共享 exit-watch supervisor 与
            // 子进程退出竞速;终态 join 一律有界(红线 a)。子进程先退出且
            // 排空耗尽 ⇒ 合成错误沿既有 Err 路径发终态 Failed。
            let result = match run_session_tail_with_exit_watch(
                // 双 session 入口 future 类型不同,各臂独立装箱统一(契约不变)。
                match policy_handshake {
                    Some(handshake) => Box::pin(session::run_codex_session_loop(
                        peer,
                        bridge,
                        event_tx.clone(),
                        input,
                        cancel.clone(),
                        handshake,
                        // direct 会话:决策审计 digest 按 input 实算 direct
                        // canonical(与 direct provider_start 同源)。
                        None,
                    ))
                        as std::pin::Pin<
                            Box<dyn Future<Output = Result<(), ProviderAdapterError>> + Send>,
                        >,
                    None => Box::pin(session::run_codex_session(
                        peer,
                        bridge,
                        event_tx.clone(),
                        input,
                        cancel.clone(),
                    ))
                        as std::pin::Pin<
                            Box<dyn Future<Output = Result<(), ProviderAdapterError>> + Send>,
                        >,
                },
                &mut child,
                PROVIDER_CHILD_EXIT_STREAM_DRAIN,
                provider_child_exit_error,
            )
            .await
            {
                SessionTailExit::Stream(result) => result,
                SessionTailExit::ChildExitBeforeTerminal { error } => Err(error),
            };
            if result.is_err() {
                let _ = child.start_kill();
            }
            let status =
                match bounded_child_wait(&mut child, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await {
                    Some(status) => Ok(status),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "bounded child join elapsed before exit status",
                    )),
                };
            let _ = bounded_task_join(stderr_task, PROVIDER_CHILD_EXIT_STREAM_DRAIN).await;
            if let Err(error) = result {
                let stderr =
                    support::combine_stderr(stderr_output.lock().await.clone(), error.stderr);
                let _ = event_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Failed))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "provider".to_string(),
                        kind: ProviderExecutionEventKind::Provider,
                        status: ProviderExecutionEventStatus::Failed,
                        title: "Codex provider failed".to_string(),
                        detail: Some(error.details.clone()),
                        command: None,
                        cwd: None,
                        output: if stderr.trim().is_empty() {
                            None
                        } else {
                            Some(stderr.clone())
                        },
                        exit_code: None,
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: support::format_codex_failure(error.details, status, stderr),
                    })
                    .await;
            }
        });

        Ok(ProviderSession {
            native_session_id,
            events: event_rx,
            commands,
        })
    }
    /// LC validated 启动(T12,REQ-LCG-04 改版):只接受 gateway 产出的
    /// validated input,经 `CodexPolicyProjector` 投影全相位 danger sandbox
    /// (Planning/Review/Coding wire 均 danger-full-access),以 canonical
    /// LC root 为进程 cwd、投影 protocol cwd 为协议 cwd 启动;统一落盘
    /// `ProviderStartAudit.lc_projection`(sandbox/approvalPolicy 与 wire
    /// 同源;无通用 tool policy 的角色不早退)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, launch) = validated.into_parts();
        self.start_lc_validated_with_phase(
            input,
            launch.envelope(),
            launch.capability_snapshot_ref(),
            launch.is_root_recipe_phase(),
            launch.lc_projection().cloned(),
            cancel,
        )
        .await
    }

    /// Task 3.4(B0c):本家只读 LC prepare facts——guard(role 一致)先于
    /// ①版本观测与一切 child;exact version 走 `LcVersionCache` 60 秒
    /// TTL/single-flight(supplier seam 优先);boundary 由 envelope 派生
    /// (逻辑交付面);approval 取独立映射(权限模式/Author 恒 on-request);
    /// Codex LC 无 MCP bundle(空串,与 adapter 侧投影输入同源);projector
    /// 与 exact version 同源。全程零 durable 写/零 spawn(gateway 消费本
    /// facts 装配 full 投影)。
    async fn prepare_lc_facts(
        &self,
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<ProviderLcPrepareFacts, ProviderAdapterError> {
        let lc_error = |details: String| {
            ProviderAdapterError::parse_error(details, String::new(), String::new())
        };
        let envelope = launch.envelope();

        // 1) 共用 LC run-context guard(早于 ①版本观测/child,与 start 路径
        //    同序、同源)。
        crate::cross_cutting::streaming_provider::guard_lc_validated_launch(
            launch.is_root_recipe_phase(),
            &input.role,
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| lc_error(format!("codex lc prepare facts: {error}")))?;

        // 2) adapter 匹配:只观测 Codex app-server(不匹配即拒,不回退)。
        if input.provider_type != crate::protocol::contracts::ProviderType::Codex
            || envelope.provider_dialect
                != crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1
        {
            return Err(lc_error(
                "codex lc prepare facts: only Codex app-server facts are served by this adapter"
                    .to_string(),
            ));
        }

        // 3) (T12)旧 REQ-LCG-04 危险门(受限写面未冻结即零观测零 child 拒绝)
        //    已废止——danger 是当前合法执行基线,不在此预拒。

        // 4) exact version:60 秒 TTL/single-flight(①口径;不可得
        //    fail-closed,普通准入失败零观测零 child)。
        let observation = self
            .observe_lc_exact_version(cancel)
            .await
            .map_err(Self::lc_version_unavailable)?;

        // 5) danger boundary/approval fact 与同版本 projector(readonly,零写;
        //    全相位 danger 基线,approval 按权限模式独立映射/Author 恒
        //    on-request)。
        let boundary = projection::lc_boundary_plan(envelope);
        let approval_policy =
            projection::lc_approval_policy(input.tool_policy.as_ref(), &input.permission_mode);

        Ok(ProviderLcPrepareFacts::new(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::Codex,
            envelope.provider_dialect,
            crate::product::logical_codebase::policy::ProviderWireDialect::CodexAppServerRpc,
            observation.exact_version().to_string(),
            observation.observed_at(),
            self.lc_version_probe_key(),
            approval_policy,
            String::new(),
            boundary,
            std::sync::Arc::new(CodexPolicyProjector::new(
                observation.exact_version().to_string(),
            )),
        ))
    }
}
