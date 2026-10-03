//! Store-backed provider capability records.
//!
//! `ProviderCapabilityRecord` 是逻辑代码库 provider 能力的持久化事实来源,存于
//! `.aria/projects/{project_id}/logical-codebase/capabilities.json`。v2 记录
//! 携带逐 action 三态矩阵(`ProviderActionCapability`:launch/resume/
//! write_boundary 各自 `Confirmed`/`Denied{reason}`/`Unknown`)、trust、探测
//! 引用与根 recipe 隔离证据;v1 旧记录(supported_actions/provenance/resume
//! 二态)仅 DTO 解码,矩阵一律读作 Unknown,不产生正常会话 Confirmed。
//!
//! `ProviderRefType` 与 `ResumeEvidenceState`(gateway 侧)未派生 serde,持久化时
//! 用 String 承载(`provider_type`/`resume_evidence`),load 时 match 映射回枚举。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyAction,
};
use crate::product::logical_codebase::provider_gateway::{ProviderRefType, ResumeEvidenceState};

/// 能力证据三态:区分「声明」「fixture 验证」与「生产验证」,避免只以单一布尔
/// 维度判定能力,使持久化记录可审计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEvidence {
    Declared,
    FixtureVerified,
    ProductionVerified,
}
/// v2 record 的持久化 schema 版本(冻结接口:Capability矩阵 DTO/持久化)。
pub const PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION: u32 = 2;

/// 逐 action 的三态 capability 行(冻结接口):`launch`/`resume`/
/// `write_boundary` 各自独立记录 `ProviderCapabilityEvidence`(1a 三态),
/// fresh 与 resume 分格,互不推导。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderActionCapability {
    pub action: SessionPolicyAction,
    pub launch: ProviderCapabilityEvidence,
    pub resume: ProviderCapabilityEvidence,
    pub write_boundary: ProviderCapabilityEvidence,
    /// 该 action 已实测 version 的完整权限 profile 摘要(证据摘要分层:
    /// profile 摘要不随单次 role 变化)。
    pub projection_digest: String,
    /// 探测证据引用(如 probe artifact ref),不得自造。
    pub evidence_ref: String,
}

impl ProviderActionCapability {
    /// 未探测行:三格全 Unknown,无 digest/证据引用。
    pub fn unknown(action: SessionPolicyAction) -> Self {
        Self {
            action,
            launch: ProviderCapabilityEvidence::Unknown,
            resume: ProviderCapabilityEvidence::Unknown,
            write_boundary: ProviderCapabilityEvidence::Unknown,
            projection_digest: String::new(),
            evidence_ref: String::new(),
        }
    }

    /// fresh 门:launch 与 write_boundary 均 `Confirmed` 才放行;resume 分格
    /// 状态不影响 fresh(fresh 必过 launch/write-boundary,resume Unknown
    /// 不阻止合法 fresh)。
    ///
    /// 阶段 1 编译桩:恒拒绝;阶段 2 实现真实三态判定。
    pub fn fresh_gate(&self) -> Result<(), ProviderActionCapabilityGateError> {
        Err(ProviderActionCapabilityGateError::LaunchNotConfirmed(
            self.launch.clone(),
        ))
    }

    /// 明确 resume 门:仅 resume `Confirmed` 放行,不得静默改 fresh。
    ///
    /// 阶段 1 编译桩:恒拒绝;阶段 2 实现真实三态判定。
    pub fn explicit_resume_gate(&self) -> Result<(), ProviderActionCapabilityGateError> {
        Err(ProviderActionCapabilityGateError::ResumeNotConfirmed(
            self.resume.clone(),
        ))
    }
}

/// 单行 capability 门的拒绝证据(带失败格的当前状态;2b source 层再包装为
/// gateway 错误)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderActionCapabilityGateError {
    LaunchNotConfirmed(ProviderCapabilityEvidence),
    WriteBoundaryNotConfirmed(ProviderCapabilityEvidence),
    ResumeNotConfirmed(ProviderCapabilityEvidence),
}

/// 逐 action 三态矩阵。`SessionPolicyAction`(1a 冻结文件,只读)无
/// Hash/Ord derive,故以 Vec 按固定 action 序承载;JSON 序列化为行数组。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderActionMatrix {
    rows: Vec<ProviderActionCapability>,
}

impl ProviderActionMatrix {
    /// 三个 canonical action 的全 Unknown 矩阵(v1 legacy 读取与 bootstrap
    /// 默认:默认值不得视作正常真实证据)。
    pub fn unknown_all() -> Self {
        Self {
            rows: [
                SessionPolicyAction::PlanningReadOnly,
                SessionPolicyAction::CodingTargetWrite,
                SessionPolicyAction::ReviewReadOnly,
            ]
            .iter()
            .map(|action| ProviderActionCapability::unknown(*action))
            .collect(),
        }
    }

    /// 由行构造:按固定 action 序排序保证确定性;重复 action 拒绝(fail-closed)。
    pub fn from_rows(mut rows: Vec<ProviderActionCapability>) -> Result<Self, ProductStoreError> {
        rows.sort_by_key(|row| Self::canonical_action_order(row.action));
        for pair in rows.windows(2) {
            if pair[0].action == pair[1].action {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "provider_capability_record",
                    reason: format!("duplicate action row: {:?}", pair[0].action),
                });
            }
        }
        Ok(Self { rows })
    }

    /// 按 action 取行;缺行视为未探测(全 Unknown 行),不 panic。
    pub fn row(&self, action: &SessionPolicyAction) -> ProviderActionCapability {
        self.rows
            .iter()
            .find(|row| row.action == *action)
            .cloned()
            .unwrap_or_else(|| ProviderActionCapability::unknown(*action))
    }

    fn canonical_action_order(action: SessionPolicyAction) -> u8 {
        match action {
            SessionPolicyAction::PlanningReadOnly => 0,
            SessionPolicyAction::CodingTargetWrite => 1,
            SessionPolicyAction::ReviewReadOnly => 2,
        }
    }
}

impl std::ops::Index<&SessionPolicyAction> for ProviderActionMatrix {
    type Output = ProviderActionCapability;

    /// 缺行 panic(矩阵消费方应保证 canonical 行存在;安全访问用 `row`)。
    fn index(&self, action: &SessionPolicyAction) -> &Self::Output {
        self.rows
            .iter()
            .find(|row| row.action == *action)
            .unwrap_or_else(|| panic!("action matrix 缺少 action 行: {action:?}"))
    }
}

/// 根 recipe 证据隔离格(record.root_recipe_evidence,冻结接口):仅承载固定
/// Claude root recipe 的已交付证据事实,与 normal action matrix 相互隔离,
/// 不参与 normal 会话放行,也不被 normal 会话消费(消费语义归后续切片)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RootRecipeEvidence {
    /// 尚无已交付 recipe 证据。
    None,
    /// 已交付的固定 Claude root recipe 事实(证据引用 + 钉定版本)。
    Delivered {
        evidence_ref: String,
        version: String,
    },
}

/// 单个 provider 的能力记录。`provider_type` 为 `ClaudeCode` | `Codex`;
/// `capability_snapshot_ref` 与 `provider_ref_for_name` 约定一致
/// (`cap_managed_snapshot`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityRecord {
    pub provider_type: ProviderRefType,
    /// 持久化 schema 版本;v2 写入恒为 `PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION`。
    pub schema_version: u32,
    pub version: String,
    pub adapter_dialect: ProviderDialect,
    /// wire(传输)dialect(1a 四家冻结枚举)。
    pub wire_dialect: ProviderWireDialect,
    pub capability_snapshot_ref: String,
    /// provenance(仅证据来源描述,不替代 action 逐格证据)。
    pub evidence: CapabilityEvidence,
    /// legacy 过渡字段(仅 DTO 解码;2b 原子迁移移除 normal 消费)。
    pub resume_evidence: ResumeEvidenceState,
    /// legacy 过渡字段(仅 DTO 解码;旧列表不产生正常会话 Confirmed)。
    pub supported_actions: Vec<SessionPolicyAction>,
    /// 逐 action 三态矩阵(v2 核心事实)。
    pub action_matrix: ProviderActionMatrix,
    /// provider trust 证据(三态)。
    pub trust: ProviderCapabilityEvidence,
    /// 最近真实探测时间(RFC3339;未探测为 None)。
    pub probed_at: Option<String>,
    /// 探测工件引用(未探测为 None)。
    pub probe_artifact_ref: Option<String>,
    /// 根 recipe 证据隔离格(独立于 normal 矩阵)。
    pub root_recipe_evidence: RootRecipeEvidence,
}

impl ProviderCapabilityRecord {
    /// ClaudeCode bootstrap 记录:v1 兼容形状(fixture 验证 + resume 确认 +
    /// 全三 action);v2 新字段保持未探测(矩阵全 Unknown)——bootstrap 默认值
    /// 不得视作正常真实证据(接地基线)。
    fn claude_code_bootstrap() -> Self {
        Self {
            provider_type: ProviderRefType::ClaudeCode,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: BOOTSTRAP_VERSION.to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::FixtureVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: vec![
                SessionPolicyAction::PlanningReadOnly,
                SessionPolicyAction::CodingTargetWrite,
                SessionPolicyAction::ReviewReadOnly,
            ],
            action_matrix: ProviderActionMatrix::unknown_all(),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: None,
            probe_artifact_ref: None,
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    /// Codex bootstrap 记录:仅声明 + resume 不支持 + 空 action,即 unsupported。
    fn codex_bootstrap() -> Self {
        Self {
            provider_type: ProviderRefType::Codex,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: BOOTSTRAP_VERSION.to_string(),
            adapter_dialect: ProviderDialect::CodexCliV1,
            wire_dialect: ProviderWireDialect::CodexAppServerRpc,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::Declared,
            resume_evidence: ResumeEvidenceState::Unsupported,
            supported_actions: Vec::new(),
            action_matrix: ProviderActionMatrix::unknown_all(),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: None,
            probe_artifact_ref: None,
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    /// 2b 原子迁移前的过渡构造器:以 v1 语义填充 v2 新字段(矩阵全 Unknown、
    /// trust Unknown、未探测)。仅供 2b 域既有 fixture 的编译桩
    /// (`production_policy_resolvers.rs:924` 的 FRU 基座),既有断言语义零变化;
    /// 2b 原子迁移时整体替换。
    pub fn legacy_transition_claude_code() -> Self {
        Self {
            provider_type: ProviderRefType::ClaudeCode,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: BOOTSTRAP_VERSION.to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::FixtureVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: vec![
                SessionPolicyAction::PlanningReadOnly,
                SessionPolicyAction::CodingTargetWrite,
                SessionPolicyAction::ReviewReadOnly,
            ],
            action_matrix: ProviderActionMatrix::unknown_all(),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: None,
            probe_artifact_ref: None,
            root_recipe_evidence: RootRecipeEvidence::None,
        }
    }

    /// 以当前 CLI 事实(exact version + wire dialect)解析 action 行:版本或
    /// wire 漂移时旧行整体失效为 Unknown,旧 Confirmed 不跨 CLI 版本沿用。
    ///
    /// 阶段 1 编译桩:恒返回全 Unknown 行;阶段 2 实现漂移判定。
    pub fn current_action_row(
        &self,
        wire_dialect: ProviderWireDialect,
        exact_version: &str,
        action: SessionPolicyAction,
    ) -> ProviderActionCapability {
        let _ = (wire_dialect, exact_version);
        ProviderActionCapability::unknown(action)
    }

    fn to_json(&self) -> ProviderCapabilityRecordJson {
        ProviderCapabilityRecordJson {
            provider_type: provider_type_to_string(self.provider_type).to_string(),
            version: self.version.clone(),
            adapter_dialect: self.adapter_dialect,
            capability_snapshot_ref: self.capability_snapshot_ref.clone(),
            evidence: self.evidence,
            resume_evidence: resume_evidence_to_string(self.resume_evidence).to_string(),
            supported_actions: self.supported_actions.clone(),
        }
    }
}

/// v1 记录无 wire dialect 字段,按 adapter dialect 唯一映射补默认(仅 v1
/// decode 使用;v1 行矩阵已读作全 Unknown,wire 仅作记录)。
fn wire_dialect_for_legacy(dialect: ProviderDialect) -> ProviderWireDialect {
    match dialect {
        ProviderDialect::ClaudeCodeCliV1 => ProviderWireDialect::ClaudeCodeStreamJson,
        ProviderDialect::CodexCliV1 => ProviderWireDialect::CodexAppServerRpc,
        ProviderDialect::PiRpcV1 => ProviderWireDialect::PiRpc,
        ProviderDialect::KimiAcpV1 => ProviderWireDialect::KimiAcp,
    }
}

/// 持久化 DTO:`ProviderRefType` 与 `ResumeEvidenceState` 无 serde 派生,用 String
/// 承载并在 load 时 match 映射回枚举。
#[derive(Debug, Serialize, Deserialize)]
struct ProviderCapabilityRecordJson {
    provider_type: String,
    version: String,
    adapter_dialect: ProviderDialect,
    capability_snapshot_ref: String,
    evidence: CapabilityEvidence,
    resume_evidence: String,
    supported_actions: Vec<SessionPolicyAction>,
}

impl ProviderCapabilityRecordJson {
    fn to_record(&self) -> Result<ProviderCapabilityRecord, ProductStoreError> {
        let provider_type = provider_type_from_string(&self.provider_type).ok_or_else(|| {
            ProductStoreError::InvalidRecord {
                kind: "provider_capability_record",
                reason: format!("unknown provider_type: {}", self.provider_type),
            }
        })?;
        let resume_evidence =
            resume_evidence_from_string(&self.resume_evidence).ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "provider_capability_record",
                    reason: format!("unknown resume_evidence: {}", self.resume_evidence),
                }
            })?;
        Ok(ProviderCapabilityRecord {
            provider_type,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: self.version.clone(),
            adapter_dialect: self.adapter_dialect,
            wire_dialect: wire_dialect_for_legacy(self.adapter_dialect),
            capability_snapshot_ref: self.capability_snapshot_ref.clone(),
            evidence: self.evidence,
            resume_evidence,
            supported_actions: self.supported_actions.clone(),
            // 阶段 1 编译桩:DTO 尚未携带 v2 字段,decode 恒为未探测默认;
            // 阶段 2 按 schema_version 分支(v1 → 全 Unknown,未知 schema 拒绝)。
            action_matrix: ProviderActionMatrix::unknown_all(),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: None,
            probe_artifact_ref: None,
            root_recipe_evidence: RootRecipeEvidence::None,
        })
    }
}

fn provider_type_to_string(provider_type: ProviderRefType) -> &'static str {
    match provider_type {
        ProviderRefType::ClaudeCode => "claude_code",
        ProviderRefType::Codex => "codex",
        ProviderRefType::Pi => "pi",
        ProviderRefType::KimiCode => "kimi_code",
    }
}

fn provider_type_from_string(value: &str) -> Option<ProviderRefType> {
    match value {
        "claude_code" => Some(ProviderRefType::ClaudeCode),
        "codex" => Some(ProviderRefType::Codex),
        "pi" => Some(ProviderRefType::Pi),
        "kimi_code" => Some(ProviderRefType::KimiCode),
        _ => None,
    }
}

fn resume_evidence_to_string(evidence: ResumeEvidenceState) -> &'static str {
    match evidence {
        ResumeEvidenceState::Confirmed => "confirmed",
        ResumeEvidenceState::Unsupported => "unsupported",
    }
}

fn resume_evidence_from_string(value: &str) -> Option<ResumeEvidenceState> {
    match value {
        "confirmed" => Some(ResumeEvidenceState::Confirmed),
        "unsupported" => Some(ResumeEvidenceState::Unsupported),
        _ => None,
    }
}

const BOOTSTRAP_VERSION: &str = "0.0.0-managed";
const MANAGED_CAPABILITY_SNAPSHOT_REF: &str = "cap_managed_snapshot";

/// provider capability 的持久化 store。
#[derive(Debug, Clone)]
pub struct ProviderCapabilityStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl ProviderCapabilityStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes capability reads/writes to one logical codebase subtree（v1.3）。
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    /// 读取指定 provider 的 capability 记录;文件或记录不存在返回 `Ok(None)`。
    pub fn get(
        &self,
        project_id: &str,
        provider_type: ProviderRefType,
    ) -> Result<Option<ProviderCapabilityRecord>, ProductStoreError> {
        let path = self.capabilities_path(project_id)?;
        if !path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
        })? {
            return Ok(None);
        }
        let records: Vec<ProviderCapabilityRecordJson> = read_json(&path)?;
        for record in records {
            let parsed = record.to_record()?;
            if parsed.provider_type == provider_type {
                return Ok(Some(parsed));
            }
        }
        Ok(None)
    }

    /// 插入或覆盖指定 provider 的 capability 记录。
    pub fn upsert(
        &self,
        project_id: &str,
        record: &ProviderCapabilityRecord,
    ) -> Result<(), ProductStoreError> {
        let path = self.capabilities_path(project_id)?;
        let mut records: Vec<ProviderCapabilityRecordJson> =
            if path.try_exists().map_err(|error| {
                ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
            })? {
                read_json(&path)?
            } else {
                Vec::new()
            };
        let json = record.to_json();
        if let Some(existing) = records
            .iter_mut()
            .find(|entry| entry.provider_type == json.provider_type)
        {
            *existing = json;
        } else {
            records.push(json);
        }
        write_json(&path, &records)
    }

    /// 确保存在 bootstrap 记录;幂等。文件不存在才写两条默认记录
    /// (ClaudeCode + Codex);文件已存在(含用户 upsert 后)直接 `Ok` 跳过,不覆盖。
    pub fn ensure_bootstrap(&self, project_id: &str) -> Result<(), ProductStoreError> {
        let path = self.capabilities_path(project_id)?;
        if path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
        })? {
            return Ok(());
        }
        let records = vec![
            ProviderCapabilityRecord::claude_code_bootstrap().to_json(),
            ProviderCapabilityRecord::codex_bootstrap().to_json(),
        ];
        write_json(&path, &records)
    }

    fn capabilities_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(
            crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)?
                .join("capabilities.json"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::policy::{ProviderDialect, SessionPolicyAction};
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };

    #[test]
    fn provider_capability_store_bootstrap_is_idempotent_and_writes_two_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));

        store.ensure_bootstrap("project_0001").unwrap();
        store.ensure_bootstrap("project_0001").unwrap(); // 幂等:第二次无副作用

        let claude = store
            .get("project_0001", ProviderRefType::ClaudeCode)
            .unwrap()
            .unwrap();
        assert_eq!(claude.version, "0.0.0-managed");
        assert_eq!(claude.adapter_dialect, ProviderDialect::ClaudeCodeCliV1);
        assert_eq!(claude.capability_snapshot_ref, "cap_managed_snapshot");
        assert_eq!(claude.evidence, CapabilityEvidence::FixtureVerified);
        assert_eq!(claude.resume_evidence, ResumeEvidenceState::Confirmed);
        assert_eq!(
            claude.supported_actions,
            vec![
                SessionPolicyAction::PlanningReadOnly,
                SessionPolicyAction::CodingTargetWrite,
                SessionPolicyAction::ReviewReadOnly,
            ]
        );

        let codex = store
            .get("project_0001", ProviderRefType::Codex)
            .unwrap()
            .unwrap();
        assert_eq!(codex.version, "0.0.0-managed");
        assert_eq!(codex.adapter_dialect, ProviderDialect::CodexCliV1);
        assert_eq!(codex.capability_snapshot_ref, "cap_managed_snapshot");
        assert_eq!(codex.evidence, CapabilityEvidence::Declared);
        assert_eq!(codex.resume_evidence, ResumeEvidenceState::Unsupported);
        assert!(codex.supported_actions.is_empty());

        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/capabilities.json")
                .exists()
        );
    }

    #[test]
    fn provider_capability_store_get_hits_existing_record_and_misses_absent() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));
        store.ensure_bootstrap("project_0001").unwrap();

        let claude = store
            .get("project_0001", ProviderRefType::ClaudeCode)
            .unwrap();
        assert!(claude.is_some());

        // 未 bootstrap 的项目 → 文件不存在 → None
        let missing = store
            .get("project_missing", ProviderRefType::ClaudeCode)
            .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn provider_capability_store_upsert_overwrites_existing_record() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));
        store.ensure_bootstrap("project_0001").unwrap();

        let updated = ProviderCapabilityRecord {
            provider_type: ProviderRefType::ClaudeCode,
            version: "1.2.3".to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: vec![SessionPolicyAction::CodingTargetWrite],
            ..ProviderCapabilityRecord::legacy_transition_claude_code()
        };
        store.upsert("project_0001", &updated).unwrap();

        let loaded = store
            .get("project_0001", ProviderRefType::ClaudeCode)
            .unwrap()
            .unwrap();
        assert_eq!(loaded, updated);
        // Codex 记录不被覆盖
        assert!(
            store
                .get("project_0001", ProviderRefType::Codex)
                .unwrap()
                .is_some()
        );
    }

    // ===== Task 2a(lcg_t02):capability 三态矩阵 DTO/roundtrip/legacy 迁移 =====

    /// 构造指定三态的矩阵行。
    fn lcg_t02_matrix_row(
        action: SessionPolicyAction,
        launch: ProviderCapabilityEvidence,
        resume: ProviderCapabilityEvidence,
        write_boundary: ProviderCapabilityEvidence,
    ) -> ProviderActionCapability {
        ProviderActionCapability {
            action,
            launch,
            resume,
            write_boundary,
            projection_digest: format!("projection-digest-{action:?}"),
            evidence_ref: format!("probe://{action:?}"),
        }
    }

    /// 构造 v2 Claude 记录(非 bootstrap 版本,便于漂移断言)。
    fn lcg_t02_v2_record(version: &str, matrix: ProviderActionMatrix) -> ProviderCapabilityRecord {
        ProviderCapabilityRecord {
            provider_type: ProviderRefType::ClaudeCode,
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: version.to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: vec![SessionPolicyAction::CodingTargetWrite],
            action_matrix: matrix,
            trust: ProviderCapabilityEvidence::Confirmed,
            probed_at: Some("2026-10-02T00:00:00Z".to_string()),
            probe_artifact_ref: Some("probe://artifact-0001".to_string()),
            root_recipe_evidence: RootRecipeEvidence::Delivered {
                evidence_ref: "recipe://evidence-0001".to_string(),
                version: "0.0.0-managed".to_string(),
            },
        }
    }

    fn lcg_t02_write_capabilities_json(temp: &tempfile::TempDir, json: &str) {
        let dir = temp.path().join("projects/project_0001/logical-codebase");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("capabilities.json"), json).unwrap();
    }

    /// v1 旧记录:无 schema_version/action_matrix,旧字段取「最强」值
    /// (supported_actions 全三 + fixture 验证 + resume 确认)。
    const LCG_T02_V1_RECORDS: &str = r#"[
  {
    "provider_type": "claude_code",
    "version": "0.0.0-managed",
    "adapter_dialect": "claude_code_cli_v1",
    "capability_snapshot_ref": "cap_managed_snapshot",
    "evidence": "fixture_verified",
    "resume_evidence": "confirmed",
    "supported_actions": ["planning_read_only", "coding_target_write", "review_read_only"]
  }
]"#;

    #[test]
    fn lcg_t02_matrix_round_trips_three_states() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));

        let matrix = ProviderActionMatrix::from_rows(vec![
            lcg_t02_matrix_row(
                SessionPolicyAction::PlanningReadOnly,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Denied {
                    reason: "boundary probe denied".to_string(),
                },
                ProviderCapabilityEvidence::Confirmed,
            ),
            lcg_t02_matrix_row(
                SessionPolicyAction::CodingTargetWrite,
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Denied {
                    reason: "root write probe rejected".to_string(),
                },
            ),
            lcg_t02_matrix_row(
                SessionPolicyAction::ReviewReadOnly,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Unknown,
                ProviderCapabilityEvidence::Unknown,
            ),
        ])
        .unwrap();
        let written = lcg_t02_v2_record("1.2.3", matrix);

        store.upsert("project_0001", &written).unwrap();
        let loaded = store
            .get("project_0001", ProviderRefType::ClaudeCode)
            .unwrap()
            .unwrap();

        // 三态经 JSON/store 往返不丢失(矩阵整体相等)。
        assert_eq!(loaded.action_matrix, written.action_matrix);
        // Denied 的 reason 字符串完整保留。
        assert_eq!(
            loaded.action_matrix[&SessionPolicyAction::PlanningReadOnly].resume,
            ProviderCapabilityEvidence::Denied {
                reason: "boundary probe denied".to_string()
            }
        );
        assert_eq!(
            loaded.action_matrix[&SessionPolicyAction::CodingTargetWrite].write_boundary,
            ProviderCapabilityEvidence::Denied {
                reason: "root write probe rejected".to_string()
            }
        );
        assert_eq!(
            loaded.action_matrix[&SessionPolicyAction::ReviewReadOnly].launch,
            ProviderCapabilityEvidence::Unknown
        );
        // v2 顶层字段一并往返。
        assert_eq!(
            loaded.schema_version,
            PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION
        );
        assert_eq!(loaded.version, "1.2.3");
        assert_eq!(
            loaded.wire_dialect,
            ProviderWireDialect::ClaudeCodeStreamJson
        );
        assert_eq!(loaded.trust, ProviderCapabilityEvidence::Confirmed);
        assert_eq!(loaded.probed_at.as_deref(), Some("2026-10-02T00:00:00Z"));
        assert_eq!(
            loaded.probe_artifact_ref.as_deref(),
            Some("probe://artifact-0001")
        );
        assert_eq!(
            loaded.root_recipe_evidence,
            RootRecipeEvidence::Delivered {
                evidence_ref: "recipe://evidence-0001".to_string(),
                version: "0.0.0-managed".to_string(),
            }
        );
    }

    #[test]
    fn lcg_t02_legacy_actions_read_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));

        lcg_t02_write_capabilities_json(&temp, LCG_T02_V1_RECORDS);
        let legacy = store
            .get("project_0001", ProviderRefType::ClaudeCode)
            .unwrap()
            .unwrap();

        // v1 旧记录缺矩阵 → 三个 action 全部 Unknown;旧 supported_actions/
        // provenance/resume 二态不产生任何 Confirmed(Global Constraints 第 2 条)。
        for action in [
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::ReviewReadOnly,
        ] {
            let legacy_row = legacy.action_matrix.row(&action);
            assert_eq!(legacy_row.launch, ProviderCapabilityEvidence::Unknown);
            assert_eq!(legacy_row.resume, ProviderCapabilityEvidence::Unknown);
            assert_eq!(
                legacy_row.write_boundary,
                ProviderCapabilityEvidence::Unknown
            );
        }

        // 未知 schema_version 拒绝(fail-closed)。
        let unknown_schema = r#"[
  {
    "provider_type": "claude_code",
    "version": "0.0.0-managed",
    "adapter_dialect": "claude_code_cli_v1",
    "capability_snapshot_ref": "cap_managed_snapshot",
    "evidence": "fixture_verified",
    "resume_evidence": "confirmed",
    "supported_actions": [],
    "schema_version": 3
  }
]"#;
        lcg_t02_write_capabilities_json(&temp, unknown_schema);
        assert!(
            store
                .get("project_0001", ProviderRefType::ClaudeCode)
                .is_err()
        );

        // 矩阵内未知 evidence 状态串拒绝(fail-closed)。
        let unknown_state = r#"[
  {
    "provider_type": "claude_code",
    "version": "0.0.0-managed",
    "adapter_dialect": "claude_code_cli_v1",
    "capability_snapshot_ref": "cap_managed_snapshot",
    "evidence": "fixture_verified",
    "resume_evidence": "confirmed",
    "supported_actions": [],
    "schema_version": 2,
    "wire_dialect": "claude-stream-json",
    "action_matrix": [
      {
        "action": "coding_target_write",
        "launch": {"state": "maybe"},
        "resume": {"state": "unknown"},
        "write_boundary": {"state": "unknown"},
        "projection_digest": "d",
        "evidence_ref": "r"
      }
    ],
    "trust": {"state": "unknown"},
    "probed_at": null,
    "probe_artifact_ref": null,
    "root_recipe_evidence": {"kind": "none"}
  }
]"#;
        lcg_t02_write_capabilities_json(&temp, unknown_state);
        assert!(
            store
                .get("project_0001", ProviderRefType::ClaudeCode)
                .is_err()
        );
    }

    #[test]
    fn lcg_t02_fresh_and_resume_use_separate_cells() {
        // launch+write_boundary Confirmed、resume Unknown:fresh 放行,明确 resume 拒绝。
        let row = lcg_t02_matrix_row(
            SessionPolicyAction::CodingTargetWrite,
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Unknown,
            ProviderCapabilityEvidence::Confirmed,
        );
        let fresh_with_launch_and_boundary_confirmed = row.fresh_gate();
        assert!(fresh_with_launch_and_boundary_confirmed.is_ok());
        let explicit_resume_with_unknown_resume = row.explicit_resume_gate();
        assert!(explicit_resume_with_unknown_resume.is_err());

        // resume 分格独立:resume Confirmed 只放行明确 resume,不改变 fresh 门。
        let resume_confirmed = ProviderActionCapability {
            resume: ProviderCapabilityEvidence::Confirmed,
            ..row.clone()
        };
        assert!(resume_confirmed.explicit_resume_gate().is_ok());
        assert!(resume_confirmed.fresh_gate().is_ok());

        // write_boundary Unknown:launch 单独 Confirmed 不足以放行 fresh。
        let boundary_unknown = ProviderActionCapability {
            write_boundary: ProviderCapabilityEvidence::Unknown,
            ..row.clone()
        };
        assert!(boundary_unknown.fresh_gate().is_err());
    }

    #[test]
    fn lcg_t02_cli_version_drift_invalidates_row() {
        let matrix = ProviderActionMatrix::from_rows(vec![lcg_t02_matrix_row(
            SessionPolicyAction::CodingTargetWrite,
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Confirmed,
        )])
        .unwrap();
        let record = lcg_t02_v2_record("1.2.3", matrix);

        // 同 CLI(exact version + wire 一致):已验证行保留 Confirmed。
        let same_cli_row = record.current_action_row(
            ProviderWireDialect::ClaudeCodeStreamJson,
            "1.2.3",
            SessionPolicyAction::CodingTargetWrite,
        );
        assert_eq!(same_cli_row.launch, ProviderCapabilityEvidence::Confirmed);

        // CLI 升级(exact version 漂移):旧行整体失效为 Unknown,零沿用。
        let new_cli_row = record.current_action_row(
            ProviderWireDialect::ClaudeCodeStreamJson,
            "2.0.0",
            SessionPolicyAction::CodingTargetWrite,
        );
        assert_eq!(new_cli_row.launch, ProviderCapabilityEvidence::Unknown);
        assert_eq!(new_cli_row.resume, ProviderCapabilityEvidence::Unknown);
        assert_eq!(
            new_cli_row.write_boundary,
            ProviderCapabilityEvidence::Unknown
        );

        // wire dialect 漂移同样失效。
        let wire_drift_row = record.current_action_row(
            ProviderWireDialect::CodexAppServerRpc,
            "1.2.3",
            SessionPolicyAction::CodingTargetWrite,
        );
        assert_eq!(wire_drift_row.launch, ProviderCapabilityEvidence::Unknown);
    }
}
