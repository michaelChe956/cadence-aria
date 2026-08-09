/// Deterministic repository-type detector backed by
/// [`crate::product::logical_codebase::RepositoryProfileDetector`]. Only reads
/// the member main checkout root: it never recurses, follows symlinks outside
/// the root, executes package scripts, runs `pnpm install`, Node or Java. The
/// evidence digest makes the observation byte-stable for the preflight record.
#[derive(Debug, Clone)]
pub struct DeterministicRepositoryTypeDetector;

impl DeterministicRepositoryTypeDetector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeterministicRepositoryTypeDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl RepositoryTypeDetector for DeterministicRepositoryTypeDetector {
    fn detect(
        &self,
        checkout_root: &std::path::Path,
        logical_repository_id: &str,
    ) -> Result<RepositoryTypeEvidence, AggregateInitializationError> {
        let profile =
            crate::product::logical_codebase::RepositoryProfileDetector::detect(checkout_root)
                .map_err(|error| AggregateInitializationError::Preflight {
                    reason: format!(
                        "repository type detection failed for {}: {error}",
                        checkout_root.display()
                    ),
                    retryable: true,
                })?;
        let profile_digest = evidence_digest(logical_repository_id, &profile.tech_stack);
        Ok(RepositoryTypeEvidence {
            logical_repository_id: logical_repository_id.to_string(),
            repo_type: profile.repo_type,
            tech_stack: profile.tech_stack,
            profile_digest: Some(profile_digest),
        })
    }
}

fn evidence_digest(logical_repository_id: &str, tech_stack: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(logical_repository_id.as_bytes());
    hasher.update(tech_stack.join(",").as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Resolve the aggregate initialization profile from the per-member evidence.
///
/// - All members `Backend` (Java/Maven/Gradle) → `JavaBackend`.
/// - All members `Frontend` (pnpm/Vite) → `FrontendPnpmVite`.
/// - A mix of both (or `Mixed`) → `Mixed`.
/// - Any `Unknown` member, or a profile that cannot be classified within the
///   requested scope, fails preflight closed.
pub fn resolve_aggregate_profile(
    evidence: &[RepositoryTypeEvidence],
) -> Result<AggregateInitializationProfile, AggregateInitializationError> {
    use crate::product::logical_codebase::types::RepositoryType;
    if evidence.is_empty() {
        return Err(AggregateInitializationError::Preflight {
            reason: "aggregate profile cannot be resolved without any members".to_string(),
            retryable: false,
        });
    }
    let mut any_backend = false;
    let mut any_frontend = false;
    for item in evidence {
        match item.repo_type {
            RepositoryType::Backend => any_backend = true,
            RepositoryType::Frontend => any_frontend = true,
            RepositoryType::Mixed => {
                any_backend = true;
                any_frontend = true;
            }
            RepositoryType::Library => {
                // A pure library member does not by itself force a profile; it
                // stays neutral and lets the backend/frontend members decide.
            }
            RepositoryType::Unknown => {
                return Err(AggregateInitializationError::Preflight {
                    reason: format!(
                        "member {} has an unknown repository type; profile cannot be classified",
                        item.logical_repository_id
                    ),
                    retryable: false,
                });
            }
        }
    }
    Ok(match (any_backend, any_frontend) {
        (true, false) => AggregateInitializationProfile::JavaBackend,
        (false, true) => AggregateInitializationProfile::FrontendPnpmVite,
        (false, false) => AggregateInitializationProfile::FrontendPnpmVite,
        (true, true) => AggregateInitializationProfile::Mixed,
    })
}

/// Profile-specific preflight command templates. Frontend pnpm/Vite never
/// includes Maven/Gradle commands; the Java/Mixed templates carry the Java
/// build commands. The five stable step IDs are unaffected.
pub fn profile_preflight_commands(profile: AggregateInitializationProfile) -> Vec<String> {
    match profile {
        AggregateInitializationProfile::FrontendPnpmVite => vec![
            "pnpm --version".to_string(),
            "pnpm exec vite --version".to_string(),
        ],
        AggregateInitializationProfile::JavaBackend => vec![
            "mvn -v".to_string(),
            "git rev-parse --show-toplevel".to_string(),
        ],
        AggregateInitializationProfile::Mixed => vec![
            "pnpm --version".to_string(),
            "mvn -v".to_string(),
            "git rev-parse --show-toplevel".to_string(),
        ],
    }
}
