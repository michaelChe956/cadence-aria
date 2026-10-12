//! REQ-REG-14：Codex/Kimi provider trust 硬前置门——gate 编排与稳定错误。
//!
//! trust 生命周期顺序固定为：LC 根准入（REQ-REG-09）冻结 canonical root
//! → 独立完成所选 Codex/Kimi trust 登记/核验/撤销与副作用审计 → 仅在
//! 全部 Ready 后允许 Claude Code 五步 recipe 创建/启动。登记不属于五步
//! operation、不插入步骤之间；任一失败进入可重试 fail-closed waiting，
//! 不降级、不部分放行。
//!
//! 3.1/3.2：ensure/revoke 的全部 home 副作用与 durable fact 写入都在稳定
//! 工件的 sidecar 锁（`ExclusiveFileLock` 先例：同目录 `.{name}.lock`）
//! 临界区内定序——Codex 整 config 共锁、Kimi 单记录锁，固定
//! home→LC fact 锁序，digest CAS 与 atomic rename 保留。该锁只约束
//! Aria 自身的协作写方：外部进程非协作编辑用户工件的竞态不因此消失，
//! 由 digest CAS/摘要校验 fail-closed 兜底（不宣称对外的强原子）。
//! 失败的 record_waiting/gate_waiting 与成功 clear_waiting 在同一临界
//! 区内定序；缺 adapter/未获锁时返回可见 Waiting、历史审计尽力写、
//! 不无序改写当前 waiting fact。锁失败（含非 Unix 平台）fail-closed。
//! 可取消入口（`ensure_ready_cancellable`）经 `acquire_async` 待锁
//! （caller 取消即停止，不留 flock waiter）、临界区同步 IO 经
//! spawn_blocking、Ready/Waiting 返回前释放 home 锁。

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::ExclusiveFileLock;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::provider_gateway::ProviderGatewayError;
use crate::product::logical_codebase::provider_trust_adapters::{
    ProviderTrustEntryState, ProviderTrustHomeAdapter, ProviderTrustWriteOutcome,
};
use crate::product::logical_codebase::provider_trust_store::{
    ProviderTrustAuditRecord, ProviderTrustOwnershipRecord, ProviderTrustStore,
    ProviderTrustWaitingRecord, new_audit_id, trust_now,
};
use crate::product::models::ProviderName;

/// trust 前置门的稳定错误：code 永不本地化；`retryable` 标记自动重试是否
/// 可能改变结果（用户值冲突不会）。错误内的 `retry_action` 是未分场景的
/// 默认动作；等待投影（`ensure_ready`/确认准备）按场景+失败分型覆写为
/// 各自正确的显式动作。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ProviderTrustError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub retry_action: String,
}

impl ProviderTrustError {
    /// fail-closed 且可重试（home 写入/读取/并发等瞬态失败）。
    pub fn transient(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            retryable: true,
            retry_action: "修复用户级 trust 配置的可写性/一致性后重试初始化".to_string(),
        }
    }

    /// 用户既有值冲突：不覆盖、不删除；等待用户处置后重试重新评估。
    pub fn conflict(code: &'static str, message: String) -> Self {
        Self {
            code,
            message,
            retryable: false,
            retry_action: "用户自行确认该 trust 键的取值后重试；系统不覆盖用户条目".to_string(),
        }
    }

    fn store(error: ProductStoreError) -> Self {
        Self::transient(
            "trust_store_io",
            format!("durable trust fact store failed: {error}"),
        )
    }
}

/// 单 provider trust 登记生命周期（trait 按 (project, lc, provider, root)
/// 调用点 scope；与计划文档相比仅前置了 `project_id` 路径参数，类型与
/// 字段名不变——durable 布局需要 project scope 才能定位 LC 子树）。
pub trait ProviderTrustRegistry: Send + Sync {
    fn ensure_trusted(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError>;

    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderTrustError>;

    fn revoke_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRevocation, ProviderTrustError>;
}

/// Task 3.1（B0a1）：只读 trust 冻结事实——resume/full 指纹冻结消费的
/// 纯读快照。`verification` 与 GET/spawn 复验共用同一 readonly 判定；
/// `record_digest` 只暴露 [`ProviderTrustHomeAdapter::digest`] 的工件摘要
/// （Codex 整份 config、Kimi 对应 record 原文），verified_at/detail 等
/// 观测时刻字段不参与摘要。零写铁律：不 ensure/revoke、不追加 trust
/// 审计（REQ-REG-15 两场景边界保持）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustFingerprintFacts {
    /// 与 [`ProviderTrustSource::verify_trusted`] 同源的只读判定。
    pub verification: ProviderTrustVerification,
    /// 该 provider 用户级工件的真实摘要；工件不存在时 `None`，不伪造。
    pub record_digest: Option<String>,
}

/// Task 3b(REQ-LCG-02/03):只读 trust source——GET/early eligibility 与
/// spawn 前复验共用的零副作用通道。
///
/// 与 [`ProviderTrustRegistry::verify_trusted`] 的区别:后者会向 durable
/// `ProviderTrustStore` 追加 verify audit 记录(副作用),不得用于
/// GET/early 或 spawn 复验;本 trait 的实现只允许经
/// [`ProviderTrustHomeAdapter::read_state`]/[`ProviderTrustHomeAdapter::digest`]
/// 读取用户级工件事实,禁止 ensure/revoke、禁止写 audit/ownership。
pub trait ProviderTrustSource: Send + Sync {
    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError>;

    /// 6.1（方案 A 附带，D5）：generation 首次 trust 失败的同源只读等待
    /// 投影（`Generation` 场景 retry_action＋manual_guidance；零 durable
    /// 副作用）。默认 `None`：非 home 背书实现（测试 fake 等）无同源
    /// 指导可给，调用方保持既有投影，不伪造。
    fn read_only_generation_waiting(
        &self,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Option<ProviderTrustWaiting> {
        let _ = (provider, canonical_root);
        None
    }

    /// T6 fix-loop r2（信任批量确认按需登记）：
    /// [`Self::read_only_generation_waiting`] 的场景参数化变体——共享
    /// 分类桥（gateway `classify_launch_trust_failure_details`）消费，
    /// Generation/Coding 面各自正确的 retry_action。默认 `None`：非
    /// home 背书实现无同源指导可给，调用方保持既有投影，不伪造。
    fn read_only_waiting_for_scenario(
        &self,
        provider: &ProviderName,
        canonical_root: &Path,
        scenario: ProviderTrustScenario,
    ) -> Option<ProviderTrustWaiting> {
        let _ = (provider, canonical_root, scenario);
        None
    }

    /// Task 3.1（B0a1）：冻结指纹消费的只读 trust 事实——verification 与
    /// [`Self::verify_trusted`] 共用同一 readonly 实现、工件摘要直接暴露
    /// [`ProviderTrustHomeAdapter::digest`]（Codex 整份 config、Kimi 对应
    /// record 原文）；**零写**：不 ensure/revoke、不追加 trust 审计。
    /// entry missing/非 trusted/登记后删除一律 typed 拒（错误携带稳定
    /// trust code），不得以 `Ok(trusted=false)` 冻结假事实。
    ///
    /// 默认实现返回 typed unsupported：仅未接入/测试 source 可停留在
    /// 默认；正例（真实 home 背书实现）必须提供可观测 facts，不得靠
    /// 默认拒绝使正例虚绿。
    fn fingerprint_facts(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustFingerprintFacts, ProviderGatewayError> {
        let _ = (project_id, lc_id, canonical_root);
        Err(ProviderGatewayError::ProviderUnavailable(format!(
            "unsupported_trust_fingerprint_facts: {provider:?} source does not expose read-only trust fingerprint facts"
        )))
    }
}

/// 只读 home 背书实现:factory 生产装配(只读 home adapter + 只读
/// ownership 查询,绝不写入)。
pub struct ReadonlyProviderTrustSource {
    paths: ProductAppPaths,
    lc_id: Option<String>,
    adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>,
}

impl ReadonlyProviderTrustSource {
    pub fn new(
        paths: ProductAppPaths,
        lc_id: Option<String>,
        adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>,
    ) -> Self {
        Self {
            paths,
            lc_id,
            adapters,
        }
    }

    /// 按 LC 作用域装配;lc 作用域未知(legacy 无别名)时 ownership 查询
    /// 跳过(只读事实缺失不构成伪造)。
    pub fn for_lc(paths: ProductAppPaths, lc_id: &str) -> Self {
        Self::new(paths, Some(lc_id.to_string()), Self::production_adapters())
    }

    /// factory 生产装配:按 LC 作用域(可空,legacy 无别名)注入生产
    /// trust adapters(Codex/Kimi——需要 workspace trust 的两家;Claude/Pi
    /// 无用户级 trust 工件,不进入本 source)。
    pub fn production_for_scope(paths: ProductAppPaths, lc_id: Option<String>) -> Self {
        Self::new(paths, lc_id, Self::production_adapters())
    }

    fn production_adapters() -> Vec<Arc<dyn ProviderTrustHomeAdapter>> {
        // production() 在 HOME 缺失(无用户级 home 可读)时返回 None——
        // 该 provider 无只读 trust 工件可查,不进入本 source。
        let codex =
            crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::production()
                .map(|adapter| Arc::new(adapter) as Arc<dyn ProviderTrustHomeAdapter>);
        let kimi =
            crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::production()
                .map(|adapter| Arc::new(adapter) as Arc<dyn ProviderTrustHomeAdapter>);
        codex.into_iter().chain(kimi).collect()
    }

    fn adapter_for(
        &self,
        provider: &ProviderName,
    ) -> Result<Arc<dyn ProviderTrustHomeAdapter>, ProviderGatewayError> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider() == *provider)
            .cloned()
            .ok_or_else(|| {
                ProviderGatewayError::ProviderUnavailable(format!(
                    "no read-only trust adapter is configured for {provider:?}"
                ))
            })
    }

    /// 6.1（方案 A 附带，D5）：generation/launch trust 失败的同源只读
    /// 等待投影（T6 fix-loop r2 起场景参数化，供共享分类桥按
    /// Generation/Coding 场景取用）。
    ///
    /// 与 [`ProviderTrustSource::verify_trusted`] 同一 adapter、同一
    /// trust key 算法，只读判定 entry 状态；未通过（缺失/非 trusted/
    /// 读取失败）时构造给定场景的等待面（场景化 retry_action＋
    /// 同源 manual_guidance——adapter 真实工件路径/键形/人工形状）。
    /// **零 durable 副作用**：不 ensure、不写 waiting/ownership/audit
    /// （只补只读指导，不自动登记）；条目已 trusted 或
    /// provider 无需 workspace trust 时返回 `None`（无可指导事实）。
    fn scenario_waiting(
        &self,
        provider: &ProviderName,
        canonical_root: &Path,
        scenario: ProviderTrustScenario,
    ) -> Option<ProviderTrustWaiting> {
        if !requires_workspace_trust(provider) {
            return None;
        }
        let waiting_for = |error: ProviderTrustError, trust_key: String| {
            let retry_action = scenario.retry_action(trust_failure_kind(&error));
            let manual_guidance = provider_trust_manual_guidance(
                &self.adapters,
                provider,
                canonical_root,
                &error,
                scenario,
            );
            ProviderTrustWaiting {
                provider: provider.clone(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key,
                reason_code: error.code.to_string(),
                message: error.message.clone(),
                retry_action,
                manual_guidance,
                command_markdown: trust_command_markdown(
                    &self.adapters,
                    provider,
                    canonical_root,
                    &error,
                ),
                recorded_at: trust_now(),
            }
        };
        let adapter = match self.adapter_for(provider) {
            Ok(adapter) => adapter,
            Err(_) => {
                // 无 adapter：NoAdapter 分型指导（修装配），给空 trust_key
                // 占位——等待面主文与指导不依赖该键。
                return Some(waiting_for(
                    ProviderTrustError::conflict(
                        "unsupported_trust_provider",
                        format!("no read-only trust adapter is configured for {provider:?}"),
                    ),
                    String::new(),
                ));
            }
        };
        if !canonical_root.is_absolute() || canonical_root.file_name().is_none() {
            return Some(waiting_for(
                ProviderTrustError::conflict(
                    "invalid_canonical_root",
                    format!(
                        "invalid canonical root for read-only trust guidance: {}",
                        canonical_root.display()
                    ),
                ),
                adapter.trust_key(canonical_root),
            ));
        }
        let trust_key = adapter.trust_key(canonical_root);
        match adapter.read_state(canonical_root) {
            Ok(ProviderTrustEntryState::Trusted) => None,
            Ok(ProviderTrustEntryState::Missing) => Some(waiting_for(
                ProviderTrustError {
                    code: TRUST_ENTRY_MISSING_CODE,
                    message: format!(
                        "canonical root {} 下未找到 {} 的用户级 workspace trust 条目（entry absent）",
                        canonical_root.display(),
                        provider_wire_label(provider),
                    ),
                    retryable: true,
                    // 场景化投影覆写；默认动作不进入等待面。
                    retry_action: String::new(),
                },
                trust_key,
            )),
            Ok(ProviderTrustEntryState::Untrusted { current_value }) => Some(waiting_for(
                ProviderTrustError::conflict(
                    adapter.conflict_code(),
                    format!(
                        "trust key holds a non-trusted value{}；系统不覆盖用户既有条目",
                        current_value
                            .map(|value| format!("（当前值 {value:?}）"))
                            .unwrap_or_default()
                    ),
                ),
                trust_key,
            )),
            Err(error) => Some(waiting_for(error, trust_key)),
        }
    }

    /// Task 3.1（B0a1）：verify/facts 共用的只读判定核心——同一 adapter、
    /// 同一 trust key 算法、同一 ownership 只读投影与 scope 校验；返回
    /// 判定结果与产生判定的 entry 原始状态（`fingerprint_facts` 按状态
    /// 给出稳定 code 的 typed 拒）。`verify_trusted` 与
    /// `fingerprint_facts` 都只经此核心读用户级工件，不新增第二份
    /// home 推导。
    fn verify_readonly_core(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<(ProviderTrustVerification, ProviderTrustEntryState), ProviderGatewayError> {
        let trust_error = |error: ProviderTrustError| {
            ProviderGatewayError::ProviderUnavailable(format!("{}: {}", error.code, error.message))
        };
        let adapter = self.adapter_for(provider)?;
        if !canonical_root.is_absolute() || canonical_root.file_name().is_none() {
            return Err(ProviderGatewayError::ProviderUnavailable(format!(
                "invalid canonical root for read-only trust check: {}",
                canonical_root.display()
            )));
        }
        let trust_key = adapter.trust_key(canonical_root);
        let state = adapter.read_state(canonical_root).map_err(trust_error)?;
        let trusted = matches!(state, ProviderTrustEntryState::Trusted);
        let detail = match state {
            ProviderTrustEntryState::Trusted => "entry present and trusted".to_string(),
            ProviderTrustEntryState::Untrusted { .. } => {
                "key holds a non-trusted value".to_string()
            }
            ProviderTrustEntryState::Missing => "entry absent".to_string(),
        };
        let ownership = match &self.lc_id {
            Some(scope) => {
                let store = ProviderTrustStore::for_lc(self.paths.clone(), scope);
                store
                    .load_ownership(project_id, provider_wire_label(provider), &trust_key)
                    .map_err(|error| {
                        ProviderGatewayError::ProviderUnavailable(format!(
                            "read trust ownership: {error}"
                        ))
                    })?
                    .map(|record| {
                        if record.owned {
                            ProviderTrustOwnership::LcManaged
                        } else {
                            ProviderTrustOwnership::UserOwned
                        }
                    })
            }
            None => None,
        };
        // T3R2-F2:形参 lc_id 与 source 装配作用域必须一致——调用侧携带
        // 的 LC 作用域(空串=无作用域)若与装配不一致,说明装配漂移,
        // fail-closed(本 change 语义:不静默采信任一侧)。
        let requested_scope = if lc_id.is_empty() { None } else { Some(lc_id) };
        if requested_scope != self.lc_id.as_deref() {
            return Err(ProviderGatewayError::ProviderUnavailable(format!(
                "read-only trust source scope mismatch: requested {requested_scope:?}, source {:?}",
                self.lc_id
            )));
        }
        Ok((
            ProviderTrustVerification {
                provider: provider.clone(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key,
                trusted,
                ownership,
                detail,
                verified_at: trust_now(),
            },
            state,
        ))
    }
}

impl ProviderTrustSource for ReadonlyProviderTrustSource {
    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError> {
        // Task 3.1（B0a1）：判定主体移入 `verify_readonly_core`，与
        // `fingerprint_facts` 共用同一 readonly 实现（不复制第二份推导）。
        self.verify_readonly_core(project_id, lc_id, provider, canonical_root)
            .map(|(verification, _)| verification)
    }

    fn fingerprint_facts(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustFingerprintFacts, ProviderGatewayError> {
        let (verification, state) =
            self.verify_readonly_core(project_id, lc_id, provider, canonical_root)?;
        if !verification.trusted {
            // typed 拒：missing/登记后删除回落 `trust_entry_missing`，
            // conflict 用 adapter 冲突 code——不折叠成通用字符串、
            // 不以 Ok(trusted=false) 冻结假事实。
            let adapter = self.adapter_for(provider)?;
            let code = match state {
                ProviderTrustEntryState::Missing => TRUST_ENTRY_MISSING_CODE,
                ProviderTrustEntryState::Untrusted { .. } => adapter.conflict_code(),
                ProviderTrustEntryState::Trusted => {
                    unreachable!("trusted state must not be rejected above")
                }
            };
            return Err(ProviderGatewayError::ProviderUnavailable(format!(
                "{code}: cannot freeze trust fingerprint for {provider:?} at {}: {}",
                canonical_root.display(),
                verification.detail
            )));
        }
        // 只暴露 adapter 工件摘要（Codex 整份 config、Kimi 对应 record
        // 原文），不复制算法、不混入 verified_at 等观测时刻字段；工件
        // 缺失时如实 `None`，不伪造。
        let record_digest =
            self.adapter_for(provider)?
                .digest(canonical_root)
                .map_err(|error| {
                    ProviderGatewayError::ProviderUnavailable(format!(
                        "{}: {}",
                        error.code, error.message
                    ))
                })?;
        Ok(ProviderTrustFingerprintFacts {
            verification,
            record_digest,
        })
    }

    fn read_only_generation_waiting(
        &self,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Option<ProviderTrustWaiting> {
        self.scenario_waiting(provider, canonical_root, ProviderTrustScenario::Generation)
    }

    fn read_only_waiting_for_scenario(
        &self,
        provider: &ProviderName,
        canonical_root: &Path,
        scenario: ProviderTrustScenario,
    ) -> Option<ProviderTrustWaiting> {
        self.scenario_waiting(provider, canonical_root, scenario)
    }
}

/// trust 准备场景：场景信息进入 preparation/等待投影（D2），使 recipe、
/// confirmation 的 retry_action 各自正确——同一失败在不同场景给出正确的
/// 显式动作，确认不得伪装成 recipe 重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustScenario {
    /// 五步 root recipe 初始化：失败后显式「重试初始化」。
    Recipe,
    /// 会话确认（human gate 确认前的 trust 准备）：失败后显式
    /// 「修复后重新确认」；确认门在修复前保持未关闭。
    Confirmation,
    /// 6.1（方案 A 附带，D5）：generation 首次 trust 失败的只读指导——
    /// 失败后显式「人工登记/修复后显式重新生成」；只读投影，不自动
    /// 登记（start_generation 不插入 ensure）。
    Generation,

    /// T6 fix-loop r2（信任批量确认按需登记）：Coding 面共享启动口
    /// (Coder/CodeReviewer/InternalReviewer 经 provider_stream launch）
    /// 的只读指导——失败后显式「重新发起该 Coding 任务」，尊重 Coding
    /// 既有人工恢复语义，不借用 generation 的「显式重新生成」。
    Coding,
    /// 指引卡「登记并重试」按钮（owner 2026-10-12 直批）：用户显式授权
    /// 的按需登记——ensure 失败的等待面 retry_action 指回按钮本身
    /// （修复后重新点击「登记并重试」）；成功只回执，不自动重新生成。
    ExplicitRegistration,
}

impl ProviderTrustScenario {
    /// 场景化 retry_action（等待面展示的产品动作）：失败分型参与分句，
    /// fact store 失败指修 store、装配失败指修装配，不把一切失败都
    /// 误导为用户 home 修复。
    fn retry_action(self, kind: TrustFailureKind) -> String {
        match kind {
            TrustFailureKind::EntryMissing => {
                format!("人工登记该 trust 条目后{}", self.explicit_action())
            }
            TrustFailureKind::UserConflict => match self {
                Self::Recipe => {
                    "用户自行确认该 trust 键的取值后重试；系统不覆盖用户条目".to_string()
                }
                Self::Confirmation => {
                    "用户自行核对该 trust 键的取值后重新确认；系统不覆盖用户条目".to_string()
                }
                Self::Generation => {
                    "用户自行核对该 trust 键的取值后显式重新生成；系统不覆盖用户条目".to_string()
                }
                Self::Coding => {
                    "用户自行核对该 trust 键的取值后重新发起该 Coding 任务；系统不覆盖用户条目"
                        .to_string()
                }
                Self::ExplicitRegistration => {
                    "用户自行核对该 trust 键的取值后重新点击「登记并重试」；系统不覆盖用户条目"
                        .to_string()
                }
            },
            TrustFailureKind::HomeArtifactRead | TrustFailureKind::HomeArtifactWrite => {
                format!(
                    "修复用户级 trust 配置的可写性/一致性后{}",
                    self.explicit_action()
                )
            }
            TrustFailureKind::FactStore => format!(
                "修复服务端 trust fact store 的可写性/一致性后{}",
                self.explicit_action()
            ),
            TrustFailureKind::NoAdapter => format!(
                "修复服务端 provider trust 依赖装配后{}",
                self.explicit_action()
            ),
            TrustFailureKind::InvalidRoot => {
                format!("修正聚合根路径参数后{}", self.explicit_action())
            }
        }
    }

    /// 场景的显式动作后缀（recipe=重试初始化，confirmation=重新确认，
    /// generation=显式重新生成，coding=重新发起该 Coding 任务——方案 A
    /// 不自动登记，登记/修复由用户显式完成后按场景重新发起）。
    /// explicit_registration=重新点击「登记并重试」（按钮场景失败指回
    /// 按钮本身；成功后按原场景显式重新生成/发起）。
    fn explicit_action(self) -> &'static str {
        match self {
            Self::Recipe => "重试初始化",
            Self::Confirmation => "重新确认",
            Self::Generation => "显式重新生成",
            Self::Coding => "重新发起该 Coding 任务",
            Self::ExplicitRegistration => "重新点击「登记并重试」",
        }
    }
}

/// trust 失败分型（D4）：人工指导与场景化 retry_action 的判据。分型不
/// 折叠 reason_code——等待面仍携带原始稳定 code。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrustFailureKind {
    /// 该根的 trust 条目缺失（entry absent）。6.1 只读 generation 投影的
    /// 专属分型——ensure 链路里缺失走登记成功路径，不产生等待。
    EntryMissing,
    /// 用户既有值冲突（不覆盖、不删除）。
    UserConflict,
    /// 用户级 home 工件读取/解析失败（权限/格式）。
    HomeArtifactRead,
    /// 用户级 home 工件写入/并发失败（可写性/CAS 漂移/摘要漂移）。
    HomeArtifactWrite,
    /// 服务端 durable trust fact store IO 失败。
    FactStore,
    /// 未装配该 provider 的 trust adapter（服务端装配问题）。
    NoAdapter,
    /// canonical root 参数无效。
    InvalidRoot,
}

/// 6.1：entry 缺失的稳定 code（只读 generation 投影构造，等待面原样
/// 携带，不折叠）。
const TRUST_ENTRY_MISSING_CODE: &str = "trust_entry_missing";

/// 按（稳定 code，retryable）分型；未知 code 落回按 retryable 的保守
/// 归类——不可自动重试视为用户冲突，其余瞬态视为 home 工件侧。
fn trust_failure_kind(error: &ProviderTrustError) -> TrustFailureKind {
    match error.code {
        TRUST_ENTRY_MISSING_CODE => TrustFailureKind::EntryMissing,
        "trust_store_io" => TrustFailureKind::FactStore,
        "unsupported_trust_provider" => TrustFailureKind::NoAdapter,
        "invalid_canonical_root" => TrustFailureKind::InvalidRoot,
        "codex_config_unreadable" | "kimi_trust_record_invalid" | "kimi_trust_unreadable" => {
            TrustFailureKind::HomeArtifactRead
        }
        _ if !error.retryable => TrustFailureKind::UserConflict,
        _ => TrustFailureKind::HomeArtifactWrite,
    }
}

/// 同源人工指导生成器（D4/D5）：确认等待与 generation 只读指导投影共用。
/// 由 adapter 窄只读 hint、失败分型与场景显式动作组成可读中文指导：
/// provider 特定工件只描述实际路径/键形与最小修复形状，不读取/返回完整
/// 配置内容或凭据；fact IO 指导修 store、无 adapter 只给装配修复。
pub(crate) fn provider_trust_manual_guidance(
    adapters: &[Arc<dyn ProviderTrustHomeAdapter>],
    provider: &ProviderName,
    canonical_root: &Path,
    error: &ProviderTrustError,
    scenario: ProviderTrustScenario,
) -> String {
    let kind = trust_failure_kind(error);
    let provider_label = provider_wire_label(provider);
    let retry_action = scenario.retry_action(kind);
    let disposition = match kind {
        TrustFailureKind::EntryMissing => format!(
            "该根尚未登记 {provider_label} 的用户级 workspace trust（reason_code={}，\
             entry absent）：系统不自动登记（方案 A：首次生成只给只读指导，\
             不代写用户级 trust）；请按下方工件路径与 trust 键人工登记，\
             然后{retry_action}。",
            error.code
        ),
        TrustFailureKind::UserConflict => format!(
            "该 trust 键已有用户既有值（reason_code={}）：系统不覆盖、不删除用户条目。\
             请人工核对该键当前取值后自行决定保留或修正（不要直接改成 trusted 了事），\
             然后{retry_action}。",
            error.code
        ),
        TrustFailureKind::HomeArtifactRead => format!(
            "用户级 trust 工件读取/解析失败（reason_code={}）：请先备份再修复该工件的\
             读取权限或格式，然后{retry_action}。",
            error.code
        ),
        TrustFailureKind::HomeArtifactWrite => format!(
            "用户级 trust 工件写入失败（reason_code={}）：请检查工件及其父目录的\
             可写性/一致性（并发修改可直接重试），然后{retry_action}。",
            error.code
        ),
        TrustFailureKind::FactStore => format!(
            "服务端 durable trust fact store 读写失败（reason_code={}）：这是服务端\
             登记事实存储的 IO 问题，不是用户级 trust 配置问题；请修复服务端 \
             trust fact store 的可写性/一致性后{retry_action}。",
            error.code
        ),
        TrustFailureKind::NoAdapter => format!(
            "未装配 {provider_label} 的 trust adapter（reason_code={}）：这是服务端 \
             provider trust 依赖装配问题，用户侧无需改动 home 配置；请修复服务端\
             装配后{retry_action}。",
            error.code
        ),
        TrustFailureKind::InvalidRoot => format!(
            "聚合根路径参数无效（reason_code={}）：{}。请修正后{retry_action}。",
            error.code, error.message
        ),
    };
    let artifact_line = match kind {
        TrustFailureKind::EntryMissing
        | TrustFailureKind::UserConflict
        | TrustFailureKind::HomeArtifactRead
        | TrustFailureKind::HomeArtifactWrite => adapters
            .iter()
            .find(|adapter| adapter.provider() == *provider)
            .map(|adapter| {
                let hint = adapter.manual_hint(canonical_root);
                format!(
                    "{provider_label} 的用户级 trust 工件：{}；本根的 trust 键：{}。\
                     人工形状：{}。",
                    hint.artifact_path.display(),
                    hint.trust_key,
                    hint.manual_shape,
                )
            }),
        // fact store/装配/根参数问题不给 home 工件指引，避免误导修复目标。
        TrustFailureKind::FactStore
        | TrustFailureKind::NoAdapter
        | TrustFailureKind::InvalidRoot => None,
    };
    // 指引卡登记按钮（owner 2026-10-12 直批）：entry 缺失时附可直接
    // 复制的登记命令（与 `trust_command_markdown` 同源 adapter hint；
    // 仅 EntryMissing 给命令——冲突/工件损坏场景给一键命令会把
    // 「不覆盖用户条目」弱化成覆盖指令）。
    let command_line = trust_command_markdown(adapters, provider, canonical_root, error)
        .map(|command| format!("可直接复制的登记命令：{command}。"));
    let mut guidance = disposition;
    if let Some(line) = artifact_line {
        guidance.push_str(&line);
    }
    if let Some(line) = command_line {
        guidance.push_str(&line);
    }
    if scenario == ProviderTrustScenario::Confirmation {
        guidance.push_str("修复前确认门保持未关闭；完成修复后请显式重新发起确认。");
    }
    guidance
}

/// 指引卡结构化命令（owner 2026-10-12 直批简化）：entry 缺失分型的
/// adapter 同源 `manual_command`（Kimi 单行 shell／Codex TOML 片段），
/// 供 wire `command_markdown` 直达前端「可复制命令窗」。冲突/工件
/// 损坏/装配/根参数等其余分型一律 `None`——一键命令只该出现在 entry
/// 缺失（冲突场景给命令会把「不覆盖用户条目」弱化成覆盖指令）。与
/// 平铺 `manual_guidance` 的命令段同源（同一 adapter hint，不手抄）。
fn trust_command_markdown(
    adapters: &[Arc<dyn ProviderTrustHomeAdapter>],
    provider: &ProviderName,
    canonical_root: &Path,
    error: &ProviderTrustError,
) -> Option<String> {
    if !matches!(trust_failure_kind(error), TrustFailureKind::EntryMissing) {
        return None;
    }
    adapters
        .iter()
        .find(|adapter| adapter.provider() == *provider)
        .and_then(|adapter| adapter.manual_hint(canonical_root).manual_command)
}

/// trust 硬前置门（场景中立）：全部所选 trust Ready 才放行调用方的后续
/// 动作（五步 recipe 的创建/启动、会话确认准备等）。`'static`：可取消
/// 入口返回的 boxed future 持有 `Arc<Self>`。
pub trait ProviderTrustPrecondition: Send + Sync + 'static {
    /// `providers` 中需要 workspace trust 的项（Codex/KimiCode）逐一登记；
    /// 任一失败返回可重试 Waiting（durable waiting fact + 审计），调用方
    /// 不得创建/启动后续动作。全部 Ready（或无需 trust）才返回 Ready。
    ///
    /// `scenario` 是准备场景（recipe/confirmation）：进入等待投影的
    /// retry_action 与人工指导，使各场景的显式动作各自正确。
    fn ensure_ready(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &Path,
        providers: &[ProviderName],
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult;

    /// 可取消等待入口（3.2）：待锁获取按既有可取消语义（try-lock+退避，
    /// caller 取消即停止，不留 flock waiter、释放后业务不复活）；临界区
    /// 一旦开始（home 锁已获取=提交边界），取消 caller 只丢失结果观察，
    /// 临界区自身安全收尾。同步 IO 经 spawn_blocking；Ready/Waiting
    /// 返回前 home 锁已释放。
    ///
    /// 默认实现直接执行同步 [`Self::ensure_ready`]（无锁 fake 适用）；
    /// 生产 `HomeBackedProviderTrustRegistry` 覆写为 `acquire_async`
    /// 先行、再经 spawn_blocking 进入临界区。
    fn ensure_ready_cancellable(
        self: Arc<Self>,
        project_id: String,
        operation_id: String,
        lc_id: String,
        canonical_root: PathBuf,
        providers: Vec<ProviderName>,
        scenario: ProviderTrustScenario,
    ) -> Pin<Box<dyn Future<Output = ProviderTrustPreparationResult> + Send>> {
        Box::pin(async move {
            self.ensure_ready(
                &project_id,
                &operation_id,
                &lc_id,
                &canonical_root,
                &providers,
                scenario,
            )
        })
    }
}

/// What one ensure pass did to the user-level trust artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustAction {
    /// Wrote the entry (first registration for this root).
    Register,
    /// Entry already trusted and owned by this LC; replay wrote nothing.
    Replay,
    /// Entry already trusted but NOT owned by this LC; reused untouched.
    ReuseUserEntry,
}

/// Ownership proof attached to every registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustOwnership {
    /// This LC created the entry and may revoke it (digest permitting).
    LcManaged,
    /// The user (or another tool) created the entry; never written/revoked.
    UserOwned,
}

/// Terminal state of one provider's registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTrustRegistrationResult {
    Ready,
    Waiting { reason_code: String },
}

/// One provider's ensure result（计划契约字段：
/// provider/root/key/action/before_digest/after_digest/ownership/result）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustRegistration {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub action: ProviderTrustAction,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub ownership: ProviderTrustOwnership,
    pub result: ProviderTrustRegistrationResult,
    pub registered_at: String,
}

/// One provider's verify result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustVerification {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub trusted: bool,
    pub ownership: Option<ProviderTrustOwnership>,
    pub detail: String,
    pub verified_at: String,
}

/// One provider's revoke result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderTrustRevocationOutcome {
    /// LC-owned entry removed after digest verification.
    Removed,
    /// User-owned (or not owned by this LC); left untouched.
    NotOwned,
    /// Nothing to remove.
    EntryMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustRevocation {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub outcome: ProviderTrustRevocationOutcome,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub revoked_at: String,
}

/// Gate verdict for the five-step recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTrustPreparationResult {
    Ready {
        registrations: Vec<ProviderTrustRegistration>,
    },
    Waiting {
        waiting: ProviderTrustWaiting,
    },
}

/// Retryable fail-closed waiting surface (stable code + product action).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustWaiting {
    pub provider: ProviderName,
    pub canonical_root: PathBuf,
    pub trust_key: String,
    pub reason_code: String,
    pub message: String,
    pub retry_action: String,
    /// 场景化人工指导（D4）：provider 特定工件路径/键形与失败分型处置，
    /// 由 adapter 窄只读能力生成；不含完整配置内容或凭据。确认等待与
    /// generation 只读指导投影共用同一生成器。
    pub manual_guidance: String,
    /// 结构化登记命令（owner 2026-10-12 直批卡简化）：entry 缺失分型
    /// 携带 adapter 同源 `manual_command`（Kimi 单行 shell／Codex TOML
    /// 片段），经 wire `command_markdown` 直达前端「可复制命令窗」，
    /// 前端不再解析平铺 `manual_guidance`；其余分型 `None`（与平铺
    /// 命令段同源生成，不手抄第二份）。
    pub command_markdown: Option<String>,
    pub recorded_at: String,
}

/// Which providers need user-level workspace trust before an LC session.
pub fn requires_workspace_trust(provider: &ProviderName) -> bool {
    matches!(provider, ProviderName::Codex | ProviderName::KimiCode)
}

/// Wire label persisted into durable trust facts（snake_case，与
/// `ProviderName` serde 序列化一致）。
pub(crate) fn provider_wire_label(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Pi => "pi",
        ProviderName::KimiCode => "kimi_code",
        ProviderName::Fake => "fake",
    }
}

/// Home-file backed registry：home 副作用全部经注入的 adapter 发生，
/// durable facts 落在 per-LC scope 的 `ProviderTrustStore`。
pub struct HomeBackedProviderTrustRegistry {
    paths: ProductAppPaths,
    adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>,
}

impl HomeBackedProviderTrustRegistry {
    pub fn new(paths: ProductAppPaths, adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>>) -> Self {
        Self { paths, adapters }
    }

    fn adapter_for(
        &self,
        provider: &ProviderName,
    ) -> Result<Arc<dyn ProviderTrustHomeAdapter>, ProviderTrustError> {
        self.adapters
            .iter()
            .find(|adapter| adapter.provider() == *provider)
            .cloned()
            .ok_or_else(|| {
                ProviderTrustError::conflict(
                    "unsupported_trust_provider",
                    format!("no trust adapter is configured for {provider:?}"),
                )
            })
    }

    fn validate_root(canonical_root: &Path) -> Result<(), ProviderTrustError> {
        if !canonical_root.is_absolute() || canonical_root.file_name().is_none() {
            return Err(ProviderTrustError::transient(
                "invalid_canonical_root",
                format!(
                    "canonical root must be an absolute path with a file name, got {}",
                    canonical_root.display()
                ),
            ));
        }
        Ok(())
    }

    fn store_for(&self, lc_id: &str) -> ProviderTrustStore {
        ProviderTrustStore::for_lc(self.paths.clone(), lc_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        store: &ProviderTrustStore,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        action: &str,
        outcome: &str,
        before_digest: Option<String>,
        after_digest: Option<String>,
    ) -> Result<(), ProviderTrustError> {
        store
            .append_audit(
                project_id,
                ProviderTrustAuditRecord {
                    audit_id: new_audit_id(),
                    lc_id: lc_id.to_string(),
                    operation_id: non_empty(operation_id),
                    provider: provider_wire_label(provider).to_string(),
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: trust_key.to_string(),
                    action: action.to_string(),
                    outcome: outcome.to_string(),
                    before_digest,
                    after_digest,
                    recorded_at: trust_now(),
                },
            )
            .map_err(ProviderTrustError::store)
    }

    /// Entry already trusted: refresh ownership/digest facts without writing
    /// the user home file.
    #[allow(clippy::too_many_arguments)]
    fn reuse_trusted_entry(
        &self,
        store: &ProviderTrustStore,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        digest: Option<String>,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError> {
        let now = trust_now();
        let provider_label = provider_wire_label(provider);
        let existing = store
            .load_ownership(project_id, provider_label, trust_key)
            .map_err(ProviderTrustError::store)?;
        let lc_managed = existing.as_ref().is_some_and(|record| record.owned);
        store
            .record_ownership(
                project_id,
                ProviderTrustOwnershipRecord {
                    lc_id: lc_id.to_string(),
                    provider: provider_label.to_string(),
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: trust_key.to_string(),
                    owned: lc_managed,
                    before_digest: digest.clone(),
                    after_digest: digest.clone(),
                    created_at: existing
                        .as_ref()
                        .map(|record| record.created_at.clone())
                        .unwrap_or_else(|| now.clone()),
                    updated_at: now.clone(),
                },
            )
            .map_err(ProviderTrustError::store)?;
        Ok(ProviderTrustRegistration {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key: trust_key.to_string(),
            action: if lc_managed {
                ProviderTrustAction::Replay
            } else {
                ProviderTrustAction::ReuseUserEntry
            },
            before_digest: digest.clone(),
            after_digest: digest,
            ownership: if lc_managed {
                ProviderTrustOwnership::LcManaged
            } else {
                ProviderTrustOwnership::UserOwned
            },
            result: ProviderTrustRegistrationResult::Ready,
            registered_at: now,
        })
    }

    /// 3.1：在首次 digest/read_state 之前获取稳定工件 sidecar 锁
    /// （`ExclusiveFileLock` 先例：同目录 `.{name}.lock`，flock 互斥，
    /// Drop 释放）。锁失败（含非 Unix 平台 fail-closed）映射为可重试
    /// `trust_home_lock_failed`，绝不无锁进入临界区。
    fn acquire_home_lock(
        adapter: &dyn ProviderTrustHomeAdapter,
        canonical_root: &Path,
    ) -> Result<ExclusiveFileLock, ProviderTrustError> {
        let artifact = adapter.artifact_path(canonical_root);
        ExclusiveFileLock::acquire(&artifact).map_err(|error| {
            ProviderTrustError::transient(
                "trust_home_lock_failed",
                format!(
                    "acquire home trust sidecar lock at {}: {error}; \
                     检查该工件父目录的可写性后重试",
                    artifact.display()
                ),
            )
        })
    }

    /// 3.2：可取消待锁（既有 `acquire_async` 语义：try-lock+退避，caller
    /// drop 即停止等待，不留 flock waiter）。
    async fn acquire_home_lock_async(
        adapter: &dyn ProviderTrustHomeAdapter,
        canonical_root: &Path,
    ) -> Result<ExclusiveFileLock, ProviderTrustError> {
        let artifact = adapter.artifact_path(canonical_root);
        ExclusiveFileLock::acquire_async(&artifact)
            .await
            .map_err(|error| {
                ProviderTrustError::transient(
                    "trust_home_lock_failed",
                    format!(
                        "acquire home trust sidecar lock at {}: {error}; \
                     检查该工件父目录的可写性后重试",
                        artifact.display()
                    ),
                )
            })
    }

    /// 失败等待投影（场景化 retry_action + 同源人工指导）；
    /// reason_code/message 原样保留，不折叠。
    #[allow(clippy::too_many_arguments)]
    fn waiting_surface(
        &self,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        error: &ProviderTrustError,
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustWaiting {
        let kind = trust_failure_kind(error);
        let retry_action = scenario.retry_action(kind);
        let manual_guidance = provider_trust_manual_guidance(
            &self.adapters,
            provider,
            canonical_root,
            error,
            scenario,
        );
        ProviderTrustWaiting {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key: trust_key.to_string(),
            reason_code: error.code.to_string(),
            message: error.message.clone(),
            retry_action,
            manual_guidance,
            command_markdown: trust_command_markdown(&self.adapters, provider, canonical_root, error),
            recorded_at: trust_now(),
        }
    }

    /// 3.2：持锁失败的等待落盘——waiting fact 与 gate_waiting 审计都在
    /// 调用方已持有的 home 临界区内（与成功 clear_waiting 同一定序）。
    /// 落盘失败不吞 gate 判定：仍返回可见 Waiting（fail-closed），不伪称
    /// 全审计落盘。
    #[allow(clippy::too_many_arguments)]
    fn gate_waiting_locked(
        &self,
        store: &ProviderTrustStore,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        error: ProviderTrustError,
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustWaiting {
        let waiting = self.waiting_surface(provider, canonical_root, trust_key, &error, scenario);
        // durable waiting fact + 审计：失败也必须可关联、可重试。
        let _ = store.record_waiting(
            project_id,
            ProviderTrustWaitingRecord {
                lc_id: lc_id.to_string(),
                provider: provider_wire_label(provider).to_string(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key: trust_key.to_string(),
                operation_id: non_empty(operation_id),
                reason_code: error.code.to_string(),
                message: error.message.clone(),
                retry_action: waiting.retry_action.clone(),
                recorded_at: waiting.recorded_at.clone(),
            },
        );
        let _ = self.audit(
            store,
            project_id,
            operation_id,
            lc_id,
            provider,
            canonical_root,
            trust_key,
            "gate_waiting",
            error.code,
            None,
            None,
        );
        waiting
    }

    /// 3.2：缺 adapter/未获锁（无法以 home 锁定序）时的等待投影：可见
    /// Waiting + 历史审计尽力写，但不无序改写当前 waiting fact。
    #[allow(clippy::too_many_arguments)]
    fn gate_waiting_unordered(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        error: ProviderTrustError,
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustWaiting {
        let waiting = self.waiting_surface(provider, canonical_root, trust_key, &error, scenario);
        let store = self.store_for(lc_id);
        let _ = self.audit(
            &store,
            project_id,
            operation_id,
            lc_id,
            provider,
            canonical_root,
            trust_key,
            "gate_waiting",
            error.code,
            None,
            None,
        );
        waiting
    }

    /// [`Self::gate_waiting_unordered`] 的异步面：可见等待面在内存构建，
    /// 历史审计（同步 IO）经 spawn_blocking 尽力写；JoinError 时仅返回
    /// 等待面（不二次阻塞）。
    #[allow(clippy::too_many_arguments)]
    async fn gate_waiting_unordered_spawned(
        self: &Arc<Self>,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        trust_key: &str,
        error: ProviderTrustError,
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustWaiting {
        let waiting = self.waiting_surface(provider, canonical_root, trust_key, &error, scenario);
        let registry = Arc::clone(self);
        let section_project = project_id.to_string();
        let section_operation = operation_id.to_string();
        let section_lc = lc_id.to_string();
        let section_provider = provider.clone();
        let section_root = canonical_root.to_path_buf();
        let section_key = trust_key.to_string();
        let section_outcome = error.code;
        let _ = tokio::task::spawn_blocking(move || {
            let store = registry.store_for(&section_lc);
            let _ = registry.audit(
                &store,
                &section_project,
                &section_operation,
                &section_lc,
                &section_provider,
                &section_root,
                &section_key,
                "gate_waiting",
                section_outcome,
                None,
                None,
            );
        })
        .await;
        waiting
    }

    /// 单 provider ensure 临界区核心（调用方必须已持 home sidecar 锁；
    /// `_home_lock` 即持锁证明）。成功的 clear_waiting 与审计、失败的
    /// record_waiting/gate_waiting 都在同一临界区内定序（3.1/3.2）；
    /// digest CAS 与 atomic rename 语义保持不变。
    #[allow(clippy::too_many_arguments)]
    fn ensure_trusted_locked(
        &self,
        _home_lock: &ExclusiveFileLock,
        store: &ProviderTrustStore,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        adapter: &dyn ProviderTrustHomeAdapter,
        trust_key: &str,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError> {
        let provider_label = provider_wire_label(provider);
        let before_digest = adapter.digest(canonical_root)?;
        let state = adapter.read_state(canonical_root)?;
        let (registration, audit_outcome) = match state {
            ProviderTrustEntryState::Trusted => {
                let registration = self.reuse_trusted_entry(
                    store,
                    project_id,
                    lc_id,
                    provider,
                    canonical_root,
                    trust_key,
                    before_digest.clone(),
                )?;
                let outcome = if registration.action == ProviderTrustAction::Replay {
                    "replay"
                } else {
                    "reuse_user_entry"
                };
                (registration, outcome)
            }
            ProviderTrustEntryState::Missing => {
                match adapter.write_trusted(canonical_root, before_digest.as_deref())? {
                    ProviderTrustWriteOutcome::AlreadyTrusted => {
                        // 另一写方在状态读取与写入之间登记了该根：重新
                        // 摘要并按可信复用处理。
                        let registration = self.reuse_trusted_entry(
                            store,
                            project_id,
                            lc_id,
                            provider,
                            canonical_root,
                            trust_key,
                            adapter.digest(canonical_root)?,
                        )?;
                        (registration, "replay")
                    }
                    ProviderTrustWriteOutcome::Created {
                        before_digest: _,
                        after_digest,
                    } => {
                        let now = trust_now();
                        store
                            .record_ownership(
                                project_id,
                                ProviderTrustOwnershipRecord {
                                    lc_id: lc_id.to_string(),
                                    provider: provider_label.to_string(),
                                    canonical_root: canonical_root.to_path_buf(),
                                    trust_key: trust_key.to_string(),
                                    owned: true,
                                    before_digest: before_digest.clone(),
                                    after_digest: Some(after_digest.clone()),
                                    created_at: now.clone(),
                                    updated_at: now.clone(),
                                },
                            )
                            .map_err(ProviderTrustError::store)?;
                        (
                            ProviderTrustRegistration {
                                provider: provider.clone(),
                                canonical_root: canonical_root.to_path_buf(),
                                trust_key: trust_key.to_string(),
                                action: ProviderTrustAction::Register,
                                before_digest: before_digest.clone(),
                                after_digest: Some(after_digest),
                                ownership: ProviderTrustOwnership::LcManaged,
                                result: ProviderTrustRegistrationResult::Ready,
                                registered_at: now,
                            },
                            "registered",
                        )
                    }
                }
            }
            ProviderTrustEntryState::Untrusted { current_value } => {
                self.audit(
                    store,
                    project_id,
                    operation_id,
                    lc_id,
                    provider,
                    canonical_root,
                    trust_key,
                    "ensure",
                    "conflict",
                    before_digest.clone(),
                    before_digest.clone(),
                )?;
                return Err(ProviderTrustError::conflict(
                    adapter.conflict_code(),
                    format!(
                        "trust key {} for {} holds a non-trusted value {:?}; \
                         拒绝覆盖用户既有条目",
                        trust_key,
                        canonical_root.display(),
                        current_value
                    ),
                ));
            }
        };

        store
            .clear_waiting(project_id, provider_label, trust_key)
            .map_err(ProviderTrustError::store)?;
        self.audit(
            store,
            project_id,
            operation_id,
            lc_id,
            provider,
            canonical_root,
            trust_key,
            "ensure",
            audit_outcome,
            registration.before_digest.clone(),
            registration.after_digest.clone(),
        )?;
        Ok(registration)
    }

    /// 单 provider 完整临界区（已持锁进入）：ensure 尝试；失败时
    /// record_waiting/gate_waiting 在同一临界区落盘后以 Err(waiting)
    /// 返回。同步 `ensure_ready` 与可取消入口共用此核心，不二次获取
    /// 非重入锁（3.2）；锁在返回前释放（值语义 Drop）。
    #[allow(clippy::too_many_arguments, clippy::result_large_err)]
    fn ensure_provider_critical_section(
        &self,
        home_lock: ExclusiveFileLock,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
        adapter: Arc<dyn ProviderTrustHomeAdapter>,
        scenario: ProviderTrustScenario,
    ) -> Result<ProviderTrustRegistration, ProviderTrustWaiting> {
        let store = self.store_for(lc_id);
        let trust_key = adapter.trust_key(canonical_root);
        let outcome = self.ensure_trusted_locked(
            &home_lock,
            &store,
            project_id,
            operation_id,
            lc_id,
            provider,
            canonical_root,
            adapter.as_ref(),
            &trust_key,
        );
        match outcome {
            Ok(registration) => Ok(registration),
            Err(error) => Err(self.gate_waiting_locked(
                &store,
                project_id,
                operation_id,
                lc_id,
                provider,
                canonical_root,
                &trust_key,
                error,
                scenario,
            )),
        }
    }
}

impl ProviderTrustRegistry for HomeBackedProviderTrustRegistry {
    fn ensure_trusted(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRegistration, ProviderTrustError> {
        let adapter = self.adapter_for(provider)?;
        Self::validate_root(canonical_root)?;
        // 3.1：首次 digest/read_state 之前拿稳定工件 sidecar 锁；锁失败
        // fail-closed，绝不无锁进入临界区。
        let home_lock = Self::acquire_home_lock(adapter.as_ref(), canonical_root)?;
        let store = self.store_for(lc_id);
        let trust_key = adapter.trust_key(canonical_root);
        let registration = self.ensure_trusted_locked(
            &home_lock,
            &store,
            project_id,
            operation_id,
            lc_id,
            provider,
            canonical_root,
            adapter.as_ref(),
            &trust_key,
        );
        drop(home_lock);
        registration
    }

    fn verify_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderTrustError> {
        let adapter = self.adapter_for(provider)?;
        Self::validate_root(canonical_root)?;
        let store = self.store_for(lc_id);
        let provider_label = provider_wire_label(provider);
        let trust_key = adapter.trust_key(canonical_root);
        let before_digest = adapter.digest(canonical_root)?;
        let state = adapter.read_state(canonical_root)?;
        let trusted = matches!(state, ProviderTrustEntryState::Trusted);
        let (detail, outcome) = match state {
            ProviderTrustEntryState::Trusted => ("entry present and trusted", "trusted"),
            ProviderTrustEntryState::Untrusted { .. } => {
                ("key holds a non-trusted value", "untrusted")
            }
            ProviderTrustEntryState::Missing => ("entry absent", "missing"),
        };
        let ownership = store
            .load_ownership(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?
            .map(|record| {
                if record.owned {
                    ProviderTrustOwnership::LcManaged
                } else {
                    ProviderTrustOwnership::UserOwned
                }
            });
        self.audit(
            &store,
            project_id,
            "",
            lc_id,
            provider,
            canonical_root,
            &trust_key,
            "verify",
            outcome,
            before_digest.clone(),
            before_digest.clone(),
        )?;
        Ok(ProviderTrustVerification {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key,
            trusted,
            ownership,
            detail: detail.to_string(),
            verified_at: trust_now(),
        })
    }

    fn revoke_trusted(
        &self,
        project_id: &str,
        lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustRevocation, ProviderTrustError> {
        let adapter = self.adapter_for(provider)?;
        Self::validate_root(canonical_root)?;
        let store = self.store_for(lc_id);
        let provider_label = provider_wire_label(provider);
        let trust_key = adapter.trust_key(canonical_root);
        // 3.1：revoke 全程在稳定工件 sidecar 锁内（digest/ownership 读改/
        // remove_trusted/审计）；锁失败 fail-closed。`_home_lock` 绑定
        // 存活到函数末尾，所有 return 路径都在锁释放之后。
        let _home_lock = Self::acquire_home_lock(adapter.as_ref(), canonical_root)?;
        let before_digest = adapter.digest(canonical_root)?;

        let ownership = store
            .load_ownership(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?;
        let Some(ownership) = ownership.filter(|record| record.owned) else {
            // 非本 LC 管理：用户条目原样保留，绝不删除。
            let state = adapter.read_state(canonical_root)?;
            let outcome = match state {
                ProviderTrustEntryState::Missing => ProviderTrustRevocationOutcome::EntryMissing,
                _ => ProviderTrustRevocationOutcome::NotOwned,
            };
            let outcome_label = match outcome {
                ProviderTrustRevocationOutcome::EntryMissing => "entry_missing",
                _ => "not_owned",
            };
            self.audit(
                &store,
                project_id,
                "",
                lc_id,
                provider,
                canonical_root,
                &trust_key,
                "revoke",
                outcome_label,
                before_digest.clone(),
                before_digest.clone(),
            )?;
            return Ok(ProviderTrustRevocation {
                provider: provider.clone(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key,
                outcome,
                before_digest: before_digest.clone(),
                after_digest: before_digest,
                revoked_at: trust_now(),
            });
        };

        // 归属证明存在：撤销前核对摘要，被外部修改即 fail-closed 等待核验。
        if before_digest != ownership.after_digest {
            self.audit(
                &store,
                project_id,
                "",
                lc_id,
                provider,
                canonical_root,
                &trust_key,
                "revoke",
                "digest_mismatch",
                before_digest.clone(),
                ownership.after_digest.clone(),
            )?;
            return Err(ProviderTrustError::transient(
                "revoke_digest_mismatch",
                format!(
                    "trust artifact for {} changed after registration \
                     (recorded {:?}, now {:?}); 保持 fail-closed 等待人工核验",
                    canonical_root.display(),
                    ownership.after_digest,
                    before_digest
                ),
            ));
        }

        adapter.remove_trusted(canonical_root, before_digest.as_deref())?;
        let after_digest = adapter.digest(canonical_root)?;
        store
            .delete_ownership(project_id, provider_label, &trust_key)
            .map_err(ProviderTrustError::store)?;
        self.audit(
            &store,
            project_id,
            "",
            lc_id,
            provider,
            canonical_root,
            &trust_key,
            "revoke",
            "removed",
            before_digest.clone(),
            after_digest.clone(),
        )?;
        Ok(ProviderTrustRevocation {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key,
            outcome: ProviderTrustRevocationOutcome::Removed,
            before_digest,
            after_digest,
            revoked_at: trust_now(),
        })
    }
}

impl ProviderTrustPrecondition for HomeBackedProviderTrustRegistry {
    fn ensure_ready(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &Path,
        providers: &[ProviderName],
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult {
        let mut selected: Vec<ProviderName> = Vec::new();
        for provider in providers {
            if requires_workspace_trust(provider) && !selected.contains(provider) {
                selected.push(provider.clone());
            }
        }
        // 无 trust provider 不取锁、不触碰 home（3.1）。
        let mut registrations = Vec::new();
        for provider in selected {
            // 登记前无副作用合法性预检：无效输入不取锁、不触碰 home。
            // 缺 adapter/根无效无法进入可定序的临界区：可见 Waiting +
            // 历史审计尽力写，但不无序改写当前 waiting fact（3.2）。
            let adapter = match self.adapter_for(&provider) {
                Ok(adapter) => adapter,
                Err(error) => {
                    return ProviderTrustPreparationResult::Waiting {
                        waiting: self.gate_waiting_unordered(
                            project_id,
                            operation_id,
                            lc_id,
                            &provider,
                            canonical_root,
                            "",
                            error,
                            scenario,
                        ),
                    };
                }
            };
            if let Err(error) = Self::validate_root(canonical_root) {
                return ProviderTrustPreparationResult::Waiting {
                    waiting: self.gate_waiting_unordered(
                        project_id,
                        operation_id,
                        lc_id,
                        &provider,
                        canonical_root,
                        &adapter.trust_key(canonical_root),
                        error,
                        scenario,
                    ),
                };
            }
            // 锁失败 fail-closed：同样不无序改写当前 waiting fact。
            let home_lock = match Self::acquire_home_lock(adapter.as_ref(), canonical_root) {
                Ok(lock) => lock,
                Err(error) => {
                    return ProviderTrustPreparationResult::Waiting {
                        waiting: self.gate_waiting_unordered(
                            project_id,
                            operation_id,
                            lc_id,
                            &provider,
                            canonical_root,
                            &adapter.trust_key(canonical_root),
                            error,
                            scenario,
                        ),
                    };
                }
            };
            // 失败 record_waiting/gate_waiting 与成功 clear_waiting 在
            // 同一已持 home 临界区内定序（3.2）；共用持锁核心
            // ensure_provider_critical_section，不二次获取非重入锁；
            // Ready/Waiting 返回前锁已释放。
            match self.ensure_provider_critical_section(
                home_lock,
                project_id,
                operation_id,
                lc_id,
                &provider,
                canonical_root,
                adapter,
                scenario,
            ) {
                Ok(registration) => registrations.push(registration),
                Err(waiting) => {
                    return ProviderTrustPreparationResult::Waiting { waiting };
                }
            }
        }
        ProviderTrustPreparationResult::Ready { registrations }
    }

    /// 3.2 覆写：待锁经 `acquire_async`（可取消：caller drop 即停止，
    /// 不留 flock waiter、释放后业务不复活）；临界区同步 IO 经
    /// spawn_blocking。临界区一旦开始（锁已获取=提交边界），取消 caller
    /// 只丢失结果观察，临界区 detached 安全收尾、不误删他人事实。
    fn ensure_ready_cancellable(
        self: Arc<Self>,
        project_id: String,
        operation_id: String,
        lc_id: String,
        canonical_root: PathBuf,
        providers: Vec<ProviderName>,
        scenario: ProviderTrustScenario,
    ) -> Pin<Box<dyn Future<Output = ProviderTrustPreparationResult> + Send>> {
        Box::pin(async move {
            let mut selected: Vec<ProviderName> = Vec::new();
            for provider in &providers {
                if requires_workspace_trust(provider) && !selected.contains(provider) {
                    selected.push(provider.clone());
                }
            }
            // 无 trust provider 不取锁、不触碰 home。
            let mut registrations = Vec::new();
            for provider in selected {
                let adapter = match self.adapter_for(&provider) {
                    Ok(adapter) => adapter,
                    Err(error) => {
                        return ProviderTrustPreparationResult::Waiting {
                            waiting: self
                                .gate_waiting_unordered_spawned(
                                    &project_id,
                                    &operation_id,
                                    &lc_id,
                                    &provider,
                                    &canonical_root,
                                    "",
                                    error,
                                    scenario,
                                )
                                .await,
                        };
                    }
                };
                if let Err(error) = Self::validate_root(&canonical_root) {
                    return ProviderTrustPreparationResult::Waiting {
                        waiting: self
                            .gate_waiting_unordered_spawned(
                                &project_id,
                                &operation_id,
                                &lc_id,
                                &provider,
                                &canonical_root,
                                &adapter.trust_key(&canonical_root),
                                error,
                                scenario,
                            )
                            .await,
                    };
                }
                let home_lock =
                    match Self::acquire_home_lock_async(adapter.as_ref(), &canonical_root).await {
                        Ok(lock) => lock,
                        Err(error) => {
                            return ProviderTrustPreparationResult::Waiting {
                                waiting: self
                                    .gate_waiting_unordered_spawned(
                                        &project_id,
                                        &operation_id,
                                        &lc_id,
                                        &provider,
                                        &canonical_root,
                                        &adapter.trust_key(&canonical_root),
                                        error,
                                        scenario,
                                    )
                                    .await,
                            };
                        }
                    };
                // 临界区同步 IO 经 spawn_blocking；caller 取消观察后本段
                // detached 安全收尾（提交边界）。
                let section_registry = Arc::clone(&self);
                let adapter_trust_key = adapter.trust_key(&canonical_root);
                let section_project = project_id.clone();
                let section_operation = operation_id.clone();
                let section_lc = lc_id.clone();
                let section_root = canonical_root.clone();
                let section_provider = provider.clone();
                #[allow(clippy::result_large_err)]
                let section = tokio::task::spawn_blocking(move || {
                    section_registry.ensure_provider_critical_section(
                        home_lock,
                        &section_project,
                        &section_operation,
                        &section_lc,
                        &section_provider,
                        &section_root,
                        adapter,
                        scenario,
                    )
                });
                match section.await {
                    Ok(Ok(registration)) => registrations.push(registration),
                    Ok(Err(waiting)) => {
                        return ProviderTrustPreparationResult::Waiting { waiting };
                    }
                    Err(join_error) => {
                        let error = ProviderTrustError::transient(
                            "trust_section_failed",
                            format!("home trust critical section aborted: {join_error}"),
                        );
                        return ProviderTrustPreparationResult::Waiting {
                            waiting: self
                                .gate_waiting_unordered_spawned(
                                    &project_id,
                                    &operation_id,
                                    &lc_id,
                                    &provider,
                                    &canonical_root,
                                    &adapter_trust_key,
                                    error,
                                    scenario,
                                )
                                .await,
                        };
                    }
                }
            }
            ProviderTrustPreparationResult::Ready { registrations }
        })
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::locking::{
        ExclusiveFileLock, LockAttemptHookGuard, register_lock_attempt_hook,
        with_exact_exclusive_lock,
    };
    use crate::product::json_store::ProductStoreError;
    use crate::product::logical_codebase::provider_trust_adapters::{
        CodexTrustAdapter, KimiTrustAdapter, ProviderTrustArtifactHint, ProviderTrustEntryState,
        ProviderTrustHomeAdapter, ProviderTrustWriteOutcome,
    };
    use crate::product::logical_codebase::provider_trust_store::{
        ProviderTrustOwnershipRecord, ProviderTrustStore, ProviderTrustWaitingRecord, trust_now,
    };
    use crate::product::models::ProviderName;

    use super::{
        HomeBackedProviderTrustRegistry, ProviderTrustAction, ProviderTrustError,
        ProviderTrustOwnership, ProviderTrustPrecondition, ProviderTrustPreparationResult,
        ProviderTrustRegistry, ProviderTrustRevocationOutcome, ProviderTrustScenario,
        ReadonlyProviderTrustSource,
    };

    const PROJECT: &str = "project_0001";
    const LC: &str = "logical_codebase_0001";
    const OPERATION: &str = "aggregate_initialization_0001";

    struct TrustFixture {
        _home: tempfile::TempDir,
        aria: tempfile::TempDir,
        registry: HomeBackedProviderTrustRegistry,
        root: PathBuf,
    }

    impl TrustFixture {
        fn new() -> Self {
            let home = tempfile::TempDir::new().unwrap();
            let aria = tempfile::TempDir::new().unwrap();
            let root = home.path().join("lc-root");
            std::fs::create_dir_all(&root).unwrap();
            let registry = HomeBackedProviderTrustRegistry::new(
                ProductAppPaths::new(aria.path().to_path_buf()),
                vec![
                    Arc::new(CodexTrustAdapter::for_home(home.path())),
                    Arc::new(KimiTrustAdapter::for_home(home.path())),
                ],
            );
            Self {
                _home: home,
                aria,
                registry,
                root,
            }
        }

        fn home(&self) -> &Path {
            self._home.path()
        }

        fn codex_config(&self) -> PathBuf {
            self.home().join(".codex").join("config.toml")
        }

        fn kimi_trust_dir(&self) -> PathBuf {
            self.home().join(".kimi-code").join("workspace-trust")
        }

        fn kimi_key(&self) -> String {
            KimiTrustAdapter::for_home(self.home()).trust_key(&self.root)
        }

        fn store(&self) -> ProviderTrustStore {
            ProviderTrustStore::for_lc(ProductAppPaths::new(self.aria.path().to_path_buf()), LC)
        }
    }

    #[test]
    fn provider_trust_ensure_registers_codex_and_kimi_entries_idempotently() {
        let fixture = TrustFixture::new();
        let providers = [ProviderName::Codex, ProviderName::KimiCode];

        let first = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
            ProviderTrustScenario::Recipe,
        );
        let ProviderTrustPreparationResult::Ready { registrations } = first else {
            panic!("expected the trust gate to become ready");
        };
        assert_eq!(registrations.len(), 2);

        // Codex：用户级 config.toml 出现 [projects."<root>"] trust_level="trusted"。
        let config = std::fs::read_to_string(fixture.codex_config()).unwrap();
        let header = format!("[projects.\"{}\"]", fixture.root.display());
        assert!(config.contains(&header));
        assert!(config.contains("trust_level = \"trusted\""));

        // Kimi：workspace-trust 出现 wd_<basename>_<sha256[:12]> 记录。
        let kimi_key = fixture.kimi_key();
        assert_eq!(
            kimi_key,
            format!(
                "wd_lc-root_{}",
                sha256_hex_12(&fixture.root.display().to_string())
            )
        );
        let kimi_path = fixture.kimi_trust_dir().join(&kimi_key);
        let kimi_raw = std::fs::read_to_string(&kimi_path).unwrap();
        let kimi: serde_json::Value = serde_json::from_str(&kimi_raw).unwrap();
        assert_eq!(kimi["root"], fixture.root.display().to_string());
        assert!(kimi["trustedAt"].as_u64().unwrap() > 0);

        // 重复 ensure 幂等：home 文件字节不变、块不重复、无重写。
        let second = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
            ProviderTrustScenario::Recipe,
        );
        let ProviderTrustPreparationResult::Ready { registrations } = second else {
            panic!("expected the replayed trust gate to stay ready");
        };
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            config
        );
        assert_eq!(std::fs::read_to_string(&kimi_path).unwrap(), kimi_raw);
        assert_eq!(config.matches(&header).count(), 1);
        assert!(registrations.iter().all(|registration| {
            matches!(
                registration.action,
                ProviderTrustAction::Replay | ProviderTrustAction::ReuseUserEntry
            )
        }));

        // 核验面保持 trusted。
        for provider in providers {
            let verification = fixture
                .registry
                .verify_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap();
            assert!(
                verification.trusted,
                "expected {provider:?} to stay trusted"
            );
        }
    }

    #[test]
    fn provider_trust_failure_enters_retryable_waiting_without_fallback() {
        let fixture = TrustFixture::new();
        // 写入失败注入：该根的 workspace-trust 记录路径被目录占用（父目录
        // 完好、sidecar 锁文件可创建；与权限无关、任何环境都稳定失败）。
        std::fs::create_dir_all(fixture.kimi_trust_dir()).unwrap();
        std::fs::create_dir(fixture.kimi_trust_dir().join(fixture.kimi_key())).unwrap();

        let providers = [ProviderName::Codex, ProviderName::KimiCode];
        let outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
            ProviderTrustScenario::Recipe,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected the trust gate to enter fail-closed waiting");
        };
        assert_eq!(waiting.provider, ProviderName::KimiCode);
        assert!(!waiting.reason_code.is_empty());
        assert!(waiting.reason_code.starts_with("kimi_"));
        assert!(!waiting.retry_action.is_empty());
        assert_eq!(waiting.canonical_root, fixture.root);

        // durable waiting fact 已落盘且可由 store 关联。
        let waiting_facts = fixture.store().list_waiting(PROJECT).unwrap();
        assert!(waiting_facts.iter().any(|fact| {
            fact.reason_code == waiting.reason_code
                && fact.trust_key == waiting.trust_key
                && fact.lc_id == LC
                && fact.operation_id.as_deref() == Some(OPERATION)
        }));

        // 不降级：Codex 已先成功登记，gate 仍不是 Ready、不部分放行。
        assert!(
            std::fs::read_to_string(fixture.codex_config())
                .unwrap()
                .contains("trust_level = \"trusted\"")
        );

        // 可重试：移除阻断后重试转 Ready，并清空 waiting fact。
        std::fs::remove_dir(fixture.kimi_trust_dir().join(fixture.kimi_key())).unwrap();
        let retried = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
            ProviderTrustScenario::Recipe,
        );
        assert!(matches!(
            retried,
            ProviderTrustPreparationResult::Ready { .. }
        ));
        assert!(fixture.store().list_waiting(PROJECT).unwrap().is_empty());
    }

    #[test]
    fn provider_trust_revoke_removes_entries_and_preserves_user_trust() {
        let fixture = TrustFixture::new();
        // 用户既有 trust：另一 root 的 Codex 条目 + 其余 TOML 内容 + 另一 Kimi 记录。
        let user_root = fixture.home().join("user-project");
        std::fs::create_dir_all(&user_root).unwrap();
        std::fs::create_dir_all(fixture.codex_config().parent().unwrap()).unwrap();
        std::fs::write(
            fixture.codex_config(),
            format!(
                "[projects.\"{}\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.spike]\ncommand = \"npx\"\n",
                user_root.display()
            ),
        )
        .unwrap();
        let user_kimi_key = KimiTrustAdapter::for_home(fixture.home()).trust_key(&user_root);
        std::fs::create_dir_all(fixture.kimi_trust_dir()).unwrap();
        std::fs::write(
            fixture.kimi_trust_dir().join(&user_kimi_key),
            format!("{{\"root\":\"{}\",\"trustedAt\":111}}", user_root.display()),
        )
        .unwrap();
        let user_kimi_before =
            std::fs::read_to_string(fixture.kimi_trust_dir().join(&user_kimi_key)).unwrap();

        let providers = [ProviderName::Codex, ProviderName::KimiCode];
        assert!(matches!(
            fixture.registry.ensure_ready(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &providers,
                ProviderTrustScenario::Recipe,
            ),
            ProviderTrustPreparationResult::Ready { .. }
        ));
        for provider in providers {
            let revocation = fixture
                .registry
                .revoke_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap();
            assert_eq!(revocation.outcome, ProviderTrustRevocationOutcome::Removed);
        }

        // 本 LC 根条目被撤销，用户条目逐字节保留。
        let codex_after = std::fs::read_to_string(fixture.codex_config()).unwrap();
        assert!(!codex_after.contains(&format!("[projects.\"{}\"]", fixture.root.display())));
        assert!(!fixture.kimi_trust_dir().join(fixture.kimi_key()).exists());
        assert!(codex_after.contains(&format!("[projects.\"{}\"]", user_root.display())));
        assert_eq!(codex_after.matches("[mcp_servers.spike]").count(), 1);
        assert!(codex_after.contains("command = \"npx\""));
        assert_eq!(
            std::fs::read_to_string(fixture.kimi_trust_dir().join(&user_kimi_key)).unwrap(),
            user_kimi_before
        );

        // 用户自建的本根条目：登记时原样复用、撤销时 NotOwned 且文件不动。
        std::fs::write(
            fixture.codex_config(),
            format!(
                "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
                fixture.root.display()
            ),
        )
        .unwrap();
        let user_pre_trusted = std::fs::read_to_string(fixture.codex_config()).unwrap();
        assert!(matches!(
            fixture.registry.ensure_ready(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &[ProviderName::Codex],
                ProviderTrustScenario::Recipe,
            ),
            ProviderTrustPreparationResult::Ready { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            user_pre_trusted
        );
        let revocation = fixture
            .registry
            .revoke_trusted(PROJECT, LC, &ProviderName::Codex, &fixture.root)
            .unwrap();
        assert_eq!(revocation.outcome, ProviderTrustRevocationOutcome::NotOwned);
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            user_pre_trusted
        );
    }

    #[test]
    fn provider_trust_audit_records_before_after_digests() {
        let fixture = TrustFixture::new();
        let providers = [ProviderName::Codex, ProviderName::KimiCode];
        assert!(matches!(
            fixture.registry.ensure_ready(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &providers,
                ProviderTrustScenario::Recipe,
            ),
            ProviderTrustPreparationResult::Ready { .. }
        ));

        let audits = fixture.store().list_audit(PROJECT).unwrap();
        let ensure_audits: Vec<_> = audits.iter().filter(|a| a.action == "ensure").collect();
        assert_eq!(ensure_audits.len(), 2);
        for audit in &ensure_audits {
            assert_eq!(audit.lc_id, LC);
            assert_eq!(audit.operation_id.as_deref(), Some(OPERATION));
            assert_eq!(audit.canonical_root, fixture.root);
            assert!(!audit.trust_key.is_empty());
            assert!(
                audit
                    .before_digest
                    .as_ref()
                    .is_none_or(|digest| digest.starts_with("sha256:"))
            );
            let after = audit.after_digest.as_ref().expect("ensure after digest");
            assert!(after.starts_with("sha256:"));
            assert_ne!(audit.before_digest.as_ref(), Some(after));
            assert!(!audit.recorded_at.is_empty());
        }

        for provider in providers {
            fixture
                .registry
                .revoke_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap();
        }
        let audits = fixture.store().list_audit(PROJECT).unwrap();
        let revoke_audits: Vec<_> = audits.iter().filter(|a| a.action == "revoke").collect();
        assert_eq!(revoke_audits.len(), 2);
        for audit in &revoke_audits {
            // 撤销前摘要必在；撤销后工件可能整体消失（Kimi 记录文件被删），
            // 此时 after 为 None，其余情况必为 sha256 摘要。
            assert!(audit.before_digest.as_ref().unwrap().starts_with("sha256:"));
            assert!(
                audit
                    .after_digest
                    .as_ref()
                    .is_none_or(|digest| digest.starts_with("sha256:"))
            );
        }

        // 审计只含 digest/path/key 事实，不携带敏感凭据体积。
        for audit in &audits {
            assert!(serde_json::to_string(audit).unwrap().len() < 1024);
        }
    }

    /// Task 2.1：确认场景的 retry_action 必须是「…重新确认」而不是
    /// recipe 专属的「重试初始化」；recipe 场景对照保持原文案，
    /// reason_code 不因场景折叠。
    #[test]
    fn provider_trust_confirmation_scenario_directs_explicit_reconfirmation() {
        let fixture = TrustFixture::new();
        std::fs::create_dir_all(fixture.kimi_trust_dir()).unwrap();
        std::fs::create_dir(fixture.kimi_trust_dir().join(fixture.kimi_key())).unwrap();
        let providers = [ProviderName::KimiCode];
        let waiting_for = |scenario| match fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &providers,
            scenario,
        ) {
            ProviderTrustPreparationResult::Waiting { waiting } => waiting,
            other => panic!("expected fail-closed waiting, got {other:?}"),
        };

        let confirmation = waiting_for(ProviderTrustScenario::Confirmation);
        assert!(
            confirmation.retry_action.contains("重新确认"),
            "confirmation waiting must direct an explicit re-confirm: {}",
            confirmation.retry_action
        );
        assert!(!confirmation.retry_action.contains("重试初始化"));
        assert!(confirmation.reason_code.starts_with("kimi_"));

        let recipe = waiting_for(ProviderTrustScenario::Recipe);
        assert!(recipe.retry_action.contains("重试初始化"));
        assert!(!recipe.retry_action.contains("重新确认"));
        assert_eq!(recipe.reason_code, confirmation.reason_code);
    }

    /// 6.1：只读 generation 投影（`ReadonlyProviderTrustSource`）——entry
    /// 缺失映射 `trust_entry_missing` 分型，retry_action 是「人工登记…
    /// 显式重新生成」（不混 recipe/confirmation 动作），manual_guidance
    /// 同源携带 adapter 真实工件路径与稳定键；条目登记后 `None`（不再
    /// 给指导）；全程零 durable 写（waiting/audit 空）。
    #[test]
    fn provider_trust_generation_scenario_projects_read_only_missing_guidance() {
        let fixture = TrustFixture::new();
        let source = ReadonlyProviderTrustSource::new(
            ProductAppPaths::new(fixture.aria.path().to_path_buf()),
            Some(LC.to_string()),
            vec![
                Arc::new(CodexTrustAdapter::for_home(fixture.home())),
                Arc::new(KimiTrustAdapter::for_home(fixture.home())),
            ],
        );

        // 无需 workspace trust 的 provider 无指导事实。
        assert!(
            source
                .scenario_waiting(
                    &ProviderName::ClaudeCode,
                    &fixture.root,
                    ProviderTrustScenario::Generation
                )
                .is_none()
        );

        let waiting = source
            .scenario_waiting(
                &ProviderName::KimiCode,
                &fixture.root,
                ProviderTrustScenario::Generation,
            )
            .expect("missing entry must project a read-only generation waiting");
        assert_eq!(waiting.reason_code, "trust_entry_missing");
        assert!(waiting.message.contains("entry absent"));
        assert!(
            waiting.retry_action.contains("人工登记")
                && waiting.retry_action.contains("显式重新生成"),
            "generation retry_action must direct manual registration then explicit re-generation: {}",
            waiting.retry_action
        );
        assert!(!waiting.retry_action.contains("重新确认"));
        assert!(!waiting.retry_action.contains("重试初始化"));
        // 同源指导：Kimi 真实工件目录 + 稳定键（与 adapter 算法同源）。
        assert!(
            waiting
                .manual_guidance
                .contains(&fixture.kimi_trust_dir().display().to_string())
        );
        assert!(waiting.manual_guidance.contains(&fixture.kimi_key()));
        assert_eq!(waiting.trust_key, fixture.kimi_key());

        // 人工登记后：只读投影不再给指导（None），且此前零 durable 写。
        assert!(fixture.store().list_waiting(PROJECT).unwrap().is_empty());
        assert!(fixture.store().list_audit(PROJECT).unwrap().is_empty());
        std::fs::create_dir_all(fixture.kimi_trust_dir()).unwrap();
        std::fs::write(
            fixture.kimi_trust_dir().join(fixture.kimi_key()),
            serde_json::json!({"root": fixture.root.display().to_string(), "trustedAt": 1_u64})
                .to_string(),
        )
        .unwrap();
        assert!(
            source
                .scenario_waiting(
                    &ProviderName::KimiCode,
                    &fixture.root,
                    ProviderTrustScenario::Generation
                )
                .is_none()
        );
        assert!(fixture.store().list_waiting(PROJECT).unwrap().is_empty());
        assert!(fixture.store().list_audit(PROJECT).unwrap().is_empty());
    }

    /// 指引卡简化（owner 2026-10-12 直批）：等待面结构化携带
    /// `command_markdown`——entry 缺失分型＝adapter 同源 `manual_command`
    /// （Kimi 单行 shell／Codex TOML 片段，前端命令窗直渲不解析平铺
    /// 串）；用户值冲突分型 None（一键命令不得弱化「不覆盖」纪律）。
    #[test]
    fn provider_trust_waiting_structures_command_markdown_same_source_as_adapter() {
        let fixture = TrustFixture::new();
        let source = ReadonlyProviderTrustSource::new(
            ProductAppPaths::new(fixture.aria.path().to_path_buf()),
            Some(LC.to_string()),
            vec![
                Arc::new(CodexTrustAdapter::for_home(fixture.home())),
                Arc::new(KimiTrustAdapter::for_home(fixture.home())),
            ],
        );

        // entry 缺失（Kimi）：command_markdown 与 adapter manual_command 同源。
        let kimi = KimiTrustAdapter::for_home(fixture.home());
        let waiting = source
            .scenario_waiting(
                &ProviderName::KimiCode,
                &fixture.root,
                ProviderTrustScenario::Generation,
            )
            .expect("missing entry must project a read-only waiting");
        let expected_command = kimi
            .manual_hint(&fixture.root)
            .manual_command
            .expect("adapter hint carries the kimi registration command");
        assert_eq!(
            waiting.command_markdown.as_deref(),
            Some(expected_command.as_str()),
            "command_markdown must be the same-source adapter manual_command, got: {:?}",
            waiting.command_markdown
        );
        assert!(
            waiting
                .command_markdown
                .as_deref()
                .unwrap_or_default()
                .contains("mkdir -p"),
            "kimi command markdown is the copyable one-line shell"
        );

        // 用户值冲突（Codex 预置非 trusted 值）：结构化命令必须缺席。
        std::fs::create_dir_all(fixture.codex_config().parent().unwrap()).unwrap();
        let codex = CodexTrustAdapter::for_home(fixture.home());
        let header = codex.trust_key(&fixture.root);
        std::fs::write(
            fixture.codex_config(),
            format!("{header}\ntrust_level = \"untrusted\"\n"),
        )
        .unwrap();
        let conflict = source
            .scenario_waiting(
                &ProviderName::Codex,
                &fixture.root,
                ProviderTrustScenario::Generation,
            )
            .expect("user conflict must project a read-only waiting");
        assert_eq!(
            conflict.reason_code, "codex_trust_key_conflict",
            "conflict kind must reach the waiting surface"
        );
        assert!(
            conflict.command_markdown.is_none(),
            "conflict must not carry a one-shot registration command: {:?}",
            conflict.command_markdown
        );
    }

    /// Task 2.1：Claude-only recipe 零登记行为不变——无需 workspace trust
    /// 的 provider 不触碰 home 工件，也不产生 waiting/audit 事实。
    #[test]
    fn provider_trust_claude_only_recipe_stays_zero_registration() {
        let fixture = TrustFixture::new();
        let outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &[ProviderName::ClaudeCode],
            ProviderTrustScenario::Recipe,
        );
        let ProviderTrustPreparationResult::Ready { registrations } = outcome else {
            panic!("claude-only recipe must stay ready without trust registration");
        };
        assert!(registrations.is_empty());
        // Task 3.1：无 trust provider 不取锁——home 下不出现任何
        // sidecar 锁文件（连同其父目录都不创建）。
        assert!(!fixture.codex_config().exists());
        assert!(!fixture.kimi_trust_dir().exists());
        assert!(!fixture.home().join(".codex").exists());
        assert!(!fixture.home().join(".kimi-code").exists());
        assert!(fixture.store().list_waiting(PROJECT).unwrap().is_empty());
        assert!(fixture.store().list_audit(PROJECT).unwrap().is_empty());
    }

    /// Task 2.1：中立化不改过滤/去重算法——重复与无需 trust 的 provider
    /// 只登记一次有效项。
    #[test]
    fn provider_trust_selected_providers_are_filtered_and_deduplicated() {
        let fixture = TrustFixture::new();
        let providers = [
            ProviderName::Codex,
            ProviderName::ClaudeCode,
            ProviderName::Codex,
        ];
        let ProviderTrustPreparationResult::Ready { registrations } =
            fixture.registry.ensure_ready(
                PROJECT,
                OPERATION,
                LC,
                &fixture.root,
                &providers,
                ProviderTrustScenario::Recipe,
            )
        else {
            panic!("expected ready");
        };
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].provider, ProviderName::Codex);
    }

    /// Task 2.2：Codex 指导的键来自 adapter 自己的 trust_key（同源 TOML
    /// 转义）；根含引号/反斜杠时不给未转义原文当键，冲突不引导一键覆盖。
    #[test]
    fn provider_trust_codex_guidance_uses_adapter_escaped_key_for_special_roots() {
        let fixture = TrustFixture::new();
        let special = fixture.home().join("lc\"root\\path");
        std::fs::create_dir_all(&special).unwrap();
        std::fs::create_dir_all(fixture.codex_config().parent().unwrap()).unwrap();
        let adapter = CodexTrustAdapter::for_home(fixture.home());
        let escaped_header = adapter.trust_key(&special);
        let naive_header = format!("[projects.\"{}\"]", special.display());
        assert_ne!(
            escaped_header, naive_header,
            "special root must exercise TOML escaping"
        );
        std::fs::write(
            fixture.codex_config(),
            format!("{escaped_header}\ntrust_level = \"untrusted\"\n"),
        )
        .unwrap();

        let outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &special,
            &[ProviderName::Codex],
            ProviderTrustScenario::Confirmation,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected conflict waiting for the special root");
        };
        assert_eq!(waiting.reason_code, "codex_trust_key_conflict");
        assert!(waiting.retry_action.contains("重新确认"));
        assert!(
            waiting.manual_guidance.contains(&escaped_header),
            "guidance must carry the adapter-escaped key: {}",
            waiting.manual_guidance
        );
        assert!(
            waiting.manual_guidance.contains("不覆盖"),
            "conflict guidance must not read as a one-click overwrite: {}",
            waiting.manual_guidance
        );
        assert!(
            !waiting.manual_guidance.contains(&naive_header),
            "guidance must not present the unescaped root as a TOML key"
        );
    }

    /// Task 2.2：Kimi 指导携带实际 key、实际记录路径与记录 JSON 形状
    /// （serde 同源转义，不手抄 key/JSON 算法）。
    #[test]
    fn provider_trust_kimi_guidance_carries_actual_key_path_and_json_shape() {
        let fixture = TrustFixture::new();
        let adapter = KimiTrustAdapter::for_home(fixture.home());
        let record_path = adapter.record_path(&fixture.root);
        std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
        std::fs::write(&record_path, "not-json").unwrap();

        let outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &[ProviderName::KimiCode],
            ProviderTrustScenario::Confirmation,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected format-failure waiting");
        };
        assert_eq!(waiting.reason_code, "kimi_trust_record_invalid");
        let key = adapter.trust_key(&fixture.root);
        assert!(
            waiting.manual_guidance.contains(&key),
            "guidance must carry the actual trust key {key}: {}",
            waiting.manual_guidance
        );
        assert!(
            waiting
                .manual_guidance
                .contains(&record_path.display().to_string())
        );
        assert!(waiting.manual_guidance.contains("\"root\""));
        assert!(waiting.manual_guidance.contains("\"trustedAt\""));
        assert!(
            waiting
                .manual_guidance
                .contains(&fixture.root.display().to_string())
        );
    }

    /// 指引卡登记命令（owner 2026-10-12 直批）：Kimi 的 entry 缺失指导附
    /// 可直接复制的单行 shell（mkdir + printf 记录 + date 毫秒时间戳），
    /// 路径/键/root 全部取 adapter 真实值（同源，不手抄算法）。
    #[test]
    fn provider_trust_kimi_guidance_carries_copyable_registration_command() {
        let fixture = TrustFixture::new();
        let adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>> = vec![
            Arc::new(CodexTrustAdapter::for_home(fixture.home())),
            Arc::new(KimiTrustAdapter::for_home(fixture.home())),
        ];
        let kimi = KimiTrustAdapter::for_home(fixture.home());
        let record_path = kimi.record_path(&fixture.root);
        let error = ProviderTrustError::transient(
            super::TRUST_ENTRY_MISSING_CODE,
            "canonical root 下未找到 kimi_code 的用户级 workspace trust 条目（entry absent）"
                .to_string(),
        );
        let guidance = super::provider_trust_manual_guidance(
            &adapters,
            &ProviderName::KimiCode,
            &fixture.root,
            &error,
            ProviderTrustScenario::Generation,
        );
        let expected_command = format!(
            "mkdir -p '{}' && printf '{{\"root\":\"{}\",\"trustedAt\":%s}}' \"$(date +%s%3N)\" > '{}' # GNU/Linux（date +%s%3N 需 GNU date）",
            kimi.workspace_trust_root().display(),
            fixture.root.display(),
            record_path.display(),
        );
        assert!(
            guidance.contains(&expected_command),
            "guidance must carry the exact copyable command {expected_command}: {guidance}"
        );
    }

    /// P3 硬化（k3 审查 2026-10-12）：root 含 `%` 时 printf 格式串必须
    /// 转义（字面 % → %%，保留唯一 %s 占位），用户复制执行才不写坏 JSON；
    /// 命令尾部注明 GNU/Linux（`date +%s%3N` 需 GNU date，macOS/BSD 失效）。
    #[test]
    fn provider_trust_kimi_command_escapes_percent_and_notes_platform() {
        let fixture = TrustFixture::new();
        let special_root = fixture.home().join("100%s-%d-root");
        std::fs::create_dir_all(&special_root).unwrap();
        let adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>> = vec![
            Arc::new(CodexTrustAdapter::for_home(fixture.home())),
            Arc::new(KimiTrustAdapter::for_home(fixture.home())),
        ];
        let kimi = KimiTrustAdapter::for_home(fixture.home());
        let error = ProviderTrustError::transient(
            super::TRUST_ENTRY_MISSING_CODE,
            "entry absent".to_string(),
        );
        let guidance = super::provider_trust_manual_guidance(
            &adapters,
            &ProviderName::KimiCode,
            &special_root,
            &error,
            ProviderTrustScenario::Generation,
        );

        // 转义后的 payload 逐字断言：root 中字面 % 全部 %%，占位符保持唯一 %s。
        let expected_payload = format!(
            "{{\"root\":\"{}\",\"trustedAt\":%s}}",
            special_root.display().to_string().replace('%', "%%"),
        );
        assert!(
            guidance.contains(&expected_payload),
            "printf payload must escape literal % in root: {guidance}"
        );
        assert!(
            guidance.contains("# GNU/Linux"),
            "command tail must note the GNU date dependency: {guidance}"
        );

        // printf 语义回放：格式串中裸 %s 恰一处；按 %%→%、%s←参数渲染后，
        // 产物必须是合法 Kimi 记录 JSON（root 原样、trustedAt 为数字）。
        let command = guidance
            .lines()
            .find(|line| line.contains("printf"))
            .expect("copyable command line");
        let format_str = command
            .split("printf '")
            .nth(1)
            .expect("printf format open")
            .split("' \"$(date")
            .next()
            .expect("printf format close");
        // 忠实 printf walker：%%→字面 %，%s←参数；其余 % 指令视为事故。
        let mut rendered = String::new();
        let mut placeholder_count = 0usize;
        let mut chars = format_str.chars();
        while let Some(current) = chars.next() {
            if current != '%' {
                rendered.push(current);
                continue;
            }
            match chars.next() {
                Some('%') => rendered.push('%'),
                Some('s') => {
                    placeholder_count += 1;
                    rendered.push_str("1760000000000");
                }
                Some(other) => panic!("unexpected printf directive %{other}"),
                None => panic!("dangling % in printf format"),
            }
        }
        assert_eq!(placeholder_count, 1, "exactly one %s placeholder: {format_str}");
        let record: serde_json::Value = serde_json::from_str(&rendered)
            .expect("rendered command output must be valid Kimi record JSON");
        assert_eq!(record["root"], special_root.display().to_string());
        assert_eq!(record["trustedAt"], 1760000000000u64);
        drop(kimi);
    }

    /// 指引卡登记命令：Codex 的 entry 缺失指导附 config.toml 路径与完整
    /// [projects."<root>"] trust_level="trusted" TOML 片段（TOML 不可一行
    /// 安全写，按「片段+路径」给）。
    #[test]
    fn provider_trust_codex_guidance_carries_config_snippet_and_path() {
        let fixture = TrustFixture::new();
        let adapters: Vec<Arc<dyn ProviderTrustHomeAdapter>> = vec![
            Arc::new(CodexTrustAdapter::for_home(fixture.home())),
            Arc::new(KimiTrustAdapter::for_home(fixture.home())),
        ];
        let codex = CodexTrustAdapter::for_home(fixture.home());
        let error = ProviderTrustError::transient(
            super::TRUST_ENTRY_MISSING_CODE,
            "canonical root 下未找到 codex 的用户级 workspace trust 条目（entry absent）"
                .to_string(),
        );
        let guidance = super::provider_trust_manual_guidance(
            &adapters,
            &ProviderName::Codex,
            &fixture.root,
            &error,
            ProviderTrustScenario::Generation,
        );
        let header = codex.trust_key(&fixture.root);
        let expected_snippet =
            format!("在 {} 追加（或并入既有表）：\n{header}\ntrust_level = \"trusted\"", codex.config_path().display());
        assert!(
            guidance.contains(&expected_snippet),
            "guidance must carry the exact config snippet at the real path: {guidance}"
        );
    }

    /// 指引卡登记命令：用户值冲突（UserConflict）不给一键登记命令——避免
    /// 把「不覆盖用户条目」的指导弱化成覆盖指令。
    #[test]
    fn provider_trust_conflict_guidance_does_not_carry_registration_command() {
        let fixture = TrustFixture::new();
        let error = ProviderTrustError::conflict("codex_trust_key_conflict", "conflict".into());
        let guidance = super::provider_trust_manual_guidance(
            &[
                Arc::new(CodexTrustAdapter::for_home(fixture.home())) as Arc<dyn ProviderTrustHomeAdapter>,
                Arc::new(KimiTrustAdapter::for_home(fixture.home())),
            ],
            &ProviderName::Codex,
            &fixture.root,
            &error,
            ProviderTrustScenario::Generation,
        );
        assert!(
            !guidance.contains("可直接复制的登记命令") && !guidance.contains("mkdir -p"),
            "conflict guidance must not suggest a one-shot overwrite: {guidance}"
        );
    }

    /// 显式按钮授权场景（ExplicitRegistration）的等待面 retry_action 指回
    /// 按钮本身（重新点击「登记并重试」），不借用生成/确认场景文案。
    #[test]
    fn provider_trust_explicit_registration_waiting_retry_action_targets_the_button() {
        let fixture = TrustFixture::new();
        std::fs::create_dir_all(fixture.codex_config().parent().unwrap()).unwrap();
        let header =
            CodexTrustAdapter::for_home(fixture.home()).trust_key(&fixture.root);
        std::fs::write(
            fixture.codex_config(),
            format!("{header}\ntrust_level = \"untrusted\"\n"),
        )
        .unwrap();

        let outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &[ProviderName::Codex],
            ProviderTrustScenario::ExplicitRegistration,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected conflict waiting");
        };
        assert_eq!(waiting.reason_code, "codex_trust_key_conflict");
        assert_eq!(
            waiting.retry_action,
            "用户自行核对该 trust 键的取值后重新点击「登记并重试」；系统不覆盖用户条目"
        );
    }

    /// Task 2.2：home 写失败与 fact IO 失败必须分型——前者指导用户级
    /// 工件修复，后者指明修服务端 trust fact store，互不误导。
    #[test]
    fn provider_trust_waiting_separates_home_write_from_fact_store_failures() {
        // home 侧失败：workspace-trust 目录只读（非 root 下 chmod 必生效）
        // → sidecar 锁文件不可创建，锁失败 fail-closed。
        let fixture = TrustFixture::new();
        let kimi_dir = fixture.kimi_trust_dir();
        std::fs::create_dir_all(&kimi_dir).unwrap();
        let original = std::fs::metadata(&kimi_dir).unwrap().permissions();
        let mut readonly = original.clone();
        std::os::unix::fs::PermissionsExt::set_mode(&mut readonly, 0o500);
        std::fs::set_permissions(&kimi_dir, readonly).unwrap();
        let home_outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &[ProviderName::KimiCode],
            ProviderTrustScenario::Confirmation,
        );
        std::fs::set_permissions(&kimi_dir, original).unwrap();
        match home_outcome {
            ProviderTrustPreparationResult::Waiting { waiting } => {
                // Task 3.1：锁文件不可创建时以 trust_home_lock_failed
                // fail-closed，仍分型为用户级工件问题。
                assert_eq!(waiting.reason_code, "trust_home_lock_failed");
                assert!(
                    waiting.retry_action.contains("用户级 trust 配置"),
                    "home failure retry action must target the user-level artifact: {}",
                    waiting.retry_action
                );
                assert!(waiting.manual_guidance.contains("workspace-trust"));
                assert!(
                    !waiting.manual_guidance.contains("fact store"),
                    "home failure must not be redirected at the server-side store: {}",
                    waiting.manual_guidance
                );
            }
            ProviderTrustPreparationResult::Ready { .. } => {
                assert_eq!(
                    unsafe { libc::geteuid() },
                    0,
                    "write-failure injection relies on chmod biting (requires non-root)"
                );
            }
        }

        // fact IO 失败：aria 根被普通文件占用 → durable store 读写失败，
        // 指导与服务面动作都指向 trust fact store，不给 home 工件指引。
        let home = tempfile::TempDir::new().unwrap();
        let aria = tempfile::TempDir::new().unwrap();
        let root = home.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        let aria_as_file = aria.path().join("aria-is-a-file");
        std::fs::write(&aria_as_file, b"not-a-dir").unwrap();
        let registry = HomeBackedProviderTrustRegistry::new(
            crate::product::app_paths::ProductAppPaths::new(aria_as_file.clone()),
            vec![Arc::new(KimiTrustAdapter::for_home(home.path()))],
        );
        let outcome = registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &root,
            &[ProviderName::KimiCode],
            ProviderTrustScenario::Confirmation,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected fact-store failure waiting");
        };
        assert_eq!(waiting.reason_code, "trust_store_io");
        assert!(
            waiting.retry_action.contains("trust fact store"),
            "fact IO retry action must target the store: {}",
            waiting.retry_action
        );
        assert!(
            !waiting.retry_action.contains("用户级 trust 配置"),
            "fact IO must not masquerade as a user-home fix"
        );
        assert!(waiting.manual_guidance.contains("trust fact store"));
        assert!(
            !waiting.manual_guidance.contains("workspace-trust"),
            "fact IO guidance must not send the user at the home artifact: {}",
            waiting.manual_guidance
        );
    }

    /// Task 2.2：无 adapter 只给装配修复——不伪造 home 工件指引，场景
    /// retry_action 不因分型折叠。
    #[test]
    fn provider_trust_missing_adapter_guidance_targets_server_assembly() {
        let home = tempfile::TempDir::new().unwrap();
        let aria = tempfile::TempDir::new().unwrap();
        let root = home.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        let registry = HomeBackedProviderTrustRegistry::new(
            crate::product::app_paths::ProductAppPaths::new(aria.path().to_path_buf()),
            // 只装配 Kimi adapter：Codex 无 adapter。
            vec![Arc::new(KimiTrustAdapter::for_home(home.path()))],
        );
        let confirmation = match registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &root,
            &[ProviderName::Codex],
            ProviderTrustScenario::Confirmation,
        ) {
            ProviderTrustPreparationResult::Waiting { waiting } => waiting,
            other => panic!("expected no-adapter waiting, got {other:?}"),
        };
        assert_eq!(confirmation.reason_code, "unsupported_trust_provider");
        assert!(
            confirmation.manual_guidance.contains("装配"),
            "no-adapter guidance must target assembly: {}",
            confirmation.manual_guidance
        );
        assert!(
            !confirmation.manual_guidance.contains(".codex"),
            "no adapter must not fabricate home-artifact guidance"
        );
        assert!(confirmation.retry_action.contains("重新确认"));

        let recipe = match registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &root,
            &[ProviderName::Codex],
            ProviderTrustScenario::Recipe,
        ) {
            ProviderTrustPreparationResult::Waiting { waiting } => waiting,
            other => panic!("expected no-adapter waiting, got {other:?}"),
        };
        assert_eq!(recipe.reason_code, "unsupported_trust_provider");
        assert!(recipe.retry_action.contains("重试初始化"));
    }

    /// Task 2.2：Codex 读取失败（权限/格式类）指导含实际 config 路径与
    /// 「先备份再修复」。
    #[test]
    fn provider_trust_codex_read_failure_guidance_mentions_backup_and_path() {
        let fixture = TrustFixture::new();
        // config.toml 被目录占用：读取结构性失败（权限/格式类分型）。
        std::fs::create_dir_all(fixture.codex_config()).unwrap();
        let outcome = fixture.registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &[ProviderName::Codex],
            ProviderTrustScenario::Confirmation,
        );
        let ProviderTrustPreparationResult::Waiting { waiting } = outcome else {
            panic!("expected read-failure waiting");
        };
        assert_eq!(waiting.reason_code, "codex_config_unreadable");
        assert!(
            waiting
                .manual_guidance
                .contains(&fixture.codex_config().display().to_string())
        );
        assert!(
            waiting.manual_guidance.contains("备份"),
            "read/format guidance must advise backing up first: {}",
            waiting.manual_guidance
        );
    }

    // ------------------------------------------------------------------
    // Task 7.1：只读 trust source 三态零副作用真实快照锚（missing/conflict/
    // 登记后删除）。readonly verify 与 generation 同源指导前后，隔离 home
    // 与 durable trust 事实（ownership/waiting/audit）快照逐字节相等；
    // adapter 写次数与 audit 数可观察（计数 adapter 按调用计数断言，不只比
    // 最终字节）；拒收不补写、不伪造 capability——不调用会追加审计的
    // registry verify，也不经 ensure 复活被用户删除的条目。
    // ------------------------------------------------------------------

    struct ReadonlySourceFixture {
        _home: tempfile::TempDir,
        _aria: tempfile::TempDir,
        root: PathBuf,
        writes: Arc<AtomicUsize>,
    }

    impl ReadonlySourceFixture {
        fn new() -> Self {
            let home = tempfile::TempDir::new().unwrap();
            let aria = tempfile::TempDir::new().unwrap();
            let root = home.path().join("lc-root");
            std::fs::create_dir_all(&root).unwrap();
            Self {
                _home: home,
                _aria: aria,
                root,
                writes: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn home(&self) -> &Path {
            self._home.path()
        }

        fn paths(&self) -> ProductAppPaths {
            ProductAppPaths::new(self._aria.path().to_path_buf())
        }

        fn store(&self) -> ProviderTrustStore {
            ProviderTrustStore::for_lc(self.paths(), LC)
        }

        /// 只读 source：计数 adapter 包装真实 Codex/Kimi adapter（隔离
        /// home），write_trusted/remove_trusted 每次调用计数——「写次数=0」
        /// 按调用计数断言。
        fn source(&self) -> ReadonlyProviderTrustSource {
            let writes = Arc::clone(&self.writes);
            ReadonlyProviderTrustSource::new(
                self.paths(),
                Some(LC.to_string()),
                vec![
                    CountingTrustAdapter::wrap(
                        Arc::new(CodexTrustAdapter::for_home(self.home())),
                        Arc::clone(&writes),
                    ),
                    CountingTrustAdapter::wrap(
                        Arc::new(KimiTrustAdapter::for_home(self.home())),
                        writes,
                    ),
                ],
            )
        }

        /// 布置用可写 registry（真实 adapter，同一隔离 home 与 durable 布局）。
        fn registry(&self) -> HomeBackedProviderTrustRegistry {
            HomeBackedProviderTrustRegistry::new(
                self.paths(),
                vec![
                    Arc::new(CodexTrustAdapter::for_home(self.home())),
                    Arc::new(KimiTrustAdapter::for_home(self.home())),
                ],
            )
        }

        fn codex_config(&self) -> PathBuf {
            self.home().join(".codex").join("config.toml")
        }

        fn kimi_record(&self) -> PathBuf {
            KimiTrustAdapter::for_home(self.home()).record_path(&self.root)
        }

        /// Codex 用户冲突条目：目标 root 的 projects 表持非 trusted 值。
        fn seed_codex_conflict(&self) {
            let config = self.codex_config();
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::write(
                &config,
                format!(
                    "[projects.\"{}\"]\ntrust_level = \"untrusted\"\n",
                    self.root.display()
                ),
            )
            .unwrap();
        }

        /// Kimi 用户冲突条目：本根稳定键的记录文件绑到其它 root。
        fn seed_kimi_conflict(&self) {
            let record = self.kimi_record();
            std::fs::create_dir_all(record.parent().unwrap()).unwrap();
            std::fs::write(
                &record,
                serde_json::json!({
                    "root": "/definitely/another/root",
                    "trustedAt": 1_u64,
                })
                .to_string(),
            )
            .unwrap();
        }

        /// 经真实 registry 登记两 provider（home 条目+ownership/audit），
        /// 再删除 home 侧用户工件——模拟「登记后用户删除条目」，durable
        /// 事实仍在场。
        fn register_then_delete_home_entries(&self) {
            let registry = self.registry();
            for provider in [ProviderName::Codex, ProviderName::KimiCode] {
                registry
                    .ensure_trusted(PROJECT, OPERATION, LC, &provider, &self.root)
                    .unwrap_or_else(|error| panic!("seed ensure {provider:?}: {error:?}"));
            }
            std::fs::remove_file(self.codex_config()).unwrap();
            std::fs::remove_file(self.kimi_record()).unwrap();
        }
    }

    /// 递归 sha256 快照（路径+内容摘要，排序稳定）——home 与 durable
    /// trust 事实目录的前后逐字节对比锚。
    fn readonly_snapshot_tree(root: &Path) -> Vec<(String, String)> {
        fn sha256_hex(bytes: &[u8]) -> String {
            use sha2::{Digest, Sha256};
            format!("sha256:{:x}", Sha256::digest(bytes))
        }
        fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    let digest = std::fs::read(&path)
                        .map(|bytes| sha256_hex(&bytes))
                        .unwrap_or_else(|error| format!("read-error:{error}"));
                    out.push((path.display().to_string(), digest));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out.sort();
        out
    }

    /// 三态共用断言面：对每个 provider 执行只读 verify + generation 同源
    /// 指导，断言 home/durable 快照前后相等、adapter 写次数=0、audit 数
    /// 不变（基线由调用方给出），并回传 verification 供状态语义断言。
    fn readonly_source_asserts_frozen(
        fixture: &ReadonlySourceFixture,
        audit_baseline: usize,
    ) -> Vec<crate::product::logical_codebase::ProviderTrustVerification> {
        use super::ProviderTrustSource;
        let source = fixture.source();
        let mut verifications = Vec::new();
        for provider in [ProviderName::Codex, ProviderName::KimiCode] {
            let home_before = readonly_snapshot_tree(fixture.home());
            let durable_before = readonly_snapshot_tree(fixture._aria.path());
            let audits_before = fixture.store().list_audit(PROJECT).unwrap().len();
            assert_eq!(audits_before, audit_baseline);

            let verification = source
                .verify_trusted(PROJECT, LC, &provider, &fixture.root)
                .unwrap_or_else(|error| panic!("readonly verify {provider:?}: {error:?}"));
            verifications.push(verification);
            let waiting = source.read_only_generation_waiting(&provider, &fixture.root);

            assert_eq!(
                fixture.writes.load(Ordering::SeqCst),
                0,
                "只读 source 不得调用 adapter 写面（{provider:?}）"
            );
            assert_eq!(
                fixture.store().list_audit(PROJECT).unwrap().len(),
                audit_baseline,
                "只读 verify/guidance 不得追加 trust 审计（{provider:?}）"
            );
            assert!(
                fixture.store().list_waiting(PROJECT).unwrap().is_empty(),
                "只读路径不得落 waiting 事实（{provider:?}）"
            );
            assert_eq!(
                readonly_snapshot_tree(fixture.home()),
                home_before,
                "只读检查前后 home 快照必须相等（{provider:?}）"
            );
            assert_eq!(
                readonly_snapshot_tree(fixture._aria.path()),
                durable_before,
                "只读检查前后 durable trust 事实快照必须相等（{provider:?}）"
            );
            let _ = waiting;
        }
        verifications
    }

    /// 7.1 主用例①（missing）：无任何条目时 readonly verify 判
    /// trusted=false、detail=entry absent、无 ownership 可言（不伪造）；
    /// generation 同源指导在场（同 adapter/同 trust key 算法），全部
    /// 只读——零写、零审计、零 waiting。
    #[test]
    fn readonly_source_missing_state_verifies_and_guides_with_zero_side_effects() {
        use super::ProviderTrustSource;
        let fixture = ReadonlySourceFixture::new();
        let source = fixture.source();
        let verifications = readonly_source_asserts_frozen(&fixture, 0);
        for verification in &verifications {
            assert!(!verification.trusted, "missing 不得伪造 trusted");
            assert_eq!(verification.detail, "entry absent");
            assert!(
                verification.ownership.is_none(),
                "missing 态无 durable ownership，不得伪造归属"
            );
        }
        let waiting = source
            .read_only_generation_waiting(&ProviderName::Codex, &fixture.root)
            .expect("missing 态 generation 同源指导必须在场");
        assert_eq!(waiting.reason_code, super::TRUST_ENTRY_MISSING_CODE);
        assert_eq!(
            waiting.trust_key,
            CodexTrustAdapter::for_home(fixture.home()).trust_key(&fixture.root),
            "指导携带的 trust key 必须与 adapter 同源算法生成"
        );
        assert!(
            waiting
                .manual_guidance
                .contains(&fixture.codex_config().display().to_string()),
            "同源指导指向隔离 home 的真实 Codex 工件路径"
        );
        assert_eq!(fixture.writes.load(Ordering::SeqCst), 0);
    }

    /// 7.1 主用例②（conflict）：用户既有非 trusted 条目只读拒收——
    /// trusted=false（不伪造 capability）、detail=非 trusted 值、冲突
    /// 原因进 generation 指导；用户条目字节保持（不覆盖、不补写）。
    #[test]
    fn readonly_source_conflict_state_rejects_without_fabrication_or_overwrite() {
        use super::ProviderTrustSource;
        let fixture = ReadonlySourceFixture::new();
        fixture.seed_codex_conflict();
        fixture.seed_kimi_conflict();
        let conflict_config = std::fs::read_to_string(fixture.codex_config()).unwrap();

        let verifications = readonly_source_asserts_frozen(&fixture, 0);
        for verification in &verifications {
            assert!(!verification.trusted, "conflict 不得伪造 trusted");
            assert_eq!(verification.detail, "key holds a non-trusted value");
        }
        let source = fixture.source();
        let codex_waiting = source
            .read_only_generation_waiting(&ProviderName::Codex, &fixture.root)
            .expect("conflict 态指导在场（引导人工核验）");
        assert_eq!(codex_waiting.reason_code, "codex_trust_key_conflict");
        let kimi_waiting = source
            .read_only_generation_waiting(&ProviderName::KimiCode, &fixture.root)
            .expect("Kimi 冲突同样给出指导");
        assert_eq!(kimi_waiting.reason_code, "kimi_trust_key_conflict");
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            conflict_config,
            "用户冲突条目字节保持不变（只读不覆盖）"
        );
        assert_eq!(fixture.writes.load(Ordering::SeqCst), 0);
        assert!(fixture.store().list_audit(PROJECT).unwrap().is_empty());
    }

    /// 7.1 主用例③（登记后删除）：registry 曾登记（home 条目+ownership+
    /// audit 在场），用户删除 home 侧工件后，readonly verify 判
    /// trusted=false（不复活条目）、ownership 仍如实投影 LcManaged（durable
    /// 事实未变）、audit 数保持登记基线；generation 指导回落 missing。
    #[test]
    fn readonly_source_deleted_after_registration_keeps_facts_frozen() {
        use super::ProviderTrustSource;
        let fixture = ReadonlySourceFixture::new();
        fixture.register_then_delete_home_entries();
        // 布置期 ensure 审计基线：两 provider 各一条 ensure/registered。
        let audits = fixture.store().list_audit(PROJECT).unwrap();
        assert_eq!(audits.len(), 2, "布置期恰好两条登记审计");
        assert!(
            audits
                .iter()
                .all(|record| record.action == "ensure" && record.outcome == "registered"),
            "审计事件名以源码为准（ensure/registered），不存在 gate_registered"
        );

        let verifications = readonly_source_asserts_frozen(&fixture, 2);
        for verification in &verifications {
            assert!(
                !verification.trusted,
                "登记后删除不得按 durable 记录伪造 trusted"
            );
            assert_eq!(verification.detail, "entry absent");
            assert_eq!(
                verification.ownership,
                Some(ProviderTrustOwnership::LcManaged),
                "ownership 只读如实投影既有 durable 事实"
            );
        }
        assert!(
            !fixture.codex_config().exists() && !fixture.kimi_record().exists(),
            "只读检查后 home 条目不得被复活"
        );
        let source = fixture.source();
        let waiting = source
            .read_only_generation_waiting(&ProviderName::Codex, &fixture.root)
            .expect("删除后回落 missing 指导");
        assert_eq!(waiting.reason_code, super::TRUST_ENTRY_MISSING_CODE);
        assert_eq!(fixture.writes.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.store().list_audit(PROJECT).unwrap().len(), 2);
    }

    // ------------------------------------------------------------------
    // Task 3.1（B0a1）：只读 trust 冻结事实 `fingerprint_facts`——GET/spawn
    // 复验与 resume 冻结共用的纯读面：verification 与 `verify_trusted` 同一
    // 真实 adapter 只读来源，record_digest 只暴露 adapter 工件摘要（Codex
    // 整份 config、Kimi 对应 record 原文），verified_at 不参与摘要；
    // missing/conflict/登记后删除一律 typed 拒（稳定 trust code），全程
    // home/ownership/waiting/audit 前后逐字节相等、adapter 写计数=0。
    // ------------------------------------------------------------------

    /// 独立复算文件字节的 sha256 摘要（与 adapter `digest` 的输出形状一致，
    /// `sha256:<hex>`），证明 record_digest 就是工件原文摘要、不含
    /// verified_at 等观测时刻字段。
    fn anchor_t08_sha256_of_file(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(path).unwrap();
        format!("sha256:{:x}", Sha256::digest(&bytes))
    }

    /// 零副作用围栏：执行 `act` 前后断言 home 与 durable trust 事实
    /// （ownership/waiting/audit）快照逐字节相等、adapter 写计数=0。
    fn anchor_t08_assert_frozen_around<T>(
        fixture: &ReadonlySourceFixture,
        audits_before: usize,
        act: impl FnOnce() -> T,
    ) -> T {
        let home_before = readonly_snapshot_tree(fixture.home());
        let durable_before = readonly_snapshot_tree(fixture._aria.path());
        let outcome = act();
        assert_eq!(
            fixture.writes.load(Ordering::SeqCst),
            0,
            "adapter_write_calls 必须为 0（不 ensure/不 revoke/不复活条目）"
        );
        assert!(
            fixture.store().list_waiting(PROJECT).unwrap().is_empty(),
            "只读 facts 不得落 waiting 事实"
        );
        assert_eq!(
            fixture.store().list_audit(PROJECT).unwrap().len(),
            audits_before,
            "只读 facts 不得追加 trust 审计"
        );
        assert_eq!(
            readonly_snapshot_tree(fixture.home()),
            home_before,
            "只读 facts 前后 home 快照必须逐字节相等"
        );
        assert_eq!(
            readonly_snapshot_tree(fixture._aria.path()),
            durable_before,
            "只读 facts 前后 ownership/durable trust 快照必须逐字节相等"
        );
        outcome
    }

    #[test]
    fn anchor_t08_readonly_trust_fingerprint_facts_have_zero_side_effects() {
        use super::ProviderTrustSource;
        use crate::product::logical_codebase::provider_gateway::ProviderGatewayError;

        // ---- 正例：经真实 registry 登记后，facts 只读冻结真实工件事实 ----
        let fixture = ReadonlySourceFixture::new();
        {
            let registry = fixture.registry();
            for provider in [ProviderName::Codex, ProviderName::KimiCode] {
                registry
                    .ensure_trusted(PROJECT, OPERATION, LC, &provider, &fixture.root)
                    .unwrap_or_else(|error| panic!("seed ensure {provider:?}: {error:?}"));
            }
        }
        // 布置期审计基线：两 provider 各一条 ensure/registered。
        assert_eq!(fixture.store().list_audit(PROJECT).unwrap().len(), 2);
        let source = fixture.source();

        for provider in [ProviderName::Codex, ProviderName::KimiCode] {
            let facts = anchor_t08_assert_frozen_around(&fixture, 2, || {
                source
                    .fingerprint_facts(PROJECT, LC, &provider, &fixture.root)
                    .unwrap_or_else(|error| panic!("fingerprint facts {provider:?}: {error:?}"))
            });
            assert!(facts.verification.trusted);
            assert!(facts.record_digest.is_some());

            // verify/digest 同真实 adapter 来源：facts.verification 与同一
            // source 的 verify_trusted 逐字段一致（verified_at 是每次观测
            // 时刻，不参与相等断言，也不参与 digest）。
            let verification = anchor_t08_assert_frozen_around(&fixture, 2, || {
                source
                    .verify_trusted(PROJECT, LC, &provider, &fixture.root)
                    .unwrap_or_else(|error| panic!("verify {provider:?}: {error:?}"))
            });
            assert_eq!(facts.verification.provider, verification.provider);
            assert_eq!(
                facts.verification.canonical_root,
                verification.canonical_root
            );
            assert_eq!(facts.verification.trust_key, verification.trust_key);
            assert_eq!(facts.verification.trusted, verification.trusted);
            assert_eq!(facts.verification.ownership, verification.ownership);
            assert_eq!(facts.verification.detail, verification.detail);

            // Codex 整份 config / Kimi 对应 record 原文：record_digest 等
            // 于工件字节的独立摘要，不含 verified_at/detail 等观测字段。
            let artifact_path = match provider {
                ProviderName::Codex => fixture.codex_config(),
                ProviderName::KimiCode => fixture.kimi_record(),
                other => panic!("unexpected trust provider {other:?}"),
            };
            let expected_digest = anchor_t08_sha256_of_file(&artifact_path);
            assert_eq!(
                facts.record_digest.as_deref(),
                Some(expected_digest.as_str()),
                "{provider:?} record_digest 必须是工件原文摘要"
            );

            // 工件不变时摘要稳定：二次调用（verified_at 为新观测时刻）
            // record_digest 不变——观测时刻不入摘要。
            let again = anchor_t08_assert_frozen_around(&fixture, 2, || {
                source
                    .fingerprint_facts(PROJECT, LC, &provider, &fixture.root)
                    .unwrap()
            });
            assert_eq!(again.record_digest, facts.record_digest);
        }

        // ---- missing：typed 拒，不产 Ok(trusted=false) 假冻结 ----
        let fixture = ReadonlySourceFixture::new();
        let source = fixture.source();
        for provider in [ProviderName::Codex, ProviderName::KimiCode] {
            let rejection = anchor_t08_assert_frozen_around(&fixture, 0, || {
                source
                    .fingerprint_facts(PROJECT, LC, &provider, &fixture.root)
                    .expect_err("missing entry must be typed-rejected, not frozen")
            });
            assert!(
                matches!(rejection, ProviderGatewayError::ProviderUnavailable(_)),
                "missing 拒收必须是 typed 错误：{rejection:?}"
            );
            assert!(
                rejection.to_string().contains("trust_entry_missing"),
                "missing 拒收携带稳定 code：{rejection}"
            );
        }

        // ---- conflict：用户非 trusted 条目 typed 拒，用户字节不动 ----
        let fixture = ReadonlySourceFixture::new();
        fixture.seed_codex_conflict();
        fixture.seed_kimi_conflict();
        let conflict_config = std::fs::read_to_string(fixture.codex_config()).unwrap();
        let source = fixture.source();
        for (provider, stable_code) in [
            (ProviderName::Codex, "codex_trust_key_conflict"),
            (ProviderName::KimiCode, "kimi_trust_key_conflict"),
        ] {
            let rejection = anchor_t08_assert_frozen_around(&fixture, 0, || {
                source
                    .fingerprint_facts(PROJECT, LC, &provider, &fixture.root)
                    .expect_err("conflict entry must be typed-rejected, not overwritten")
            });
            assert!(
                matches!(rejection, ProviderGatewayError::ProviderUnavailable(_)),
                "conflict 拒收必须是 typed 错误：{rejection:?}"
            );
            assert!(
                rejection.to_string().contains(stable_code),
                "conflict 拒收携带 adapter 稳定 code：{rejection}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(fixture.codex_config()).unwrap(),
            conflict_config,
            "用户冲突条目字节保持不变（只读不覆盖）"
        );

        // ---- 登记后删除：durable 事实仍在，typed 拒回落 missing、不复活 ----
        let fixture = ReadonlySourceFixture::new();
        fixture.register_then_delete_home_entries();
        let source = fixture.source();
        for provider in [ProviderName::Codex, ProviderName::KimiCode] {
            let rejection = anchor_t08_assert_frozen_around(&fixture, 2, || {
                source
                    .fingerprint_facts(PROJECT, LC, &provider, &fixture.root)
                    .expect_err("deleted entry must be typed-rejected, not resurrected")
            });
            assert!(
                matches!(rejection, ProviderGatewayError::ProviderUnavailable(_)),
                "删除后拒收必须是 typed 错误：{rejection:?}"
            );
            assert!(
                rejection.to_string().contains("trust_entry_missing"),
                "删除后回落 trust_entry_missing：{rejection}"
            );
        }
        assert!(
            !fixture.codex_config().exists() && !fixture.kimi_record().exists(),
            "只读 facts 后 home 条目不得被复活"
        );

        // ---- trait 默认：仅未接入/测试 source 停留在 typed unsupported ----
        struct UnwiredFakeSource;
        impl ProviderTrustSource for UnwiredFakeSource {
            fn verify_trusted(
                &self,
                _project_id: &str,
                _lc_id: &str,
                _provider: &ProviderName,
                _canonical_root: &Path,
            ) -> Result<
                crate::product::logical_codebase::ProviderTrustVerification,
                ProviderGatewayError,
            > {
                Err(ProviderGatewayError::ProviderUnavailable(
                    "unwired fake source".to_string(),
                ))
            }
        }
        let default_rejection = UnwiredFakeSource
            .fingerprint_facts(PROJECT, LC, &ProviderName::Codex, &fixture.root)
            .expect_err("default trait impl must stay typed-unsupported");
        assert!(
            matches!(
                default_rejection,
                ProviderGatewayError::ProviderUnavailable(_)
            ),
            "默认实现必须是 typed unsupported：{default_rejection:?}"
        );
        assert!(
            default_rejection
                .to_string()
                .contains("unsupported_trust_fingerprint_facts"),
            "默认 unsupported 携带稳定标记：{default_rejection}"
        );
    }
    // ------------------------------------------------------------------
    // Task 3.1/3.2：sidecar 锁并发矩阵与取消安全（受控窗口：lock-attempt
    // hook 先例 + 测试线程持锁 barrier；禁 sleep 概率绿灯——所有窗口都
    // 由 hook 通知/信号 channel 精确控制，超时即揭红）。
    // ------------------------------------------------------------------

    /// 测试线程持有的 home sidecar 锁窗口：竞争者只能在锁上排队。
    struct HeldHomeLock {
        _guard: ExclusiveFileLock,
        _hook: LockAttemptHookGuard,
        attempts: mpsc::Receiver<()>,
    }

    fn hold_home_lock(target: &Path) -> HeldHomeLock {
        let _guard = ExclusiveFileLock::acquire(target).expect("test holds the home sidecar lock");
        let (_hook, attempts) = register_lock_attempt_hook(target);
        HeldHomeLock {
            _guard,
            _hook,
            attempts,
        }
    }

    /// 竞争者到达 sidecar 锁的超时窗：未持锁实现永远不会通知——超时即红。
    const CONTENDER_TIMEOUT: Duration = Duration::from_secs(5);

    fn contender_arrived(attempts: &mpsc::Receiver<()>) -> bool {
        attempts.recv_timeout(CONTENDER_TIMEOUT).is_ok()
    }

    /// 计数 adapter：委托真实 adapter 并统计 write_trusted/remove_trusted 的
    /// 调用次数——「Trusted 写次数=0」按调用计数断言，不只比最终字节。
    struct CountingTrustAdapter {
        inner: Arc<dyn ProviderTrustHomeAdapter>,
        writes: Arc<AtomicUsize>,
    }

    impl CountingTrustAdapter {
        fn wrap(
            inner: Arc<dyn ProviderTrustHomeAdapter>,
            writes: Arc<AtomicUsize>,
        ) -> Arc<dyn ProviderTrustHomeAdapter> {
            Arc::new(Self { inner, writes })
        }
    }

    impl ProviderTrustHomeAdapter for CountingTrustAdapter {
        fn provider(&self) -> ProviderName {
            self.inner.provider()
        }

        fn trust_key(&self, canonical_root: &Path) -> String {
            self.inner.trust_key(canonical_root)
        }

        fn read_state(
            &self,
            canonical_root: &Path,
        ) -> Result<ProviderTrustEntryState, ProviderTrustError> {
            self.inner.read_state(canonical_root)
        }

        fn digest(&self, canonical_root: &Path) -> Result<Option<String>, ProviderTrustError> {
            self.inner.digest(canonical_root)
        }

        fn write_trusted(
            &self,
            canonical_root: &Path,
            expected_digest: Option<&str>,
        ) -> Result<ProviderTrustWriteOutcome, ProviderTrustError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.inner.write_trusted(canonical_root, expected_digest)
        }

        fn remove_trusted(
            &self,
            canonical_root: &Path,
            expected_digest: Option<&str>,
        ) -> Result<(), ProviderTrustError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.inner.remove_trusted(canonical_root, expected_digest)
        }

        fn conflict_code(&self) -> &'static str {
            self.inner.conflict_code()
        }

        fn manual_hint(&self, canonical_root: &Path) -> ProviderTrustArtifactHint {
            self.inner.manual_hint(canonical_root)
        }

        fn artifact_path(&self, canonical_root: &Path) -> PathBuf {
            self.inner.artifact_path(canonical_root)
        }
    }

    /// 门闩 adapter：digest 先发信号再委托——信号到达时，持锁实现必已进入
    /// home 临界区（digest 只在取锁之后执行），提供无竞态 barrier。
    struct SectionSignalAdapter {
        inner: Arc<dyn ProviderTrustHomeAdapter>,
        entered: mpsc::Sender<()>,
    }

    impl ProviderTrustHomeAdapter for SectionSignalAdapter {
        fn provider(&self) -> ProviderName {
            self.inner.provider()
        }

        fn trust_key(&self, canonical_root: &Path) -> String {
            self.inner.trust_key(canonical_root)
        }

        fn read_state(
            &self,
            canonical_root: &Path,
        ) -> Result<ProviderTrustEntryState, ProviderTrustError> {
            self.inner.read_state(canonical_root)
        }

        fn digest(&self, canonical_root: &Path) -> Result<Option<String>, ProviderTrustError> {
            let _ = self.entered.send(());
            self.inner.digest(canonical_root)
        }

        fn write_trusted(
            &self,
            canonical_root: &Path,
            expected_digest: Option<&str>,
        ) -> Result<ProviderTrustWriteOutcome, ProviderTrustError> {
            self.inner.write_trusted(canonical_root, expected_digest)
        }

        fn remove_trusted(
            &self,
            canonical_root: &Path,
            expected_digest: Option<&str>,
        ) -> Result<(), ProviderTrustError> {
            self.inner.remove_trusted(canonical_root, expected_digest)
        }

        fn conflict_code(&self) -> &'static str {
            self.inner.conflict_code()
        }

        fn manual_hint(&self, canonical_root: &Path) -> ProviderTrustArtifactHint {
            self.inner.manual_hint(canonical_root)
        }

        fn artifact_path(&self, canonical_root: &Path) -> PathBuf {
            self.inner.artifact_path(canonical_root)
        }
    }

    /// 3.1/3.2 并发测试夹具：独立 home/aria，按测试装配计数/门闩 registry。
    struct ConcurrencyFixture {
        _home: tempfile::TempDir,
        aria: tempfile::TempDir,
        root: PathBuf,
    }

    impl ConcurrencyFixture {
        fn new() -> Self {
            let home = tempfile::TempDir::new().unwrap();
            let aria = tempfile::TempDir::new().unwrap();
            let root = home.path().join("lc-root");
            std::fs::create_dir_all(&root).unwrap();
            Self {
                _home: home,
                aria,
                root,
            }
        }

        fn home(&self) -> &Path {
            self._home.path()
        }

        fn root_b(&self) -> PathBuf {
            let root = self.home().join("lc-root-b");
            std::fs::create_dir_all(&root).unwrap();
            root
        }

        fn codex_config(&self) -> PathBuf {
            self.home().join(".codex").join("config.toml")
        }

        fn paths(&self) -> ProductAppPaths {
            ProductAppPaths::new(self.aria.path().to_path_buf())
        }

        fn store(&self, lc: &str) -> ProviderTrustStore {
            ProviderTrustStore::for_lc(self.paths(), lc)
        }

        fn counting_registry(
            &self,
            adapter: Arc<dyn ProviderTrustHomeAdapter>,
        ) -> (HomeBackedProviderTrustRegistry, Arc<AtomicUsize>) {
            let writes = Arc::new(AtomicUsize::new(0));
            let registry = HomeBackedProviderTrustRegistry::new(
                self.paths(),
                vec![CountingTrustAdapter::wrap(adapter, Arc::clone(&writes))],
            );
            (registry, writes)
        }

        fn counting_codex_registry(&self) -> (HomeBackedProviderTrustRegistry, Arc<AtomicUsize>) {
            self.counting_registry(Arc::new(CodexTrustAdapter::for_home(self.home())))
        }
    }

    /// Task 3.1：同 root 同 LC 的并发 ensure 不得把 owned=true 降级——
    /// 竞争者必须等在 sidecar 锁上，看到的只能是「W1 已完成登记」的完整
    /// 状态（Replay），而不是 Trusted-but-ownership-missing 中间态。
    #[test]
    fn provider_trust_concurrent_same_lc_ensure_does_not_downgrade_ownership() {
        let fixture = ConcurrencyFixture::new();
        let (registry, writes) = fixture.counting_codex_registry();
        // W1 中途态：home 条目已写、ownership 尚未落盘。
        let codex = CodexTrustAdapter::for_home(fixture.home());
        let created = codex.write_trusted(&fixture.root, None).unwrap();
        assert!(matches!(created, ProviderTrustWriteOutcome::Created { .. }));
        let trust_key = codex.trust_key(&fixture.root);

        let held = hold_home_lock(&fixture.codex_config());
        let (result_tx, result_rx) = mpsc::channel();
        let root = fixture.root.clone();
        std::thread::spawn(move || {
            let _ = result_tx.send(registry.ensure_trusted(
                PROJECT,
                OPERATION,
                LC,
                &ProviderName::Codex,
                &root,
            ));
        });

        let arrived = contender_arrived(&held.attempts);
        if arrived {
            // 持锁实现：竞争者停在锁上；W1 在临界区内完成 ownership 落盘。
            let now = trust_now();
            fixture
                .store(LC)
                .record_ownership(
                    PROJECT,
                    ProviderTrustOwnershipRecord {
                        lc_id: LC.to_string(),
                        provider: "codex".to_string(),
                        canonical_root: fixture.root.clone(),
                        trust_key: trust_key.clone(),
                        owned: true,
                        before_digest: None,
                        after_digest: codex.digest(&fixture.root).unwrap(),
                        created_at: now.clone(),
                        updated_at: now,
                    },
                )
                .unwrap();
        }
        drop(held);

        let registration = result_rx
            .recv_timeout(CONTENDER_TIMEOUT)
            .expect("contender completes")
            .expect("same-LC ensure succeeds");
        assert_eq!(
            registration.ownership,
            ProviderTrustOwnership::LcManaged,
            "same-LC replay must stay LC-managed (arrival={arrived})"
        );
        assert_eq!(registration.action, ProviderTrustAction::Replay);
        assert!(
            fixture
                .store(LC)
                .load_ownership(PROJECT, "codex", &trust_key)
                .unwrap()
                .is_some_and(|record| record.owned),
            "same-LC replay must not downgrade ownership to user-owned"
        );
        assert_eq!(
            writes.load(Ordering::SeqCst),
            0,
            "Trusted replay must not call adapter writes (call count, not byte equality)"
        );
        // Ready 返回前 home 锁已释放：可立即重新获取。
        assert!(ExclusiveFileLock::acquire(&fixture.codex_config()).is_ok());
    }

    /// Task 3.1：跨 LC 并发复用——另一 LC 的竞争者等在锁上直到 LC1 完成
    /// 登记，随后以 ReuseUserEntry/owned=false 复用且零 adapter 写调用。
    #[test]
    fn provider_trust_concurrent_cross_lc_reuse_stays_user_owned_without_adapter_writes() {
        let fixture = ConcurrencyFixture::new();
        const LC2: &str = "logical_codebase_0002";
        let (registry, writes) = fixture.counting_codex_registry();

        let held = hold_home_lock(&fixture.codex_config());
        let (result_tx, result_rx) = mpsc::channel();
        let root = fixture.root.clone();
        std::thread::spawn(move || {
            let _ = result_tx.send(registry.ensure_trusted(
                PROJECT,
                OPERATION,
                LC2,
                &ProviderName::Codex,
                &root,
            ));
        });

        let arrived = contender_arrived(&held.attempts);
        if arrived {
            // 持锁实现：竞争者停在锁上；LC1 在临界区内完成登记。
            let codex = CodexTrustAdapter::for_home(fixture.home());
            let created = codex.write_trusted(&fixture.root, None).unwrap();
            assert!(matches!(created, ProviderTrustWriteOutcome::Created { .. }));
            let trust_key = codex.trust_key(&fixture.root);
            let now = trust_now();
            fixture
                .store(LC)
                .record_ownership(
                    PROJECT,
                    ProviderTrustOwnershipRecord {
                        lc_id: LC.to_string(),
                        provider: "codex".to_string(),
                        canonical_root: fixture.root.clone(),
                        trust_key,
                        owned: true,
                        before_digest: None,
                        after_digest: codex.digest(&fixture.root).unwrap(),
                        created_at: now.clone(),
                        updated_at: now,
                    },
                )
                .unwrap();
        }
        drop(held);

        let registration = result_rx
            .recv_timeout(CONTENDER_TIMEOUT)
            .expect("contender completes")
            .expect("cross-LC ensure succeeds");
        assert_eq!(
            registration.action,
            ProviderTrustAction::ReuseUserEntry,
            "cross-LC must reuse the LC1 entry without registering (arrival={arrived})"
        );
        assert_eq!(registration.ownership, ProviderTrustOwnership::UserOwned);
        assert_eq!(
            writes.load(Ordering::SeqCst),
            0,
            "cross-LC reuse must not call adapter writes (call count, not byte equality)"
        );
        let codex = CodexTrustAdapter::for_home(fixture.home());
        let trust_key = codex.trust_key(&fixture.root);
        assert!(
            fixture
                .store(LC)
                .load_ownership(PROJECT, "codex", &trust_key)
                .unwrap()
                .is_some_and(|record| record.owned),
            "LC1 ownership must survive the cross-LC reuse"
        );
        assert!(
            !fixture
                .store(LC2)
                .load_ownership(PROJECT, "codex", &trust_key)
                .unwrap()
                .is_some_and(|record| record.owned),
            "reusing LC must record owned=false"
        );
    }

    /// Task 3.1：不同 root 共享 Codex config——两个 ensure 必须在整 config
    /// 的 sidecar 锁上串行；持锁窗口内零写入，释放后两个 root 的条目都在
    /// （无丢更新、无伪 CAS 失败）。
    #[test]
    fn provider_trust_concurrent_codex_roots_share_config_lock_without_lost_updates() {
        let fixture = ConcurrencyFixture::new();
        let root_b = fixture.root_b();
        let (registry, writes) = fixture.counting_codex_registry();
        let config_before = std::fs::read_to_string(fixture.codex_config()).ok();

        let held = hold_home_lock(&fixture.codex_config());
        let (done_tx, done_rx) = mpsc::channel();
        let registry = Arc::new(registry);
        for root in [fixture.root.clone(), root_b.clone()] {
            let registry = Arc::clone(&registry);
            let done_tx = done_tx.clone();
            std::thread::spawn(move || {
                let _ = done_tx.send(registry.ensure_trusted(
                    PROJECT,
                    OPERATION,
                    LC,
                    &ProviderName::Codex,
                    &root,
                ));
            });
        }
        drop(done_tx);

        let mut arrivals = 0;
        for _ in 0..2 {
            if contender_arrived(&held.attempts) {
                arrivals += 1;
            }
        }
        if arrivals == 2 {
            // 持锁实现：他人临界区内零写入。
            assert_eq!(
                std::fs::read_to_string(fixture.codex_config()).ok(),
                config_before,
                "no writes inside another holder's critical section"
            );
            drop(held);
            let mut outcomes = Vec::new();
            for _ in 0..2 {
                outcomes.push(
                    done_rx
                        .recv_timeout(CONTENDER_TIMEOUT)
                        .expect("serialized contender finishes"),
                );
            }
            assert!(
                outcomes.iter().all(|outcome| outcome.is_ok()),
                "serialized ensures must both succeed without spurious CAS failures: {outcomes:?}"
            );
        } else {
            drop(held);
            for _ in 0..2 {
                let _ = done_rx.recv_timeout(CONTENDER_TIMEOUT);
            }
            assert_eq!(
                std::fs::read_to_string(fixture.codex_config()).ok(),
                config_before,
                "ensures must not touch the shared config while another holder owns the sidecar lock"
            );
        }

        // 无丢更新：两个 root 的条目都在。
        let config = std::fs::read_to_string(fixture.codex_config()).unwrap();
        let codex = CodexTrustAdapter::for_home(fixture.home());
        assert!(config.contains(&codex.trust_key(&fixture.root)));
        assert!(config.contains(&codex.trust_key(&root_b)));
        assert_eq!(writes.load(Ordering::SeqCst), 2);
    }

    /// Task 3.1：ensure×revoke 交错——revoke 中途（条目已摘、ownership 待删）
    /// 到达的 ensure 必须等在锁上；最终状态自洽：条目在 ⇒ 本 LC owned ⇒
    /// 可再撤销，绝不留下不可撤销的孤儿条目。
    #[test]
    fn provider_trust_concurrent_ensure_and_revoke_keep_entry_ownership_coherent() {
        let fixture = ConcurrencyFixture::new();
        let (registry, _writes) = fixture.counting_codex_registry();
        // 前置：本 LC 已完成登记（条目 + ownership owned=true）。
        registry
            .ensure_trusted(PROJECT, OPERATION, LC, &ProviderName::Codex, &fixture.root)
            .unwrap();
        let codex = CodexTrustAdapter::for_home(fixture.home());
        let trust_key = codex.trust_key(&fixture.root);

        let held = hold_home_lock(&fixture.codex_config());
        // revoke 中途态：remove_trusted 已执行（条目摘除），delete_ownership
        // 未执行——由测试在受控窗口内收尾。
        let digest = codex.digest(&fixture.root).unwrap();
        codex
            .remove_trusted(&fixture.root, digest.as_deref())
            .unwrap();
        assert!(
            !std::fs::read_to_string(fixture.codex_config())
                .unwrap()
                .contains(&trust_key)
        );

        let (result_tx, result_rx) = mpsc::channel();
        let registry = Arc::new(registry);
        let root = fixture.root.clone();
        std::thread::spawn(move || {
            let _ = result_tx.send(registry.ensure_trusted(
                PROJECT,
                OPERATION,
                LC,
                &ProviderName::Codex,
                &root,
            ));
        });

        let arrived = contender_arrived(&held.attempts);
        let early = if !arrived {
            Some(
                result_rx
                    .recv_timeout(CONTENDER_TIMEOUT)
                    .expect("unlocked contender completes"),
            )
        } else {
            None
        };
        // revoke 收尾：删除 ownership（未持锁实现里竞争者早已完成；持锁
        // 实现里竞争者还停在锁上）。
        fixture
            .store(LC)
            .delete_ownership(PROJECT, "codex", &trust_key)
            .unwrap();
        drop(held);

        let registration = match early {
            Some(result) => result,
            None => result_rx
                .recv_timeout(CONTENDER_TIMEOUT)
                .expect("contender completes"),
        }
        .expect("ensure after revoke succeeds");
        assert!(matches!(registration.action, ProviderTrustAction::Register));
        // 自洽：条目在 ⇒ ownership 在且 owned（可撤销，不是孤儿）。
        assert!(
            std::fs::read_to_string(fixture.codex_config())
                .unwrap()
                .contains(&trust_key)
        );
        assert!(
            fixture
                .store(LC)
                .load_ownership(PROJECT, "codex", &trust_key)
                .unwrap()
                .is_some_and(|record| record.owned),
            "entry present must stay LC-owned and revocable after ensure×revoke interleave"
        );
    }

    /// Task 3.1：Kimi 单记录锁粒度——R1 的记录锁被持有时，R2 的登记不被
    /// 过度串行（不得把整个 workspace-trust 目录当锁面）。
    #[test]
    fn provider_trust_kimi_per_record_locks_allow_different_roots() {
        let fixture = ConcurrencyFixture::new();
        let root_b = fixture.root_b();
        let kimi = KimiTrustAdapter::for_home(fixture.home());
        let (registry, writes) =
            fixture.counting_registry(Arc::new(KimiTrustAdapter::for_home(fixture.home())));

        let record_a = kimi.record_path(&fixture.root);
        let record_b = kimi.record_path(&root_b);
        let held = hold_home_lock(&record_a);
        let (result_tx, result_rx) = mpsc::channel();
        let registry = Arc::new(registry);
        std::thread::spawn(move || {
            let _ = result_tx.send(registry.ensure_trusted(
                PROJECT,
                OPERATION,
                LC,
                &ProviderName::KimiCode,
                &root_b,
            ));
        });

        let registration = result_rx
            .recv_timeout(CONTENDER_TIMEOUT)
            .expect("per-record lock must not serialize different roots")
            .expect("R2 ensure succeeds while R1's record lock is held");
        assert!(matches!(registration.action, ProviderTrustAction::Register));
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        // R2 的记录落盘；R1 的记录仍未被触碰。
        assert!(record_b.is_file());
        assert!(!record_a.exists());
        drop(held);
    }

    /// Task 3.1：sidecar 锁失败（锁文件不可创建）fail-closed——可见
    /// Waiting（trust_home_lock_failed），不无序写当前 waiting fact，
    /// 历史审计尽力留痕，home 工件字节不动。
    #[test]
    fn provider_trust_home_lock_failure_fails_closed_without_waiting_fact() {
        let fixture = ConcurrencyFixture::new();
        let codex_dir = fixture.codex_config().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            fixture.codex_config(),
            "[projects.\"other\"]\ntrust_level = \"trusted\"\n",
        )
        .unwrap();
        let original = std::fs::read_to_string(fixture.codex_config()).unwrap();
        let original_permissions = std::fs::metadata(&codex_dir).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        std::os::unix::fs::PermissionsExt::set_mode(&mut readonly, 0o500);
        std::fs::set_permissions(&codex_dir, readonly).unwrap();

        let (registry, writes) = fixture.counting_codex_registry();
        let outcome = registry.ensure_ready(
            PROJECT,
            OPERATION,
            LC,
            &fixture.root,
            &[ProviderName::Codex],
            ProviderTrustScenario::Confirmation,
        );
        std::fs::set_permissions(&codex_dir, original_permissions).unwrap();

        match outcome {
            ProviderTrustPreparationResult::Waiting { waiting } => {
                assert_eq!(waiting.reason_code, "trust_home_lock_failed");
                // 未获锁：不无序改写当前 waiting fact。
                assert!(fixture.store(LC).list_waiting(PROJECT).unwrap().is_empty());
                // 历史审计尽力写。
                assert!(
                    fixture
                        .store(LC)
                        .list_audit(PROJECT)
                        .unwrap()
                        .iter()
                        .any(|audit| audit.action == "gate_waiting"
                            && audit.outcome == "trust_home_lock_failed")
                );
                // home 工件字节不动、无 adapter 写调用。
                assert_eq!(
                    std::fs::read_to_string(fixture.codex_config()).unwrap(),
                    original
                );
                assert_eq!(writes.load(Ordering::SeqCst), 0);
            }
            ProviderTrustPreparationResult::Ready { .. } => {
                assert_eq!(
                    unsafe { libc::geteuid() },
                    0,
                    "lock-failure injection relies on chmod biting (requires non-root)"
                );
            }
        }
    }

    /// Task 3.2：失败的 record_waiting/gate_waiting 与成功 clear_waiting 在
    /// 同一已持 home 临界区内定序——A（旧失败）停在临界区内（fact 锁被
    /// 测试持有）时，较新的 B 只能等在 home 锁后面；A 落盘 waiting 后 B
    /// 才能成功并清除，绝不残留旧当前 waiting（历史失败审计保留）。
    #[test]
    fn provider_trust_failure_waiting_fact_ordered_inside_home_critical_section() {
        let fixture = ConcurrencyFixture::new();
        // 注入：config.toml 为目录 → A 的 digest 确定性失败。
        std::fs::create_dir_all(fixture.codex_config()).unwrap();

        let (entered_tx, entered_rx) = mpsc::channel();
        let registry_a = HomeBackedProviderTrustRegistry::new(
            fixture.paths(),
            vec![Arc::new(SectionSignalAdapter {
                inner: Arc::new(CodexTrustAdapter::for_home(fixture.home())),
                entered: entered_tx,
            })],
        );
        let registry_b = HomeBackedProviderTrustRegistry::new(
            fixture.paths(),
            vec![Arc::new(CodexTrustAdapter::for_home(fixture.home()))],
        );
        let fact_lock = crate::product::logical_codebase::lc_scope_root(
            &fixture.paths(),
            PROJECT,
            &Some(LC.to_string()),
        )
        .unwrap()
        .join(".provider-trust.lock");
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        let root_a = fixture.root.clone();
        let root_b = fixture.root.clone();

        let ordered = with_exact_exclusive_lock(&fact_lock, || {
            // 预创建 sidecar 锁文件（未持锁实现不会创建它），供稍后注册 hook。
            drop(ExclusiveFileLock::acquire(&fixture.codex_config()).unwrap());
            std::thread::spawn(move || {
                let _ = a_tx.send(registry_a.ensure_ready(
                    PROJECT,
                    OPERATION,
                    LC,
                    &root_a,
                    &[ProviderName::Codex],
                    ProviderTrustScenario::Recipe,
                ));
            });
            // A 已进入其临界区（digest 只在取锁后执行）并将停在 fact 锁上。
            entered_rx
                .recv_timeout(CONTENDER_TIMEOUT)
                .expect("failure pass must enter its home critical section");
            // 解除注入：较新的 B 此后可以成功。
            std::fs::remove_dir_all(fixture.codex_config()).unwrap();
            // hook 注册（锁文件已存在）；排掉 A 自己的取锁尝试事件。
            let (_hook, attempts) = register_lock_attempt_hook(&fixture.codex_config());
            let _ = attempts.try_recv();
            std::thread::spawn(move || {
                let _ = b_tx.send(registry_b.ensure_ready(
                    PROJECT,
                    OPERATION,
                    LC,
                    &root_b,
                    &[ProviderName::Codex],
                    ProviderTrustScenario::Recipe,
                ));
            });
            let b_waited = attempts.recv_timeout(CONTENDER_TIMEOUT).is_ok();
            if b_waited {
                Ok(())
            } else {
                Err(ProductStoreError::Io(
                    "newer ensure did not wait behind the in-flight failing section on the home lock"
                        .to_string(),
                ))
            }
        });
        ordered.expect("newer ensure must queue behind the in-flight failing section");

        let a_outcome = a_rx
            .recv_timeout(CONTENDER_TIMEOUT)
            .expect("failure pass completes");
        let b_outcome = b_rx
            .recv_timeout(CONTENDER_TIMEOUT)
            .expect("success pass completes");
        match &a_outcome {
            ProviderTrustPreparationResult::Waiting { waiting } => {
                assert_eq!(waiting.reason_code, "codex_config_unreadable");
            }
            other => panic!("expected the old failure to surface waiting, got {other:?}"),
        }
        assert!(matches!(
            b_outcome,
            ProviderTrustPreparationResult::Ready { .. }
        ));
        // 较新 Ready 不残留旧当前 waiting。
        assert!(fixture.store(LC).list_waiting(PROJECT).unwrap().is_empty());
        // 历史失败审计保留 + B 的成功审计在案。
        let audits = fixture.store(LC).list_audit(PROJECT).unwrap();
        assert!(
            audits.iter().any(|audit| audit.action == "gate_waiting"
                && audit.outcome == "codex_config_unreadable")
        );
        assert!(
            audits
                .iter()
                .any(|audit| audit.action == "ensure" && audit.outcome == "registered")
        );
    }

    /// 异步取消测试共享装配：独立 home/aria + 只读 Codex registry。
    struct AsyncFixture {
        _home: tempfile::TempDir,
        _aria: tempfile::TempDir,
        root: PathBuf,
        codex_config: PathBuf,
        paths: ProductAppPaths,
    }

    impl AsyncFixture {
        fn new() -> Self {
            let home = tempfile::TempDir::new().unwrap();
            let aria = tempfile::TempDir::new().unwrap();
            let root = home.path().join("lc-root");
            std::fs::create_dir_all(&root).unwrap();
            Self {
                codex_config: home.path().join(".codex").join("config.toml"),
                paths: ProductAppPaths::new(aria.path().to_path_buf()),
                _home: home,
                _aria: aria,
                root,
            }
        }

        fn registry(&self) -> Arc<dyn ProviderTrustPrecondition> {
            Arc::new(HomeBackedProviderTrustRegistry::new(
                self.paths.clone(),
                vec![Arc::new(CodexTrustAdapter::for_home(self._home.path()))],
            ))
        }

        fn store(&self) -> ProviderTrustStore {
            ProviderTrustStore::for_lc(self.paths.clone(), LC)
        }
    }

    /// Task 3.2：待锁获取可取消——竞争锁上的 ensure_ready_cancellable 被
    /// caller 取消后：不留 flock waiter、持锁者释放后业务不复活、不占满
    /// Tokio worker，home/store 零副作用。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_trust_cancellable_lock_wait_never_resurrects_registration() {
        let fixture = AsyncFixture::new();
        let registry = fixture.registry();
        let guard = ExclusiveFileLock::acquire(&fixture.codex_config).unwrap();
        let (_hook, attempts) = register_lock_attempt_hook(&fixture.codex_config);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            release_rx.recv_timeout(Duration::from_secs(10)).ok();
            drop(guard);
        });

        // 竞争锁上的等待必须可取消（caller 侧 timeout 即丢弃 future）。
        let cancelled = tokio::time::timeout(
            Duration::from_millis(300),
            registry.ensure_ready_cancellable(
                PROJECT.to_string(),
                OPERATION.to_string(),
                LC.to_string(),
                fixture.root.clone(),
                vec![ProviderName::Codex],
                ProviderTrustScenario::Confirmation,
            ),
        )
        .await;
        assert!(
            cancelled.is_err(),
            "contended home-lock wait must be cancellable"
        );
        // 取消前确实在按退避尝试取锁（不是空转）。
        assert!(
            attempts.try_recv().is_ok(),
            "cancellable wait must keep attempting the sidecar lock"
        );

        // 释放持锁者：取消后的业务不得复活（负向断言，有界宽限窗）。
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !fixture.codex_config.exists(),
            "cancelled lock wait must not resurrect the registration"
        );
        assert!(
            fixture
                .store()
                .load_ownership(
                    PROJECT,
                    "codex",
                    &CodexTrustAdapter::for_home(fixture._home.path()).trust_key(&fixture.root)
                )
                .unwrap()
                .is_none()
        );
        assert!(fixture.store().list_waiting(PROJECT).unwrap().is_empty());
        assert!(fixture.store().list_audit(PROJECT).unwrap().is_empty());
        // runtime 未被占满：普通任务照常推进。
        assert_eq!(tokio::task::spawn(async { 42 }).await.unwrap(), 42);
        // 锁未被遗留 waiter 占用：可立即重新获取。
        assert!(ExclusiveFileLock::acquire(&fixture.codex_config).is_ok());
    }

    /// Task 3.2：提交边界收尾不误删——临界区开始（home 锁已获取）后取消
    /// caller：临界区 detached 安全收尾（条目 + ownership + 审计一致），
    /// 他人 waiting fact 不被误删，收尾后 home 锁已释放。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_trust_cancelled_commit_boundary_finalizes_without_deleting_others() {
        let fixture = AsyncFixture::new();
        let store = fixture.store();
        let trust_key = CodexTrustAdapter::for_home(fixture._home.path()).trust_key(&fixture.root);
        // 他人（Kimi）的当前 waiting fact：不得被 Codex 收尾误删。
        store
            .record_waiting(
                PROJECT,
                ProviderTrustWaitingRecord {
                    lc_id: LC.to_string(),
                    provider: "kimi_code".to_string(),
                    canonical_root: fixture.root.clone(),
                    trust_key: "wd_other_deadbeef".to_string(),
                    operation_id: None,
                    reason_code: "kimi_trust_write_failed".to_string(),
                    message: "injected".to_string(),
                    retry_action: "fix".to_string(),
                    recorded_at: "2026-10-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        // fact 锁由外部线程持有：临界区将停在 record_ownership（提交已开始）。
        let fact_lock = crate::product::logical_codebase::lc_scope_root(
            &fixture.paths,
            PROJECT,
            &Some(LC.to_string()),
        )
        .unwrap()
        .join(".provider-trust.lock");
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            with_exact_exclusive_lock(&fact_lock, || {
                release_rx.recv_timeout(Duration::from_secs(10)).ok();
                Ok::<(), ProductStoreError>(())
            })
        });
        // 预创建 sidecar 锁文件并注册 hook（观察临界区开始）。
        drop(ExclusiveFileLock::acquire(&fixture.codex_config).unwrap());
        let (_hook, attempts) = register_lock_attempt_hook(&fixture.codex_config);

        let registry = fixture.registry();
        let mut section = std::pin::pin!(registry.ensure_ready_cancellable(
            PROJECT.to_string(),
            OPERATION.to_string(),
            LC.to_string(),
            fixture.root.clone(),
            vec![ProviderName::Codex],
            ProviderTrustScenario::Confirmation,
        ));
        // 等临界区开始（home 锁已获取 → 提交已开始，此后 spawn_blocking 已提交）。
        tokio::select! {
            outcome = &mut section => {
                panic!("section must park on the fact lock, got {outcome:?}");
            }
            _ = async {
                while attempts.try_recv().is_err() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            } => {}
        }
        // 提交边界后取消 caller：临界区须 detached 安全收尾，不误删。
        let cancelled = tokio::time::timeout(Duration::from_millis(100), &mut section).await;
        assert!(
            cancelled.is_err(),
            "in-flight section parked on the fact lock must not finish within the cancel window"
        );

        // 释放 fact 锁：临界区 detached 收尾（有界等待全部收尾事实落盘：
        // ownership 与末尾的 ensure 审计都在案才算收尾完成）。
        release_tx.send(()).unwrap();
        holder.join().unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let finalized = loop {
            let ownership = store
                .load_ownership(PROJECT, "codex", &trust_key)
                .unwrap()
                .filter(|record| record.owned);
            let audited = store
                .list_audit(PROJECT)
                .unwrap()
                .iter()
                .any(|audit| audit.action == "ensure" && audit.outcome == "registered");
            if ownership.is_some() && audited {
                break ownership;
            }
            assert!(
                Instant::now() < deadline,
                "detached section must finalize after cancellation"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(finalized.is_some());
        assert!(
            std::fs::read_to_string(&fixture.codex_config)
                .unwrap()
                .contains(&trust_key)
        );
        // 他人 waiting fact 原样保留；本 provider 的 waiting 不存在。
        let waiting = store.list_waiting(PROJECT).unwrap();
        assert!(waiting.iter().any(|record| record.provider == "kimi_code"));
        assert!(!waiting.iter().any(|record| record.provider == "codex"));
        // 收尾完成后 home 锁已释放。
        assert!(ExclusiveFileLock::acquire(&fixture.codex_config).is_ok());
    }
    fn sha256_hex_12(value: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(value.as_bytes());
        format!("{:x}", digest)[..12].to_string()
    }
}
