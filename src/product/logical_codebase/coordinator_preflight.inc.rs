/// Deterministic aggregate preflight implementation backed by the on-disk
/// logical codebase state. Validates the manifest, canonical non-Git aggregate
/// root, member main checkouts and that the aggregate index excludes assets.
#[derive(Debug, Clone)]
pub struct DeterministicAggregatePreflightService {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl DeterministicAggregatePreflightService {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes member/checkout reads to one logical codebase subtree.
    pub fn for_lc(&self, lc_id: impl Into<String>) -> Self {
        Self {
            paths: self.paths.clone(),
            lc_id: Some(lc_id.into()),
        }
    }

    fn authority_store(&self) -> LogicalCodebaseStore {
        match &self.lc_id {
            Some(lc_id) => LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id.clone()),
            None => LogicalCodebaseStore::new(self.paths.clone()),
        }
    }
}

impl AggregatePreflightService for DeterministicAggregatePreflightService {
    fn rescoped(&self, lc_id: &str) -> Option<Arc<dyn AggregatePreflightService>> {
        Some(Arc::new(self.for_lc(lc_id)))
    }
    fn inspect(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        _cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        if manifest.project_id != project_id {
            return Err(AggregateInitializationError::Preflight {
                reason: format!(
                    "manifest project {} does not match requested project {}",
                    manifest.project_id, project_id
                ),
                retryable: false,
            });
        }
        let canonical_root =
            std::fs::canonicalize(&manifest.provider_context_root).map_err(|error| {
                AggregateInitializationError::Preflight {
                    reason: format!(
                        "aggregate root {} cannot be canonicalized: {error}",
                        manifest.provider_context_root.display()
                    ),
                    retryable: true,
                }
            })?;
        if canonical_root.join(".git").exists() {
            return Err(AggregateInitializationError::Preflight {
                reason: format!(
                    "aggregate root {} is a Git repository; choose its non-Git common parent",
                    canonical_root.display()
                ),
                retryable: false,
            });
        }
        // 聚合根可写性是后续每一步的硬前提（provider turn 要往根上写规则/
        // 配置/示例）。探测建删一个 `.aria-probe-<nonce>`，失败即
        // `aggregate_root_not_writable`，且不留残留。
        crate::product::logical_codebase::environment_probe::ensure_aggregate_root_writable(
            &canonical_root,
        )?;

        let store = self.authority_store();
        let members = store.list_members(project_id).map_err(|error| {
            AggregateInitializationError::Preflight {
                reason: format!("members could not be loaded: {error}"),
                retryable: true,
            }
        })?;
        let checkouts = store.list_checkouts(project_id).map_err(|error| {
            AggregateInitializationError::Preflight {
                reason: format!("checkouts could not be loaded: {error}"),
                retryable: true,
            }
        })?;

        let candidate_paths = members
            .iter()
            .filter_map(|member| {
                checkouts
                    .iter()
                    .find(|checkout| checkout.logical_repository_id == member.logical_repository_id)
                    .map(|checkout| checkout.canonical_path.clone())
            })
            .collect::<Vec<_>>();
        // 登记/重跑共用同一入口：根上已有的用户规则文件不是门禁，预检只核
        // root 结构、成员边界与 root 重叠，无需借用旧 receipt 证据。范围随
        // `lc_id` 划定自身——重跑复验跳过自身 manifest，其余 LC 仍拦。
        let root_preflight = match self.lc_id.as_deref() {
            Some(lc_id) => AggregateRootPreflight::for_lc(self.paths.clone(), lc_id),
            None => AggregateRootPreflight::new(self.paths.clone()),
        };
        root_preflight
            .validate(
                project_id,
                &manifest.provider_context_root,
                &candidate_paths,
            )
            .map_err(|error| AggregateInitializationError::Preflight {
                reason: format!("{}: {}", error.code(), error.message()),
                retryable: false,
            })?;

        let mut projections = Vec::with_capacity(members.len());
        for member in &members {
            let projection = project_member(member, &checkouts)?;
            projections.push(projection);
        }

        let manifest_digest = manifest_digest(manifest);
        Ok(AggregatePreflightSnapshot {
            aggregate_root: canonical_root.to_string_lossy().into_owned(),
            index_excludes_assets: true,
            members: projections,
            manifest_revision: manifest.membership_revision,
            manifest_digest,
            ..Default::default()
        })
    }
}

impl AggregatePreflightSnapshot {
    /// REQ-BOOT-09：把 `advance_remaining` 探测到的 codegraph 证据落进
    /// durable 预检快照。空版本 = 未装配探针（不伪造可用与版本号）。
    pub(crate) fn with_codegraph_evidence(mut self, version: &str) -> Self {
        self.codegraph_available = !version.is_empty();
        self.codegraph_version = (!version.is_empty()).then(|| version.to_string());
        self
    }
}

fn project_member(
    member: &CodebaseMemberRecord,
    checkouts: &[RepositoryCheckoutRecord],
) -> Result<AggregatePreflightMemberProjection, AggregateInitializationError> {
    let main = checkouts
        .iter()
        .find(|checkout| checkout.logical_repository_id == member.logical_repository_id)
        .ok_or_else(|| AggregateInitializationError::Preflight {
            reason: format!(
                "member {} has no recorded checkout",
                member.logical_repository_id.0
            ),
            retryable: false,
        })?;
    let canonical_path = std::fs::canonicalize(&main.canonical_path).map_err(|error| {
        AggregateInitializationError::Preflight {
            reason: format!(
                "member {} checkout {} cannot be canonicalized: {error}",
                member.logical_repository_id.0,
                main.canonical_path.display()
            ),
            retryable: true,
        }
    })?;
    if !canonical_path.join(".git").exists() {
        return Err(AggregateInitializationError::Preflight {
            reason: format!(
                "member {} checkout {} is not a Git root",
                member.logical_repository_id.0,
                canonical_path.display()
            ),
            retryable: false,
        });
    }
    Ok(AggregatePreflightMemberProjection {
        logical_repository_id: member.logical_repository_id.0.to_string(),
        checkout_id: main.checkout_id.0.to_string(),
        canonical_path: canonical_path.to_string_lossy().into_owned(),
        git_root: canonical_path.to_string_lossy().into_owned(),
        revision: main.revision.clone(),
    })
}

fn manifest_digest(manifest: &LogicalCodebaseManifest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(manifest.project_id.as_bytes());
    hasher.update(manifest.membership_revision.to_be_bytes());
    hasher.update(manifest.provider_context_root.to_string_lossy().as_bytes());
    for member in &manifest.member_ids {
        hasher.update(member.0.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}


