// ---------------------------------------------------------------------------
// R2:政策重新发布(Republish)——冷启动 drift 等待面的唯一自愈通道。
//
// 历史 root recipe receipt 是不可变审计事实，因此重新发布不改写任何
// receipt：它铸一个新的发布 identity(不是 aggregate initialization
// operation，五步状态机与 receipt 链保持原样)，以当前源重新合成并走 R1
// 原子发布链，再落一条 supersede 记录，声明「这份 artifact + 这份 rule
// digest 已经 supersede 某条 receipt 冻结的摘要」。readiness 投影据此在
// drift 等待面上收敛为 Completed；旧 receipt 原样保留。
// ---------------------------------------------------------------------------

/// per-LC scope root 下的重新发布记录文件名。
pub(crate) const POLICY_REPUBLICATION_FILE: &str = "aggregate-policy-republication.json";

/// 一条显式重新发布的 durable 事实。
///
/// 语义:`created_at` 时刻以 `sources` 重新合成得到 `policy_id/revision/
/// digest`,当前根规则摘要为 `rule_digest`;它 supersede 了
/// `superseded_receipt_operation_id` 那条 receipt 冻结的
/// `superseded_receipt_policy_digest` / `superseded_receipt_rule_digest`。
/// `prior_*` 只用于审计(重新发布前的 artifact 身份)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AggregatePolicyRepublication {
    pub(crate) republication_id: String,
    pub(crate) command_id: String,
    pub(crate) project_id: String,
    pub(crate) logical_codebase_id: String,
    pub(crate) canonical_root: PathBuf,
    pub(crate) superseded_receipt_operation_id: Option<String>,
    pub(crate) superseded_receipt_policy_digest: String,
    pub(crate) superseded_receipt_rule_digest: String,
    pub(crate) prior_policy_id: String,
    pub(crate) prior_revision: u64,
    pub(crate) prior_digest: String,
    pub(crate) policy_id: String,
    pub(crate) revision: u64,
    pub(crate) digest: String,
    pub(crate) rule_digest: String,
    pub(crate) sources: Vec<AggregatePolicySourceDigest>,
    pub(crate) created_at: String,
}

impl AggregatePolicyRepublication {
    /// 记录指向的是当前 artifact 本身(重新发布之后没有被更晚的发布推进)。
    pub(crate) fn covers_artifact(&self, artifact: &AggregatePolicyArtifact) -> bool {
        self.policy_id == artifact.policy_id && self.digest == artifact.digest
    }

    fn supersedes_receipt(&self, receipt_operation_id: &str) -> bool {
        self.superseded_receipt_operation_id.as_deref() == Some(receipt_operation_id)
    }

    /// R2.5:该 receipt 冻结的 policy digest 已被重新发布 supersede。
    pub(crate) fn supersedes_receipt_policy(
        &self,
        receipt_operation_id: &str,
        artifact: &AggregatePolicyArtifact,
        frozen_policy_digest: &str,
    ) -> bool {
        self.supersedes_receipt(receipt_operation_id)
            && self.covers_artifact(artifact)
            && (self.superseded_receipt_policy_digest == frozen_policy_digest
                || self.digest == frozen_policy_digest)
    }

    /// R2.5:该 receipt 冻结的 rule digest 已被重新发布 supersede，且记录的
    /// rule digest 仍是当前根的摘要(源再次变动则重新回到 drift 等待)。
    pub(crate) fn supersedes_receipt_rule(
        &self,
        receipt_operation_id: &str,
        artifact: &AggregatePolicyArtifact,
        frozen_rule_digest: &str,
        current_rule_digest: &str,
    ) -> bool {
        self.supersedes_receipt(receipt_operation_id)
            && self.covers_artifact(artifact)
            && self.rule_digest == current_rule_digest
            && (self.superseded_receipt_rule_digest == frozen_rule_digest
                || self.rule_digest == frozen_rule_digest)
    }

    fn validate(&self, project_id: &str) -> Result<(), ProductStoreError> {
        if self.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "aggregate_policy_republication",
                id: project_id.to_string(),
            });
        }
        if self.republication_id.trim().is_empty() || self.command_id.trim().is_empty() {
            return Err(invalid_publication(
                "republication_id/command_id must not be empty",
            ));
        }
        if self.revision == 0 || !self.digest.starts_with("sha256:") {
            return Err(invalid_publication(
                "republication must reference a positive revision and a canonical digest",
            ));
        }
        if self.rule_digest != self.sources.first().map(|s| s.digest.clone()).unwrap_or_default() {
            return Err(invalid_publication(
                "republication rule digest must be the entry source digest",
            ));
        }
        Ok(())
    }
}

/// 最新一条 root receipt 冻结的摘要三元(= readiness 投影比对的那条)。
struct RootReceiptIdentity {
    operation_id: String,
    policy_digest: String,
    rule_digest: String,
}

impl AggregatePolicyArtifactStore {
    fn republication_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .scope_root(project_id)?
            .join(POLICY_REPUBLICATION_FILE))
    }

    /// 当前生效的重新发布记录(纯读)。
    pub(crate) fn republication(
        &self,
        project_id: &str,
    ) -> Result<Option<AggregatePolicyRepublication>, ProductStoreError> {
        let path = self.republication_path(project_id)?;
        if !path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", path.display()))
        })? {
            return Ok(None);
        }
        let record: AggregatePolicyRepublication = read_json(&path)?;
        record.validate(project_id)?;
        Ok(Some(record))
    }

    /// 落重新发布记录:revision 不得回退(正文未变的重新发布复用同 revision)。
    fn save_republication(
        &self,
        project_id: &str,
        record: &AggregatePolicyRepublication,
    ) -> Result<(), ProductStoreError> {
        record.validate(project_id)?;
        if let Some(previous) = self.republication(project_id)? {
            if previous.logical_codebase_id != record.logical_codebase_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "aggregate_policy_republication",
                    id: record.logical_codebase_id.clone(),
                });
            }
            if record.revision < previous.revision {
                return Err(invalid_publication(format!(
                    "republication revision must not go backwards from {} to {}",
                    previous.revision, record.revision
                )));
            }
        }
        write_json_durable(&self.republication_path(project_id)?, record)
    }

    /// 最新 recipe operation 的最终 receipt 摘要(不存在时 `Ok(None)`)。
    fn latest_root_receipt_identity(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Option<RootReceiptIdentity>, ProductStoreError> {
        let operations =
            AggregateInitializationOperationStore::for_lc(self.paths.clone(), logical_codebase_id)
                .list(project_id)?;
        let Some(latest) = operations.first() else {
            return Ok(None);
        };
        let Some(receipt) =
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                self.paths.clone(),
                logical_codebase_id,
            )
            .get(project_id, &latest.operation_id)?
        else {
            return Ok(None);
        };
        Ok(Some(RootReceiptIdentity {
            operation_id: latest.operation_id.clone(),
            policy_digest: receipt.policy_digest,
            rule_digest: receipt.rule_digest,
        }))
    }

    /// R2:以当前源重新合成并发布(R1 原子链)，返回记录与「是否同 command
    /// 重放」。同 `command_id` 重放返回既有记录且不新增 revision。
    pub(crate) fn republish_policy(
        &self,
        manifest: &LogicalCodebaseManifest,
        canonical_root: &Path,
        command_id: &str,
        created_at: String,
    ) -> Result<(AggregatePolicyRepublication, bool), ProductStoreError> {
        validate_relative_id(&manifest.project_id)?;
        validate_relative_id(command_id)?;
        let root = std::fs::canonicalize(canonical_root).map_err(|error| {
            ProductStoreError::Io(format!("canonicalize {}: {error}", canonical_root.display()))
        })?;
        ensure_canonical_matches(&manifest.provider_context_root, &root, "manifest")?;
        let project_id = manifest.project_id.as_str();
        let logical_codebase_id = manifest.logical_codebase_id.to_string();

        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            if let Some(existing) = self.republication(project_id)?
                && existing.command_id == command_id
            {
                let artifact = self.get(project_id)?.ok_or_else(|| {
                    invalid_publication("current aggregate policy artifact disappeared")
                })?;
                if !existing.covers_artifact(&artifact) {
                    return Err(invalid_publication(format!(
                        "republication {} no longer matches the current artifact",
                        existing.republication_id
                    )));
                }
                return Ok((existing, true));
            }

            let prior = self.get(project_id)?;
            let superseded = self.latest_root_receipt_identity(project_id, &logical_codebase_id)?;
            let sources = collect_policy_sources(&root)?;
            let source_digests = source_digests_of(&sources);
            let rule_digest = sha256_hex(&sources[0].bytes);
            let policy_text = build_policy_text(&sources)?;
            let decision = decide_publication(
                project_id,
                &logical_codebase_id,
                &root,
                prior.as_ref(),
                policy_text,
                created_at.clone(),
            )?;
            let republication_id = format!("policy-republish-{}", uuid::Uuid::new_v4());
            let output_path = self.publication_output_path(project_id, &republication_id)?;
            let output = AggregatePolicyPublicationOutput {
                project_id: project_id.to_string(),
                lc_id: self.scoped_lc_id(project_id),
                logical_codebase_id: decision.artifact.logical_codebase_id.clone(),
                operation_id: republication_id.clone(),
                canonical_root: root.clone(),
                base_policy: decision.base.clone(),
                artifact: decision.artifact.clone(),
                sources: source_digests.clone(),
                rule_digest: rule_digest.clone(),
            };
            self.commit_publication(
                project_id,
                &root,
                &decision.artifact,
                &output,
                &output_path,
            )?;

            let record = AggregatePolicyRepublication {
                republication_id,
                command_id: command_id.to_string(),
                project_id: project_id.to_string(),
                logical_codebase_id: decision.artifact.logical_codebase_id.clone(),
                canonical_root: root.clone(),
                superseded_receipt_operation_id: superseded
                    .as_ref()
                    .map(|identity| identity.operation_id.clone()),
                superseded_receipt_policy_digest: superseded
                    .as_ref()
                    .map(|identity| identity.policy_digest.clone())
                    .unwrap_or_default(),
                superseded_receipt_rule_digest: superseded
                    .as_ref()
                    .map(|identity| identity.rule_digest.clone())
                    .unwrap_or_default(),
                prior_policy_id: prior
                    .as_ref()
                    .map(|artifact| artifact.policy_id.clone())
                    .unwrap_or_default(),
                prior_revision: prior.as_ref().map(|artifact| artifact.revision).unwrap_or(0),
                prior_digest: prior
                    .as_ref()
                    .map(|artifact| artifact.digest.clone())
                    .unwrap_or_default(),
                policy_id: decision.artifact.policy_id.clone(),
                revision: decision.artifact.revision,
                digest: decision.artifact.digest.clone(),
                rule_digest,
                sources: source_digests,
                created_at,
            };
            self.save_republication(project_id, &record)?;
            self.retain_policy_revisions_best_effort(project_id, &logical_codebase_id, &root);
            Ok((record, false))
        })
    }
}
