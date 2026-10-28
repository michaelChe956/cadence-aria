// ---------------------------------------------------------------------------
// Task 3.7(B0f):四 provider 公共汇合——full 唯一 start 路径与跨家组合守卫。
// `anchor_t10_support` 为 pub(crate):lifecycle_store 的 durable 同源测试
// 复用同一四家 fixture(仅测试构建可达)。
// ---------------------------------------------------------------------------

//! 四家形态参数(消费 3.3–3.6 各家已过的本家事实语义,公共行为断言归
//! 本包):Claude `--resume` argv、Codex 受限 sandbox/approval、Pi
//! extension 写时机、Kimi native-source 与 Aria MCP bundle 区分。
use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    LcVersionCache, LcVersionProbe, LcVersionProbeKey, StreamingProviderAdapter,
};
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, LcProjectionAudit, LcProviderStartAudit, ProviderStartAudit,
    StoredProviderStart, ToolPolicyAuditSink, append_superseded_policy_drift,
    resume_with_lc_start_record,
};
use crate::product::logical_codebase::policy::{ProviderDialect, ProviderWireDialect};
use crate::product::logical_codebase::provider_projection::{
    ProviderLcPrepareFacts, ProviderPolicyProjection, ProviderPolicyProjector,
    ProviderProjectionError, ProviderProjectionInput, UnprovisionedProviderPolicyProjector,
};
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustFingerprintFacts, ProviderTrustSource, ProviderTrustVerification,
};
use crate::product::models::ProviderName;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 四家公共 family 形态(版本/exe 身份/approval/MCP/audit 文本各随本家)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnchorT10Family {
    Claude,
    Codex,
    Pi,
    Kimi,
}

impl AnchorT10Family {
    pub(crate) const ALL: [AnchorT10Family; 4] = [
        AnchorT10Family::Claude,
        AnchorT10Family::Codex,
        AnchorT10Family::Pi,
        AnchorT10Family::Kimi,
    ];

    pub(crate) fn tag(&self) -> &'static str {
        match self {
            AnchorT10Family::Claude => "claude",
            AnchorT10Family::Codex => "codex",
            AnchorT10Family::Pi => "pi",
            AnchorT10Family::Kimi => "kimi",
        }
    }

    pub(crate) fn name(&self) -> ProviderName {
        match self {
            AnchorT10Family::Claude => ProviderName::ClaudeCode,
            AnchorT10Family::Codex => ProviderName::Codex,
            AnchorT10Family::Pi => ProviderName::Pi,
            AnchorT10Family::Kimi => ProviderName::KimiCode,
        }
    }

    fn ref_type(&self) -> ProviderRefType {
        match self {
            AnchorT10Family::Claude => ProviderRefType::ClaudeCode,
            AnchorT10Family::Codex => ProviderRefType::Codex,
            AnchorT10Family::Pi => ProviderRefType::Pi,
            AnchorT10Family::Kimi => ProviderRefType::KimiCode,
        }
    }

    fn provider_ref(&self) -> ProviderRef {
        match self {
            AnchorT10Family::Claude => ProviderRef::claude_code("cap_claude_t10"),
            AnchorT10Family::Codex => ProviderRef::codex("cap_codex_t10"),
            AnchorT10Family::Pi => ProviderRef::pi("cap_pi_t10"),
            AnchorT10Family::Kimi => ProviderRef::kimi_code("cap_kimi_t10"),
        }
    }

    fn dialect(&self) -> ProviderDialect {
        match self {
            AnchorT10Family::Claude => ProviderDialect::ClaudeCodeCliV1,
            AnchorT10Family::Codex => ProviderDialect::CodexCliV1,
            AnchorT10Family::Pi => ProviderDialect::PiRpcV1,
            AnchorT10Family::Kimi => ProviderDialect::KimiAcpV1,
        }
    }

    fn wire(&self) -> ProviderWireDialect {
        match self {
            AnchorT10Family::Claude => ProviderWireDialect::ClaudeCodeStreamJson,
            AnchorT10Family::Codex => ProviderWireDialect::CodexAppServerRpc,
            AnchorT10Family::Pi => ProviderWireDialect::PiRpc,
            AnchorT10Family::Kimi => ProviderWireDialect::KimiAcp,
        }
    }

    pub(crate) fn wire_text(&self) -> &'static str {
        match self {
            AnchorT10Family::Claude => "claude-stream-json",
            AnchorT10Family::Codex => "codex-app-server-rpc",
            AnchorT10Family::Pi => "pi-rpc",
            AnchorT10Family::Kimi => "kimi-acp",
        }
    }

    fn protocol_type(&self) -> crate::protocol::contracts::ProviderType {
        match self {
            AnchorT10Family::Claude => crate::protocol::contracts::ProviderType::ClaudeCode,
            AnchorT10Family::Codex => crate::protocol::contracts::ProviderType::Codex,
            AnchorT10Family::Pi => crate::protocol::contracts::ProviderType::Pi,
            AnchorT10Family::Kimi => crate::protocol::contracts::ProviderType::KimiCode,
        }
    }

    pub(crate) fn exact_version(&self) -> &'static str {
        match self {
            AnchorT10Family::Claude => "claude 1.4.0-t10",
            AnchorT10Family::Codex => "codex 0.9.4-t10",
            AnchorT10Family::Pi => "pi 0.83.0-t10",
            AnchorT10Family::Kimi => "kimi 0.34.0-t10",
        }
    }

    fn base_executable_identity(&self) -> String {
        format!("sha256:t10-{}-exe-v1", self.tag())
    }

    fn canonical_executable(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/opt/anchor-t10/{}/cli", self.tag()))
    }

    pub(crate) fn approval_policy(&self) -> &'static str {
        match self {
            AnchorT10Family::Claude => "permission-prompt-tool=stdio",
            AnchorT10Family::Codex => "on-request",
            AnchorT10Family::Pi => "ask-for-extension",
            AnchorT10Family::Kimi => "kimi-acp-default",
        }
    }

    pub(crate) fn aria_mcp_bundle_digest(&self) -> &'static str {
        match self {
            AnchorT10Family::Kimi => "sha256:t10-mcp-aria-bundle",
            _ => "sha256:t10-mcp-bundle",
        }
    }

    pub(crate) fn audit_sandbox(&self) -> Option<String> {
        match self {
            AnchorT10Family::Codex => Some("danger-full-access".to_string()),
            _ => None,
        }
    }

    pub(crate) fn audit_approval(&self) -> Option<String> {
        match self {
            AnchorT10Family::Codex => Some("on-request".to_string()),
            _ => None,
        }
    }

    pub(crate) fn audit_provider_text(&self) -> &'static str {
        match self {
            AnchorT10Family::Claude => "claude-code",
            AnchorT10Family::Codex => "codex",
            AnchorT10Family::Pi => "pi",
            AnchorT10Family::Kimi => "kimi-code",
        }
    }

    /// 本家 argv 形态(claude 携 `--resume <id>`;pi 携预生成 session id;
    /// codex 携 danger sandbox/approval 片段(T12 基线);kimi 携 acp 冻结片段)。
    fn audit_argv(&self, native_id: &str, resume_id: Option<&str>) -> Vec<String> {
        match self {
            AnchorT10Family::Claude => {
                let mut argv = vec![self.tag().to_string()];
                if let Some(id) = resume_id {
                    argv.push("--resume".to_string());
                    argv.push(id.to_string());
                }
                argv
            }
            AnchorT10Family::Codex => vec![
                self.tag().to_string(),
                "--sandbox".to_string(),
                "danger-full-access".to_string(),
                "--ask".to_string(),
                "on-request".to_string(),
            ],
            AnchorT10Family::Pi => vec![
                self.tag().to_string(),
                "--session-id".to_string(),
                native_id.to_string(),
            ],
            AnchorT10Family::Kimi => {
                vec![self.tag().to_string(), "acp".to_string()]
            }
        }
    }
}

fn anchor_t10_digest(parts: &[&str]) -> String {
    use sha2::Digest as _;
    let mut canonical = String::new();
    for part in parts {
        canonical.push_str(&format!("{}:{}", part.len(), part));
    }
    format!("sha256:{:x}", sha2::Sha256::digest(canonical.as_bytes()))
}

/// t10 四家 capability source:version/exact_version 同源,action 行
/// `projection_digest` 与 t10 projector 的 capability 摘要逐字相等
/// (`projection-digest-{action:?}`,与 StaticCapabilitySource 同形)。
pub(crate) struct AnchorT10CapabilitySource {
    family: AnchorT10Family,
}

impl AnchorT10CapabilitySource {
    pub(crate) fn new(family: AnchorT10Family) -> Self {
        Self { family }
    }

    fn capability(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> ProviderCapability {
        ProviderCapability {
            provider_type: self.family.ref_type(),
            version: self.family.exact_version().to_string(),
            adapter_dialect: self.family.dialect(),
            wire_dialect: self.family.wire(),
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability:
                crate::product::logical_codebase::provider_capability_store::ProviderActionCapability {
                    action,
                    launch: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
                    resume: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
                    write_boundary: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
                    projection_digest: format!("projection-digest-{action:?}"),
                    evidence_ref: format!("probe://t10-{action:?}"),
                },
            trust: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for AnchorT10CapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "anchor t10 fixture has no root recipe facts".to_string(),
        ))
    }
}

/// t10 可翻转只读 trust source:verify_trusted 与 fingerprint_facts 同源
/// 翻转(trust spawn 前变化负例);Codex/Kimi 需 workspace trust 的家族
/// 真实触达该 source。
pub(crate) struct AnchorT10TrustSource {
    trusted: AtomicBool,
}

impl AnchorT10TrustSource {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            trusted: AtomicBool::new(true),
        })
    }

    pub(crate) fn set_trusted(&self, trusted: bool) {
        self.trusted.store(trusted, Ordering::SeqCst);
    }

    fn verification(
        &self,
        provider: &ProviderName,
        canonical_root: &std::path::Path,
    ) -> ProviderTrustVerification {
        ProviderTrustVerification {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key: String::new(),
            trusted: self.trusted.load(Ordering::SeqCst),
            ownership: None,
            detail: String::new(),
            verified_at: String::new(),
        }
    }
}

impl ProviderTrustSource for AnchorT10TrustSource {
    fn verify_trusted(
        &self,
        _project_id: &str,
        _lc_id: &str,
        provider: &ProviderName,
        canonical_root: &std::path::Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError> {
        Ok(self.verification(provider, canonical_root))
    }

    fn fingerprint_facts(
        &self,
        _project_id: &str,
        _lc_id: &str,
        provider: &ProviderName,
        canonical_root: &std::path::Path,
    ) -> Result<ProviderTrustFingerprintFacts, ProviderGatewayError> {
        if !self.trusted.load(Ordering::SeqCst) {
            return Err(ProviderGatewayError::ProviderUnavailable(
                "anchor t10 trust source: entry not trusted; refusing to freeze facts".to_string(),
            ));
        }
        Ok(ProviderTrustFingerprintFacts {
            verification: self.verification(provider, canonical_root),
            record_digest: Some(format!(
                "sha256:t10-trust-record-{}",
                provider_wire_tag(provider)
            )),
        })
    }
}

fn provider_wire_tag(provider: &ProviderName) -> String {
    format!("{provider:?}").to_lowercase()
}

/// t10 版本化 projector:capability 摘要与 capability 行逐字相等
/// (`projection-digest-{action:?}`);会话摘要为长度分隔+schema 前缀的
/// SHA-256(family/version/trust/MCP/approval/cwd/config 全部入摘要);
/// 负例可覆写 capability/会话摘要(v2 材料完整性门)。
pub(crate) struct AnchorT10Projector {
    family: AnchorT10Family,
    exact_version: String,
    capability_digest_override: Option<String>,
    session_digest_override: Option<String>,
}

impl AnchorT10Projector {
    fn new(family: AnchorT10Family, exact_version: String) -> Self {
        Self {
            family,
            exact_version,
            capability_digest_override: None,
            session_digest_override: None,
        }
    }
}

impl ProviderPolicyProjector for AnchorT10Projector {
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        let envelope = input.envelope();
        // 匹配门:只投影本家(跨家投影拒绝,与真实 projector 同构)。
        if input.provider().provider_type != self.family.ref_type() {
            return Err(ProviderProjectionError::Unsupported(format!(
                "anchor t10 projector only projects {} (got {:?})",
                self.family.tag(),
                input.provider().provider_type
            )));
        }
        if envelope.provider_dialect != self.family.dialect() {
            return Err(ProviderProjectionError::Unsupported(format!(
                "anchor t10 projector requires dialect {:?}",
                self.family.dialect()
            )));
        }
        // read-only action 的 boundary 证据引用为空串(合法形态),
        // 不消费 facts 边界计划。
        let capability_projection_digest = self
            .capability_digest_override
            .clone()
            .unwrap_or_else(|| format!("projection-digest-{:?}", envelope.action));
        let session_projection_digest = self.session_digest_override.clone().unwrap_or_else(|| {
            anchor_t10_digest(&[
                "anchor-t10-session-v1",
                self.family.tag(),
                self.exact_version.as_str(),
                capability_projection_digest.as_str(),
                input.trust_digest(),
                input.mcp_bundle_digest(),
                input.approval_policy(),
                &envelope.working_directory.to_string_lossy(),
                &envelope.config_digest,
            ])
        });
        Ok(ProviderPolicyProjection::new(
            self.family.ref_type(),
            envelope.provider_dialect,
            self.family.wire(),
            self.exact_version.clone(),
            envelope.action,
            input.role().clone(),
            input.permission_mode().clone(),
            input.tool_policy().cloned(),
            input.approval_policy().to_string(),
            self.family
                .audit_sandbox()
                .unwrap_or_else(|| "read-only".to_string()),
            envelope.working_directory.clone(),
            envelope.working_directory.clone(),
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            input.trust_digest().to_string(),
            envelope.config_digest.clone(),
            input.mcp_bundle_digest().to_string(),
            String::new(),
            capability_projection_digest,
            session_projection_digest,
        ))
    }
}

/// t10 四家 fake streaming adapter:真实 prepare 链(TTL 缓存+计数
/// probe=口径①)+start_validated 消费冻结 full projection(口径②)并
/// 如实计数 native child(口径③)与 kill/reap;basis 旁路(裸投影
/// validated)typed 拒;audit 字面量与 gateway 冻结 full projection
/// 同源;pi extension 写只在 mismatch 判定之后;kimi 复验 MCP 来源。
pub(crate) struct AnchorT10FamilyAdapter {
    family: AnchorT10Family,
    cache: Arc<LcVersionCache>,
    probe_calls: Arc<AtomicUsize>,
    executable_identity: Mutex<String>,
    observed_version: Arc<Mutex<String>>,
    mcp_bundle_digest: Mutex<String>,
    capability_digest_override: Mutex<Option<String>>,
    session_digest_override: Mutex<Option<String>>,
    handshake_native_id: Mutex<Option<String>>,
    adapter_starts: AtomicUsize,
    session_children: AtomicUsize,
    children_killed: AtomicUsize,
    extension_writes: AtomicUsize,
    starts_without_projection: AtomicUsize,
    last_start_projection_digest: Mutex<Option<String>>,
    last_start_audit_literal: Mutex<Option<LcProviderStartAudit>>,
}

impl AnchorT10FamilyAdapter {
    pub(crate) fn new(family: AnchorT10Family, cache: Arc<LcVersionCache>) -> Arc<Self> {
        Arc::new(Self {
            family,
            cache,
            probe_calls: Arc::new(AtomicUsize::new(0)),
            executable_identity: Mutex::new(family.base_executable_identity()),
            observed_version: Arc::new(Mutex::new(family.exact_version().to_string())),
            mcp_bundle_digest: Mutex::new(family.aria_mcp_bundle_digest().to_string()),
            capability_digest_override: Mutex::new(None),
            session_digest_override: Mutex::new(None),
            handshake_native_id: Mutex::new(None),
            adapter_starts: AtomicUsize::new(0),
            session_children: AtomicUsize::new(0),
            children_killed: AtomicUsize::new(0),
            extension_writes: AtomicUsize::new(0),
            starts_without_projection: AtomicUsize::new(0),
            last_start_projection_digest: Mutex::new(None),
            last_start_audit_literal: Mutex::new(None),
        })
    }

    pub(crate) fn set_executable_identity(&self, identity: &str) {
        *self.executable_identity.lock().unwrap() = identity.to_string();
    }

    pub(crate) fn set_observed_version(&self, version: &str) {
        *self.observed_version.lock().unwrap() = version.to_string();
    }

    pub(crate) fn set_mcp_bundle_digest(&self, digest: &str) {
        *self.mcp_bundle_digest.lock().unwrap() = digest.to_string();
    }

    pub(crate) fn set_projector_overrides(
        &self,
        capability: Option<String>,
        session: Option<String>,
    ) {
        *self.capability_digest_override.lock().unwrap() = capability;
        *self.session_digest_override.lock().unwrap() = session;
    }

    pub(crate) fn set_handshake_native_id(&self, id: Option<String>) {
        *self.handshake_native_id.lock().unwrap() = id;
    }

    pub(crate) fn probe_calls(&self) -> usize {
        self.probe_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn adapter_starts(&self) -> usize {
        self.adapter_starts.load(Ordering::SeqCst)
    }

    pub(crate) fn session_children(&self) -> usize {
        self.session_children.load(Ordering::SeqCst)
    }

    pub(crate) fn children_killed(&self) -> usize {
        self.children_killed.load(Ordering::SeqCst)
    }

    pub(crate) fn extension_writes(&self) -> usize {
        self.extension_writes.load(Ordering::SeqCst)
    }

    pub(crate) fn starts_without_projection(&self) -> usize {
        self.starts_without_projection.load(Ordering::SeqCst)
    }

    pub(crate) fn last_start_projection_digest(&self) -> Option<String> {
        self.last_start_projection_digest.lock().unwrap().clone()
    }

    pub(crate) fn last_start_audit_literal(&self) -> Option<LcProviderStartAudit> {
        self.last_start_audit_literal.lock().unwrap().clone()
    }

    fn minimal_session(
        native_id: Option<String>,
    ) -> crate::cross_cutting::streaming_provider::ProviderSession {
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: native_id,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AnchorT10FamilyAdapter {
    async fn start_validated(
        &self,
        input: ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
    {
        self.adapter_starts.fetch_add(1, Ordering::SeqCst);
        let (input, launch) = input.into_parts();
        // basis 旁路不可达:唯一 start 路径必须携带冻结 full projection。
        let Some(lc_projection) = launch.lc_projection().cloned() else {
            self.starts_without_projection
                .fetch_add(1, Ordering::SeqCst);
            return Err(ProviderAdapterError::parse_error(
                "anchor t10 adapter: bare-basis validated input reached start_validated \
                 (no frozen full projection); refusing to spawn"
                    .to_string(),
                String::new(),
                String::new(),
            ));
        };
        let envelope = launch.envelope();
        // 本家 mutable facts 复验(与真实 adapter 同构:version 恒复验,
        // kimi 追加 MCP 来源复验)。
        let observed_version = self.observed_version.lock().unwrap().clone();
        if lc_projection.exact_version() != observed_version.as_str() {
            return Err(ProviderAdapterError::parse_error(
                format!(
                    "anchor t10 {}: prepared projection version {} drifted from current \
                     observation {observed_version}; refusing to spawn",
                    self.family.tag(),
                    lc_projection.exact_version()
                ),
                String::new(),
                String::new(),
            ));
        }
        if self.family == AnchorT10Family::Kimi
            && lc_projection.mcp_bundle_digest() != *self.mcp_bundle_digest.lock().unwrap()
        {
            return Err(ProviderAdapterError::parse_error(
                format!(
                    "anchor t10 kimi: mcp source drifted before spawn (frozen={}, current={})",
                    lc_projection.mcp_bundle_digest(),
                    self.mcp_bundle_digest.lock().unwrap()
                ),
                String::new(),
                String::new(),
            ));
        }
        // audit 字面量(与真实 lc_start_audit_literal 同形):从冻结
        // full projection 装配,v2 材料不完整即拒。
        let git_identity = canonical_target_git_identity(&envelope.target.worktree);
        let action_evidence = lc_projection.capability_projection_digest().to_string();
        let literal = LcProviderStartAudit {
            action: "planning_read_only".to_string(),
            wire_dialect: self.family.wire_text().to_string(),
            capability_projection_digest: action_evidence.clone(),
            projection_digest: lc_projection.projection_digest().to_string(),
            boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
            resume_fingerprint: SessionResumeFingerprint::from_envelope(
                envelope,
                &lc_projection,
                &action_evidence,
                &git_identity,
            )
            .digest,
            action_evidence_digest: action_evidence,
            target_git_identity: git_identity,
        };
        if !literal.is_v2_complete() {
            return Err(ProviderAdapterError::parse_error(
                "anchor t10 adapter: lc start audit v2 material incomplete".to_string(),
                String::new(),
                String::new(),
            ));
        }
        *self.last_start_projection_digest.lock().unwrap() =
            Some(lc_projection.projection_digest().to_string());
        *self.last_start_audit_literal.lock().unwrap() = Some(literal.clone());
        // 显式 resume:先对存档完整字面量判定(漂移 → superseded 终止
        // 审计写被取代旧 run,零 child typed 拒)。
        let resume_id = input
            .resume_provider_session_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        if let (Some(resume_id), Some(sink)) = (resume_id.clone(), input.audit_sink.as_ref()) {
            let stored: Option<StoredProviderStart> =
                sink.find_provider_start(&resume_id).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("anchor t10 resume lookup failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
            if let Some(stored) = stored.as_ref()
                && matches!(
                    resume_with_lc_start_record(&stored.record, &literal),
                    crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                )
            {
                append_superseded_policy_drift(sink.as_ref(), stored).map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("anchor t10 superseded audit append failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
                return Err(ProviderAdapterError::parse_error(
                    "anchor t10 adapter: resume audit drifted or legacy record lacks lc \
                     projection; refusing to spawn a fresh child for an explicit LC resume"
                        .to_string(),
                    String::new(),
                    String::new(),
                ));
            }
        }
        // pi extension 准备在 mismatch 判定之后(早写拒收)。
        if self.family == AnchorT10Family::Pi {
            self.extension_writes.fetch_add(1, Ordering::SeqCst);
        }
        // native child(口径③)。
        self.session_children.fetch_add(1, Ordering::SeqCst);
        let native_id = self
            .handshake_native_id
            .lock()
            .unwrap()
            .clone()
            .or_else(|| resume_id.clone())
            .unwrap_or_else(|| format!("native-{}-t10", self.family.tag()));
        if let (Some(expected), Some(confirmed)) = (
            resume_id.as_deref(),
            self.handshake_native_id.lock().unwrap().as_deref(),
        ) && expected != confirmed
        {
            // 错 ID:child 已启动的 runtime 失败——如实 kill/reap,不伪称零 child。
            self.children_killed.fetch_add(1, Ordering::SeqCst);
            return Err(ProviderAdapterError::parse_error(
                format!(
                    "anchor t10 {}: handshake confirmed {confirmed} but resume requested \
                     {expected} (session NOT resumed; child killed and reaped)",
                    self.family.tag()
                ),
                String::new(),
                String::new(),
            ));
        }
        // 成功路径:provider_start 首行(含本家 argv/sandbox/approval 与
        // lc_projection 五字段摘要)。
        if let Some(sink) = input.audit_sink.as_ref() {
            sink.append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: self.family.audit_provider_text().to_string(),
                role: "executor".to_string(),
                workspace_session_id: input
                    .workspace_session_id
                    .clone()
                    .unwrap_or_else(|| "ws-anchor-t10".to_string()),
                provider_session_id: native_id.clone(),
                tool_policy_canonical_digest: format!("sha256:t10-{}-policy", self.family.tag()),
                argv: self.family.audit_argv(&native_id, resume_id.as_deref()),
                sandbox: self.family.audit_sandbox(),
                approval_policy: self.family.audit_approval(),
                provider_version: observed_version,
                adapter_dialect: self.family.wire_text().to_string(),
                lc_projection: Some(LcProjectionAudit {
                    action: literal.action.clone(),
                    wire_dialect: literal.wire_dialect.clone(),
                    capability_projection_digest: literal.capability_projection_digest.clone(),
                    projection_digest: literal.projection_digest.clone(),
                    boundary_evidence_ref: literal.boundary_evidence_ref.clone(),
                }),
            }))
            .map_err(|error| {
                ProviderAdapterError::parse_error(
                    format!("anchor t10 provider_start append failed: {error}"),
                    String::new(),
                    String::new(),
                )
            })?;
        }
        Ok(Self::minimal_session(Some(native_id)))
    }

    async fn prepare_lc_facts(
        &self,
        _input: &crate::cross_cutting::streaming_provider::StreamingProviderInput,
        launch: &ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<ProviderLcPrepareFacts, ProviderAdapterError> {
        let key = LcVersionProbeKey {
            provider: self.family.name(),
            canonical_executable: self.family.canonical_executable(),
            executable_identity: self.executable_identity.lock().unwrap().clone(),
            argv: vec!["--version".to_string()],
            supplier_scope: 10 + (self.family.tag().len() as u64),
        };
        let probe_calls = self.probe_calls.clone();
        let observed_version = self.observed_version.clone();
        let probe: LcVersionProbe = Arc::new(move |_key| {
            let probe_calls = probe_calls.clone();
            let observed_version = observed_version.clone();
            Box::pin(async move {
                probe_calls.fetch_add(1, Ordering::SeqCst);
                Ok(observed_version.lock().unwrap().clone())
            })
        });
        let observation = self
            .cache
            .cached_lc_cli_version(key.clone(), probe, cancel)
            .await
            .map_err(|error| {
                ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    format!("anchor t10 fake prepare: {error}"),
                    0,
                )
            })?;
        let envelope = launch.envelope();
        let boundary = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            envelope.working_directory.clone(),
            None,
            Vec::new(),
        );
        let mut projector =
            AnchorT10Projector::new(self.family, observation.exact_version().to_string());
        projector.capability_digest_override =
            self.capability_digest_override.lock().unwrap().clone();
        projector.session_digest_override = self.session_digest_override.lock().unwrap().clone();
        Ok(ProviderLcPrepareFacts::new(
            self.family.ref_type(),
            envelope.provider_dialect,
            self.family.wire(),
            observation.exact_version().to_string(),
            observation.observed_at(),
            key,
            self.family.approval_policy().to_string(),
            self.mcp_bundle_digest.lock().unwrap().clone(),
            boundary,
            Arc::new(projector),
        ))
    }
}

/// t10 汇合 harness:四家公共 fixture(自包含 tempdir,不依赖
/// GatewayFixture;policy/capability/target/gate/audit 与 anchor_t09
/// 同形,registry 槽位随 family)。
pub(crate) struct AnchorT10Harness {
    pub(crate) adapter: Arc<AnchorT10FamilyAdapter>,
    pub(crate) gateway: LogicalCodebaseProviderGateway,
    pub(crate) trust: Arc<AnchorT10TrustSource>,
    pub(crate) audit: Arc<GatewayRunAudit>,
    pub(crate) request: SessionLaunchRequest,
    pub(crate) family: AnchorT10Family,
    paths: ProductAppPaths,
    authority_root: std::path::PathBuf,
    capabilities: Arc<AnchorT10CapabilitySource>,
    targets: Arc<MutableTargetResolver>,
    _root: tempfile::TempDir,
}

impl AnchorT10Harness {
    pub(crate) fn install_policy(&self) {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![]);
        AggregatePolicyArtifactStore::new(self.paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("anchor t10 bootstrap policy");
    }

    pub(crate) fn streaming_input(
        &self,
        resume: Option<String>,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        crate::cross_cutting::streaming_provider::StreamingProviderInput {
            provider_type: self.family.protocol_type(),
            role: crate::protocol::contracts::AdapterRole::Executor,
            prompt: "anchor t10 convergence".to_string(),
            working_dir: self.authority_root.clone(),
            working_directory: Some(self.authority_root.clone()),
            workspace_session_id: None,
            resume_provider_session_id: resume,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            tool_policy: None,
            audit_sink: None,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 5,
            baseline_tree: None,
        }
    }

    pub(crate) fn audit_context(
        &self,
        sink: Arc<dyn ToolPolicyAuditSink>,
    ) -> ProviderLaunchAuditContext {
        anchor_t10_audit_context(sink, "ws-anchor-t10", 1)
    }

    /// 以自定义 registry 构造同源 gateway(混合矩阵负例:换绑 adapter
    /// 实例/跨家串接)。
    pub(crate) fn gateway_with_registry(
        &self,
        registry: ProviderRegistry,
    ) -> LogicalCodebaseProviderGateway {
        self.build_gateway(Arc::new(registry))
    }

    pub(crate) fn registry(&self) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry
            .register_gated(
                self.family.name(),
                self.adapter.clone(),
                Arc::new(UnprovisionedProviderPolicyProjector),
                always_available_gate(),
            )
            .expect("anchor t10 harness registry register");
        registry
    }

    fn build_gateway(&self, registry: Arc<ProviderRegistry>) -> LogicalCodebaseProviderGateway {
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(self.paths.clone()),
            self.capabilities.clone(),
            self.targets.clone(),
            registry,
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            self.audit.clone(),
            self.authority_root.clone(),
        )
        .with_readonly_lc_facts(
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                self.paths.clone(),
                "logical_codebase_0001",
            ),
            self.trust.clone(),
            Some("logical_codebase_0001".to_string()),
        )
    }
}

pub(crate) fn anchor_t10_harness(family: AnchorT10Family) -> AnchorT10Harness {
    let root = tempfile::tempdir().expect("anchor t10 tempdir");
    let paths = ProductAppPaths::new(root.path());
    let manifest = LogicalCodebaseManifest::new("project_0001", paths.root().to_path_buf(), vec![]);
    let authority_root =
        std::fs::canonicalize(&manifest.provider_context_root).expect("authority root");
    let worktree = paths.root().join("worktree");
    std::fs::create_dir_all(&worktree).expect("anchor t10 worktree");
    let request = SessionLaunchRequest {
        project_id: manifest.project_id.clone(),
        provider: family.provider_ref(),
        action: SessionPolicyAction::PlanningReadOnly,
        target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
        // LC 会话 cwd = canonical LC root(root-cwd 契约,与 target 分离)。
        working_directory: authority_root.clone(),
        readable_roots: vec![paths.root().to_path_buf()],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };
    let adapter = AnchorT10FamilyAdapter::new(family, Arc::new(LcVersionCache::new()));
    let audit = Arc::new(GatewayRunAudit::new());
    let trust = AnchorT10TrustSource::new();
    let capabilities = Arc::new(AnchorT10CapabilitySource::new(family));
    let targets = Arc::new(MutableTargetResolver::new(
        paths.root().join("baseline.git"),
    ));
    let mut registry = ProviderRegistry::new();
    registry
        .register_gated(
            family.name(),
            adapter.clone(),
            Arc::new(UnprovisionedProviderPolicyProjector),
            always_available_gate(),
        )
        .expect("anchor t10 register gated family adapter");
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(paths.clone()),
        capabilities.clone(),
        targets.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
        audit.clone(),
        authority_root.clone(),
    )
    .with_readonly_lc_facts(
        crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
            paths.clone(),
            "logical_codebase_0001",
        ),
        trust.clone(),
        Some("logical_codebase_0001".to_string()),
    );
    AnchorT10Harness {
        adapter,
        gateway,
        trust,
        audit,
        request,
        family,
        paths,
        authority_root,
        capabilities,
        targets,
        _root: root,
    }
}

pub(crate) fn anchor_t10_audit_context(
    sink: Arc<dyn ToolPolicyAuditSink>,
    workspace_session_id: &str,
    role_run_seq: u64,
) -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: workspace_session_id.to_string(),
        role_run_seq,
        audit_sink: sink,
    }
}

/// 预置旧 run 的 durable provider_start 存档(resume 检索输入;lc_
/// projection 五字段与冻结 full projection 同源)。
pub(crate) fn anchor_t10_preset_resume_archive(
    sink: &Arc<crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink>,
    family: AnchorT10Family,
    resume_id: &str,
    projection: &ProviderPolicyProjection,
) {
    let record = ProviderStartAudit {
        provider: family.audit_provider_text().to_string(),
        role: "executor".to_string(),
        workspace_session_id: "ws-anchor-t10".to_string(),
        provider_session_id: resume_id.to_string(),
        tool_policy_canonical_digest: format!("sha256:t10-{}-policy", family.tag()),
        argv: Vec::new(),
        sandbox: family.audit_sandbox(),
        approval_policy: family.audit_approval(),
        provider_version: family.exact_version().to_string(),
        adapter_dialect: family.wire_text().to_string(),
        lc_projection: Some(LcProjectionAudit {
            action: "planning_read_only".to_string(),
            wire_dialect: family.wire_text().to_string(),
            capability_projection_digest: projection.capability_projection_digest().to_string(),
            projection_digest: projection.projection_digest().to_string(),
            boundary_evidence_ref: projection.boundary_evidence_ref().to_string(),
        }),
    };
    sink.with_stored_provider_start(record);
}
