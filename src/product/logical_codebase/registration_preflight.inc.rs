pub struct AggregateRootPreflight {
    paths: ProductAppPaths,
}

impl AggregateRootPreflight {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 公开登记入口保持严格：任何 AGENTS.md/CLAUDE.md/`.aria` 在场一律
    /// 拒绝，不接收任何证明参数；首次登记的用户文件冲突检查永不放宽。
    pub fn validate(
        &self,
        project_id: &str,
        root: &Path,
        candidate_paths: &[PathBuf],
    ) -> Result<CanonicalAggregateRoot, AggregateRootPreflightError> {
        self.validate_common(project_id, root, candidate_paths, None)
    }

    /// Task 3（aggregate-policy-root-publication，REQ-BOOT-06）：recipe 复用
    /// 需 receipt 证明——producer 侧重跑预检的窄复验入口。map 只允许
    /// `AGENTS.md` 与可选 `CLAUDE.md`，值是各自旧 snapshot 原字节 digest；
    /// 入口复验普通文件、无 symlink 及当前字节一致，只豁免本次复验通过
    /// 的这两个文件的存在冲突。`.aria` 与其他一切用户文件永不豁免。
    pub(crate) fn validate_recipe_replay(
        &self,
        project_id: &str,
        root: &Path,
        candidate_paths: &[PathBuf],
        proven_entry_digests: &std::collections::BTreeMap<String, String>,
    ) -> Result<CanonicalAggregateRoot, AggregateRootPreflightError> {
        for name in proven_entry_digests.keys() {
            if name != "AGENTS.md" && name != "CLAUDE.md" {
                return Err(preflight_error(
                    "aggregate_root_ownership_conflict",
                    format!(
                        "proven root entry {name:?} is not an exemptible recipe artifact; \
                         only AGENTS.md/CLAUDE.md can be receipt-proven for recipe replay"
                    ),
                ));
            }
        }
        self.validate_common(
            project_id,
            root,
            candidate_paths,
            Some(proven_entry_digests),
        )
    }

    /// validate/validate_recipe_replay 的共享核心：project id、canonical
    /// non-Git root、成员 canonical/越界/worktree、重叠检查全部照跑；
    /// `proven_entry_digests` 只影响 reject_owned_root_files 对 AGENTS/CLAUDE
    /// 存在冲突的窄豁免。
    fn validate_common(
        &self,
        project_id: &str,
        root: &Path,
        candidate_paths: &[PathBuf],
        proven_entry_digests: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<CanonicalAggregateRoot, AggregateRootPreflightError> {
        validate_relative_id(project_id).map_err(|error| {
            preflight_error(
                "aggregate_root_invalid_project_id",
                format!(
                    "project ID {project_id:?} is not a safe relative identifier: {error}; use a project ID without path separators"
                ),
            )
        })?;

        let canonical_root = canonicalize_for_preflight(root, "aggregate_root_missing")?;
        if !canonical_root.is_dir() {
            return Err(preflight_error(
                "aggregate_root_missing",
                format!(
                    "aggregate root {} is not a directory; choose an existing common parent directory",
                    canonical_root.display()
                ),
            ));
        }
        if is_git_root(&canonical_root)? {
            return Err(preflight_error(
                "aggregate_root_is_git",
                format!(
                    "aggregate root {} is a Git repository; choose its non-Git common parent instead",
                    canonical_root.display()
                ),
            ));
        }

        for candidate in candidate_paths {
            self.validate_member_path(root, &canonical_root, candidate)?;
        }

        self.reject_owned_root_files(&canonical_root, proven_entry_digests)?;
        self.reject_overlapping_manifest_root(project_id, &canonical_root)?;

        Ok(CanonicalAggregateRoot {
            canonical_path: canonical_root,
        })
    }

    fn validate_member_path(
        &self,
        supplied_root: &Path,
        canonical_root: &Path,
        candidate: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        let candidate_is_under_root =
            candidate.starts_with(supplied_root) || candidate.starts_with(canonical_root);
        let canonical_member = canonicalize_for_preflight(candidate, "member_path_missing")?;
        if canonical_member == canonical_root {
            return Err(preflight_error(
                "member_path_outside_root",
                format!(
                    "member path {} resolves to aggregate root {}; select a descendant member directory",
                    candidate.display(),
                    canonical_root.display()
                ),
            ));
        }
        if !candidate_is_under_root {
            return Err(preflight_error(
                "member_path_outside_root",
                format!(
                    "member path {} resolves to {} outside aggregate root {}; select a path below the aggregate root",
                    candidate.display(),
                    canonical_member.display(),
                    canonical_root.display()
                ),
            ));
        }
        if !canonical_member.starts_with(canonical_root) {
            return Err(preflight_error(
                "member_symlink_escape",
                format!(
                    "member path {} resolves to {} outside aggregate root {}; remove the escaping symlink or select an in-root member",
                    candidate.display(),
                    canonical_member.display(),
                    canonical_root.display()
                ),
            ));
        }
        if is_linked_worktree(&canonical_member)? {
            return Err(preflight_error(
                "nested_worktree",
                format!(
                    "member path {} resolves to linked worktree {}; select the main checkout instead",
                    candidate.display(),
                    canonical_member.display()
                ),
            ));
        }
        Ok(())
    }

    fn reject_owned_root_files(
        &self,
        canonical_root: &Path,
        proven_entry_digests: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<(), AggregateRootPreflightError> {
        for name in ["CLAUDE.md", "AGENTS.md", ".aria"] {
            let path = canonical_root.join(name);
            if !path_exists_for_preflight(&path)? {
                continue;
            }
            // 窄豁免：receipt 证明的 AGENTS/CLAUDE 必须仍是普通文件、无
            // symlink 且当前字节与旧 snapshot digest 一致；`.aria` 永不豁免。
            if name != ".aria"
                && let Some(expected_digest) =
                    proven_entry_digests.and_then(|proven| proven.get(name))
                && proven_entry_matches(&path, expected_digest)?
            {
                continue;
            }
            return Err(preflight_error(
                "aggregate_root_ownership_conflict",
                format!(
                    "aggregate root {} already contains user-owned {}; move or merge it before aggregate initialization",
                    canonical_root.display(),
                    path.display()
                ),
            ));
        }
        Ok(())
    }

    fn reject_overlapping_manifest_root(
        &self,
        project_id: &str,
        canonical_root: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        for manifest in LogicalCodebaseStore::new(self.paths.clone())
            .list_manifests()
            .map_err(|error| {
                preflight_error(
                    "aggregate_root_overlap",
                    format!(
                        "could not inspect existing logical-codebase manifests while validating {}: {error}; resolve the state-store error before retrying",
                        canonical_root.display()
                    ),
                )
            })?
        {
            if manifest.project_id == project_id {
                continue;
            }
            let existing_root = canonicalize_for_preflight(
                &manifest.provider_context_root,
                "aggregate_root_overlap",
            )?;
            if paths_overlap(canonical_root, &existing_root) {
                return Err(preflight_error(
                    "aggregate_root_overlap",
                    format!(
                        "aggregate root {} overlaps logical codebase root {} owned by project {}; choose a disjoint common parent",
                        canonical_root.display(),
                        existing_root.display(),
                        manifest.project_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn canonicalize_for_preflight(
    path: &Path,
    missing_code: &'static str,
) -> Result<PathBuf, AggregateRootPreflightError> {
    fs::canonicalize(path).map_err(|error| {
        preflight_error(
            missing_code,
            format!(
                "path {} cannot be canonicalized: {error}; choose an existing accessible path",
                path.display()
            ),
        )
    })
}

fn is_git_root(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    let git_path = path.join(".git");
    let metadata = match fs::symlink_metadata(&git_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(preflight_error(
                "aggregate_root_is_git",
                format!(
                    "could not inspect Git metadata {}: {error}; fix access and retry",
                    git_path.display()
                ),
            ));
        }
    };
    Ok(metadata.file_type().is_dir()
        || metadata.file_type().is_file()
        || metadata.file_type().is_symlink())
}

fn is_linked_worktree(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    let git_file = path.join(".git");
    let metadata = match fs::symlink_metadata(&git_file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(preflight_error(
                "nested_worktree",
                format!(
                    "could not inspect worktree metadata {}: {error}; fix access and retry",
                    git_file.display()
                ),
            ));
        }
    };
    if !metadata.file_type().is_file() {
        return Ok(false);
    }

    let contents = fs::read_to_string(&git_file).map_err(|error| {
        preflight_error(
            "nested_worktree",
            format!(
                "could not read worktree metadata {}: {error}; select a main checkout instead",
                git_file.display()
            ),
        )
    })?;
    let Some(gitdir) = contents.strip_prefix("gitdir:") else {
        return Ok(false);
    };
    let gitdir = gitdir.trim();
    if gitdir.is_empty() {
        return Err(preflight_error(
            "nested_worktree",
            format!(
                "worktree metadata {} has an empty gitdir target; select a main checkout instead",
                git_file.display()
            ),
        ));
    }

    let gitdir_path = Path::new(gitdir);
    let gitdir_path = if gitdir_path.is_absolute() {
        gitdir_path.to_path_buf()
    } else {
        path.join(gitdir_path)
    };
    let canonical_gitdir = fs::canonicalize(&gitdir_path).map_err(|error| {
            preflight_error(
                "nested_worktree",
                format!(
                    "worktree metadata {} points to inaccessible gitdir {}: {error}; select a main checkout instead",
                    git_file.display(),
                    gitdir_path.display()
                ),
            )
        })?;
    if canonical_gitdir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "worktrees")
    {
        return Ok(true);
    }
    Ok(false)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn path_exists_for_preflight(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(preflight_error(
            "aggregate_root_ownership_conflict",
            format!(
                "could not inspect existing root asset {}: {error}; fix access and retry",
                path.display()
            ),
        )),
    }
}

/// 复验一个 receipt 证明的根入口仍与旧 snapshot 摘要一致：必须是普通文件
///（symlink 一律不豁免，即使目标字节相同）、当前字节 SHA-256 与旧 digest
/// 相等。不可观测 fail-closed，绝不静默当作已证明。
fn proven_entry_matches(
    path: &Path,
    expected_digest: &str,
) -> Result<bool, AggregateRootPreflightError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        preflight_error(
            "aggregate_root_ownership_conflict",
            format!(
                "could not inspect proven root entry {}: {error}; fix access and retry",
                path.display()
            ),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(false);
    }
    let bytes = fs::read(path).map_err(|error| {
        preflight_error(
            "aggregate_root_ownership_conflict",
            format!(
                "could not read proven root entry {}: {error}; fix access and retry",
                path.display()
            ),
        )
    })?;
    let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
    Ok(digest == expected_digest)
}

fn preflight_error(code: &'static str, message: impl Into<String>) -> AggregateRootPreflightError {
    AggregateRootPreflightError {
        code,
        message: message.into(),
    }
}
