/// Deterministic aggregate preflight implementation backed by the on-disk
/// logical codebase state. Validates the manifest, canonical non-Git aggregate
/// root, member main checkouts and that the aggregate index excludes assets.
#[derive(Debug, Clone)]
pub struct DeterministicAggregatePreflightService {
    paths: ProductAppPaths,
}

impl DeterministicAggregatePreflightService {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }
}

impl AggregatePreflightService for DeterministicAggregatePreflightService {
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

        let store = LogicalCodebaseStore::new(self.paths.clone());
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
        AggregateRootPreflight::new(self.paths.clone())
            .validate(project_id, &manifest.provider_context_root, &candidate_paths)
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
        })
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
