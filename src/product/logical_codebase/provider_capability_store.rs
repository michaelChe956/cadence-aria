//! Store-backed provider capability records.
//!
//! `ProviderCapabilityRecord` 是逻辑代码库 provider 能力的持久化事实来源,存于
//! `.aria/projects/{project_id}/logical-codebase/capabilities.json`。记录携带
//! 三态 evidence(`Declared`/`FixtureVerified`/`ProductionVerified`)、resume 能力
//! 三态与受支持的 action 列表,供 `StoreBackedProviderCapabilitySource` 在 launch
//! 前解析并 fail-closed。
//!
//! `ProviderRefType` 与 `ResumeEvidenceState`(gateway 侧)未派生 serde,持久化时
//! 用 String 承载(`provider_type`/`resume_evidence`),load 时 match 映射回枚举。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::policy::{ProviderDialect, SessionPolicyAction};
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

/// 单个 provider 的能力记录。`provider_type` 为 `ClaudeCode` | `Codex`;
/// `capability_snapshot_ref` 与 `provider_ref_for_name` 约定一致
/// (`cap_managed_snapshot`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapabilityRecord {
    pub provider_type: ProviderRefType,
    pub version: String,
    pub adapter_dialect: ProviderDialect,
    pub capability_snapshot_ref: String,
    pub evidence: CapabilityEvidence,
    pub resume_evidence: ResumeEvidenceState,
    pub supported_actions: Vec<SessionPolicyAction>,
}

impl ProviderCapabilityRecord {
    /// ClaudeCode bootstrap 记录:fixture 验证 + resume 确认,支持全部三个 action。
    fn claude_code_bootstrap() -> Self {
        Self {
            provider_type: ProviderRefType::ClaudeCode,
            version: BOOTSTRAP_VERSION.to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::FixtureVerified,
            resume_evidence: ResumeEvidenceState::Confirmed,
            supported_actions: vec![
                SessionPolicyAction::PlanningReadOnly,
                SessionPolicyAction::CodingTargetWrite,
                SessionPolicyAction::ReviewReadOnly,
            ],
        }
    }

    /// Codex bootstrap 记录:仅声明 + resume 不支持 + 空 action,即 unsupported。
    fn codex_bootstrap() -> Self {
        Self {
            provider_type: ProviderRefType::Codex,
            version: BOOTSTRAP_VERSION.to_string(),
            adapter_dialect: ProviderDialect::CodexCliV1,
            capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
            evidence: CapabilityEvidence::Declared,
            resume_evidence: ResumeEvidenceState::Unsupported,
            supported_actions: Vec::new(),
        }
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
            version: self.version.clone(),
            adapter_dialect: self.adapter_dialect,
            capability_snapshot_ref: self.capability_snapshot_ref.clone(),
            evidence: self.evidence,
            resume_evidence,
            supported_actions: self.supported_actions.clone(),
        })
    }
}

fn provider_type_to_string(provider_type: ProviderRefType) -> &'static str {
    match provider_type {
        ProviderRefType::ClaudeCode => "claude_code",
        ProviderRefType::Codex => "codex",
    }
}

fn provider_type_from_string(value: &str) -> Option<ProviderRefType> {
    match value {
        "claude_code" => Some(ProviderRefType::ClaudeCode),
        "codex" => Some(ProviderRefType::Codex),
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
}
