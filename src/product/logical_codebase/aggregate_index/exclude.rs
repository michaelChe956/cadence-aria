use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::json_store::{validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest, MemberStatus,
    RepositoryCheckoutRecord,
};

use super::AggregateIndexError;

const BUILTIN_EXCLUDES: [&str; 11] = [
    "**/.worktrees/",
    "**/.aria/",
    "**/.git/",
    "**/build/",
    "**/target/",
    "**/node_modules/",
    "**/dist/",
    "**/.env",
    "**/.env.*",
    "**/*credential*",
    "**/*secret*",
];

/// The CodeGraph configuration written at a logical-codebase aggregate root.
///
/// CodeGraph v1.5.0 has no allowlist option.  The denylist is consequently the
/// range contract: every direct child which is not a manifest member is excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphConfig {
    pub exclude: Vec<String>,
}

pub struct CodeGraphExcludeGenerator;

impl CodeGraphExcludeGenerator {
    /// Produces the denylist from the authoritative logical-codebase manifest.
    ///
    /// A member may be indexed only when its single main checkout resolves to a
    /// direct child of `provider_context_root`.  Rejecting layouts which cannot
    /// meet that invariant prevents an incomplete denylist from widening the
    /// index scope.
    pub fn generate(
        &self,
        manifest: &LogicalCodebaseManifest,
        members: &[CodebaseMemberRecord],
        checkouts: &[RepositoryCheckoutRecord],
    ) -> Result<CodeGraphConfig, AggregateIndexError> {
        let root = canonical_aggregate_root(&manifest.provider_context_root)?;
        let member_names = member_root_names(manifest, members, checkouts, &root)?;
        let member_names = member_names.iter().map(String::as_str).collect::<Vec<_>>();
        Self::from_member_roots(&root, &member_names)
    }

    /// Builds the root-level denylist from validated member directory names.
    ///
    /// This helper is intentionally public for callers which have already
    /// projected member paths, while `generate` is the authority-backed API.
    pub fn from_member_roots(
        root: &Path,
        member_names: &[&str],
    ) -> Result<CodeGraphConfig, AggregateIndexError> {
        let allowed = validated_member_names(member_names)?;
        let mut exclude = Vec::new();
        let entries = std::fs::read_dir(root).map_err(io_error)?;
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name().into_string().map_err(|name| {
                layout_unsupported(format!(
                    "aggregate root {} contains a non-UTF-8 entry name: {}",
                    root.display(),
                    PathBuf::from(name).display()
                ))
            })?;
            if !allowed.contains(&name) && name != "codegraph.json" && name != ".codegraph" {
                exclude.push(format!("{name}/"));
            }
        }
        exclude.extend(BUILTIN_EXCLUDES.into_iter().map(str::to_string));
        exclude.sort();
        exclude.dedup();
        Ok(CodeGraphConfig { exclude })
    }

    /// Atomically replaces only the aggregate-root configuration and returns
    /// the digest of the exact JSON bytes written.
    pub fn write_atomically(
        &self,
        aggregate_root: &Path,
        config: &CodeGraphConfig,
    ) -> Result<String, AggregateIndexError> {
        let encoded = serde_json::to_vec_pretty(config).map_err(json_error)?;
        // `write_json` uses a same-directory create_new temporary file, sync_all,
        // and rename.  The fixed file name ensures the .codegraph database is
        // never selected as the atomic-write target.
        write_json(&aggregate_root.join("codegraph.json"), config)?;
        Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
    }
}

fn member_root_names(
    manifest: &LogicalCodebaseManifest,
    members: &[CodebaseMemberRecord],
    checkouts: &[RepositoryCheckoutRecord],
    canonical_root: &Path,
) -> Result<BTreeSet<String>, AggregateIndexError> {
    let mut manifest_ids = BTreeSet::new();
    for member_id in &manifest.member_ids {
        if !manifest_ids.insert(*member_id) {
            return Err(layout_unsupported(format!(
                "manifest {} repeats member {}",
                manifest.project_id, member_id.0
            )));
        }
    }

    let mut members_by_id = BTreeMap::new();
    for member in members {
        if members_by_id
            .insert(member.logical_repository_id, member)
            .is_some()
        {
            return Err(layout_unsupported(format!(
                "member records repeat logical repository {}",
                member.logical_repository_id.0
            )));
        }
    }

    let mut names = BTreeSet::new();
    for member_id in manifest_ids {
        let member = members_by_id.get(&member_id).ok_or_else(|| {
            layout_unsupported(format!(
                "manifest member {} has no authority member record",
                member_id.0
            ))
        })?;
        if member.status != MemberStatus::Active {
            return Err(layout_unsupported(format!(
                "manifest member {} is not active",
                member_id.0
            )));
        }

        let main_checkouts = checkouts
            .iter()
            .filter(|checkout| {
                checkout.logical_repository_id == member_id && checkout.kind == CheckoutKind::Main
            })
            .collect::<Vec<_>>();
        let [checkout] = main_checkouts.as_slice() else {
            return Err(layout_unsupported(format!(
                "manifest member {} must have exactly one main checkout, found {}",
                member_id.0,
                main_checkouts.len()
            )));
        };
        if !member.checkout_ids.contains(&checkout.checkout_id)
            || member.physical_repository_id != checkout.physical_repository_id
        {
            return Err(layout_unsupported(format!(
                "main checkout {} does not belong to manifest member {}",
                checkout.checkout_id.0, member_id.0
            )));
        }

        let checkout_path = std::fs::canonicalize(&checkout.canonical_path).map_err(|error| {
            layout_unsupported(format!(
                "main checkout {} for member {} cannot be canonicalized: {error}",
                checkout.canonical_path.display(),
                member_id.0
            ))
        })?;
        if checkout_path.parent() != Some(canonical_root) {
            return Err(layout_unsupported(format!(
                "main checkout {} for member {} is not a direct child of aggregate root {}",
                checkout_path.display(),
                member_id.0,
                canonical_root.display()
            )));
        }
        let name = checkout_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                layout_unsupported(format!(
                    "main checkout {} for member {} has no UTF-8 directory name",
                    checkout_path.display(),
                    member_id.0
                ))
            })?;
        validate_member_name(name)?;
        if !names.insert(name.to_string()) {
            return Err(layout_unsupported(format!(
                "manifest members resolve to duplicate aggregate child {name}"
            )));
        }
    }
    Ok(names)
}

fn canonical_aggregate_root(root: &Path) -> Result<PathBuf, AggregateIndexError> {
    std::fs::canonicalize(root).map_err(|error| {
        layout_unsupported(format!(
            "aggregate root {} cannot be canonicalized: {error}",
            root.display()
        ))
    })
}

fn validated_member_names(member_names: &[&str]) -> Result<BTreeSet<String>, AggregateIndexError> {
    let mut allowed = BTreeSet::new();
    for name in member_names {
        validate_member_name(name)?;
        if !allowed.insert((*name).to_string()) {
            return Err(layout_unsupported(format!(
                "manifest members resolve to duplicate aggregate child {name}"
            )));
        }
    }
    Ok(allowed)
}

fn validate_member_name(name: &str) -> Result<(), AggregateIndexError> {
    validate_relative_id(name).map_err(|error| {
        layout_unsupported(format!(
            "aggregate member directory {name:?} is invalid: {error}"
        ))
    })
}

fn layout_unsupported(message: String) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_layout_unsupported",
        message,
    }
}

fn io_error(error: io::Error) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_io",
        message: error.to_string(),
    }
}

fn json_error(error: serde_json::Error) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_json",
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::product::logical_codebase::{
        CheckoutAvailability, LogicalRepositoryId, RepositoryCheckoutId, RepositorySourceIdentity,
        RepositoryType,
    };
    use uuid::Uuid;

    #[test]
    fn generated_denylist_excludes_non_members_and_non_builtin_aggregate_assets() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("api/.worktrees/issue/src")).unwrap();
        std::fs::create_dir_all(root.path().join("api/.aria")).unwrap();
        std::fs::create_dir_all(root.path().join("web/src")).unwrap();
        std::fs::create_dir_all(root.path().join("not-a-repo/src")).unwrap();
        let config =
            CodeGraphExcludeGenerator::from_member_roots(root.path(), &["api", "web"]).unwrap();

        assert!(config.exclude.contains(&"not-a-repo/".to_string()));
        assert!(config.exclude.contains(&"**/.worktrees/".to_string()));
        assert!(config.exclude.contains(&"**/.aria/".to_string()));
        assert!(config.exclude.contains(&"**/build/".to_string()));
        assert!(config.exclude.contains(&"**/*credential*".to_string()));
        assert!(config.exclude.contains(&"**/*secret*".to_string()));
        assert!(
            !config
                .exclude
                .iter()
                .any(|entry| entry == "api/" || entry == "web/")
        );
    }

    #[test]
    fn generate_uses_manifest_members_so_removed_members_are_excluded_on_republish() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("api/src")).unwrap();
        std::fs::create_dir_all(root.path().join("web/src")).unwrap();
        let (api_member, api_checkout) = member_with_main_checkout(root.path(), "api");
        let (web_member, web_checkout) = member_with_main_checkout(root.path(), "web");
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root.path().to_path_buf(),
            vec![api_member.logical_repository_id],
        );

        let config = CodeGraphExcludeGenerator
            .generate(
                &manifest,
                &[api_member, web_member],
                &[api_checkout, web_checkout],
            )
            .unwrap();

        assert!(!config.exclude.contains(&"api/".to_string()));
        assert!(config.exclude.contains(&"web/".to_string()));
    }

    #[test]
    fn generate_rejects_main_checkout_outside_direct_aggregate_child_layout() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("api/.worktrees/issue")).unwrap();
        let (member, mut checkout) = member_with_main_checkout(root.path(), "api");
        checkout.canonical_path = root.path().join("api/.worktrees/issue");
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root.path().to_path_buf(),
            vec![member.logical_repository_id],
        );

        let error = CodeGraphExcludeGenerator
            .generate(&manifest, &[member], &[checkout])
            .unwrap_err();

        assert_layout_unsupported(error);
    }

    #[test]
    fn generate_rejects_duplicate_member_root_names() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("api/src")).unwrap();
        let (first_member, first_checkout) = member_with_main_checkout(root.path(), "api");
        let (second_member, mut second_checkout) = member_with_main_checkout(root.path(), "api");
        second_checkout.canonical_path = first_checkout.canonical_path.clone();
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root.path().to_path_buf(),
            vec![
                first_member.logical_repository_id,
                second_member.logical_repository_id,
            ],
        );

        let error = CodeGraphExcludeGenerator
            .generate(
                &manifest,
                &[first_member, second_member],
                &[first_checkout, second_checkout],
            )
            .unwrap_err();

        assert_layout_unsupported(error);
    }

    #[cfg(unix)]
    #[test]
    fn generate_rejects_member_checkout_symlink_escaping_aggregate_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("src")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("api")).unwrap();
        let (member, checkout) = member_with_main_checkout(root.path(), "api");
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root.path().to_path_buf(),
            vec![member.logical_repository_id],
        );

        let error = CodeGraphExcludeGenerator
            .generate(&manifest, &[member], &[checkout])
            .unwrap_err();

        assert_layout_unsupported(error);
    }

    #[test]
    fn write_atomically_replaces_only_codegraph_configuration_and_returns_its_digest() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".codegraph")).unwrap();
        let config = CodeGraphConfig {
            exclude: vec!["former-member/".to_string(), "**/target/".to_string()],
        };

        let digest = CodeGraphExcludeGenerator
            .write_atomically(root.path(), &config)
            .unwrap();

        assert_eq!(
            digest,
            format!(
                "sha256:{:x}",
                Sha256::digest(serde_json::to_vec_pretty(&config).unwrap())
            )
        );
        let written: CodeGraphConfig =
            serde_json::from_slice(&std::fs::read(root.path().join("codegraph.json")).unwrap())
                .unwrap();
        assert_eq!(written, config);
        assert!(root.path().join(".codegraph").is_dir());
    }

    fn member_with_main_checkout(
        root: &Path,
        name: &str,
    ) -> (CodebaseMemberRecord, RepositoryCheckoutRecord) {
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let path = root.join(name);
        let now = "2026-08-09T00:00:00Z".to_string();
        let source_identity = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: "sha256:source".to_string(),
            canonical_git_dir: path.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: "sha256:path".to_string(),
        };
        let member = CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: format!("repository_{name}"),
            alias: name.to_string(),
            role: "repository".to_string(),
            ordinal: 1,
            source_identity: source_identity.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let checkout = RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: member.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path: path,
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        (member, checkout)
    }

    fn assert_layout_unsupported(error: AggregateIndexError) {
        assert!(matches!(
            error,
            AggregateIndexError::Failed {
                code: "aggregate_index_layout_unsupported",
                ..
            }
        ));
    }
}
