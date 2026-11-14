/// 聚合根登记的判定入口：project id、root 结构（存在/目录/非 Git）、成员
/// 边界（在 root 内、非 symlink 逃逸、非嵌套 worktree）与 root 与其他 LC
/// 不重叠（同项目或跨项目的其他 LC 一律拒绝，重跑时跳过 LC 自身）。
/// 根上已有的用户规则文件不参与判定。
pub struct AggregateRootPreflight {
    paths: ProductAppPaths,
    /// 判定范围所属的 LC：重叠检查跳过该 LC 自己的 manifest。`None`
    /// 表示未指明 LC，无从认定“自己”，不跳过任何 manifest。
    lc_id: Option<String>,
}

impl AggregateRootPreflight {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// 把判定范围限定到一个 LC：重叠检查跳过该 LC 自己的 manifest
    /// （登记/聚合初始化复验不得把自己拦掉），其余任何 LC——同项目或跨
    /// 项目——的 manifest 仍参与判定。
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    /// 登记入口：project id 合法性、canonical non-Git root、成员 canonical/
    /// 越界/worktree 边界、与其他 LC root 互不重叠（同项目/跨项目皆不
    /// 允许；`for_lc` 划定自身后跳过自身 manifest）。根上已存在的用户规则
    /// 文件（`AGENTS.md`/`CLAUDE.md`/`.claude/**`/`.mcp.json`/`.gitignore`）
    /// 不参与判定：做过 skills 初始化的项目必然带这些文件，把它们当门禁
    /// 会让这类项目永远无法添加 LC。
    pub fn validate(
        &self,
        project_id: &str,
        root: &Path,
        candidate_paths: &[PathBuf],
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

    /// 同一根目录不允许两个 LC（同项目或跨项目），见
    /// [`ensure_aggregate_root_unowned`]。
    fn reject_overlapping_manifest_root(
        &self,
        project_id: &str,
        canonical_root: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        ensure_aggregate_root_unowned(
            &self.paths,
            self.lc_id.as_deref().map(|lc_id| (project_id, lc_id)),
            canonical_root,
        )
    }
}

/// 聚合根唯一性的权威判定：预检与登记提交点（manifest 单写锁内）共用同一份
/// 枚举，避免两处口径分叉。`owner` 是本次判定的 LC 存储身份键
/// `(project_id, lc_id)`，命中的条目跳过（同一 LC 复验不得把自己拦掉）；
/// `None` 表示无从认定「自己」，一条都不跳过。软删（墓碑）LC 的 root 仍占用
/// ——restore 语义要求归属保留到 purge。读取/解析失败 fail-closed。
pub(crate) fn ensure_aggregate_root_unowned(
    paths: &ProductAppPaths,
    owner: Option<(&str, &str)>,
    canonical_root: &Path,
) -> Result<(), AggregateRootPreflightError> {
    for existing in existing_lc_roots(paths, canonical_root)? {
        if owner.is_some_and(|(project_id, lc_id)| {
            existing.project_id == project_id && existing.lc_id == lc_id
        }) {
            continue;
        }
        let existing_root =
            canonicalize_for_preflight(&existing.provider_context_root, "aggregate_root_overlap")?;
        if paths_overlap(canonical_root, &existing_root) {
            return Err(preflight_error(
                "aggregate_root_overlap",
                format!(
                    "aggregate root {} overlaps logical codebase {} root {} owned by project {}; choose a disjoint common parent",
                    canonical_root.display(),
                    existing.lc_id,
                    existing_root.display(),
                    existing.project_id
                ),
            ));
        }
    }
    Ok(())
}

/// 枚举所有项目的权威 LC root（legacy root ＋ `logical-codebases/*` 子树；
/// 与 legacy 同名的迁移副本跳过，legacy root 才是权威源——口径与
/// `repository_routing_policy_reader` 一致）。返回每条 root 的 LC 存储身份键
/// （legacy root 记 `legacy_logical_codebase_id`，per-LC 子树记目录名）。
/// 任何读取/解析失败都 fail-closed。
fn existing_lc_roots(
    paths: &ProductAppPaths,
    canonical_root: &Path,
) -> Result<Vec<ExistingLcManifest>, AggregateRootPreflightError> {
    let projects_root = paths.projects_root();
    let entries = match std::fs::read_dir(&projects_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(inspect_failure(
                canonical_root,
                format!("read {}: {error}", projects_root.display()),
            ));
        }
    };

    let mut manifests = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            inspect_failure(
                canonical_root,
                format!("read {} entry: {error}", projects_root.display()),
            )
        })?;
        if !entry
            .file_type()
            .map_err(|error| {
                inspect_failure(
                    canonical_root,
                    format!("stat {}: {error}", entry.path().display()),
                )
            })?
            .is_dir()
        {
            continue;
        }
        let project_id = entry.file_name().into_string().map_err(|value| {
            inspect_failure(
                canonical_root,
                format!(
                    "project directory name is not UTF-8: {}",
                    PathBuf::from(value).display()
                ),
            )
        })?;
        validate_relative_id(&project_id).map_err(|error| {
            inspect_failure(
                canonical_root,
                format!(
                    "project directory {project_id:?} is not a safe relative identifier: {error}"
                ),
            )
        })?;
        collect_project_lc_roots(paths, &project_id, canonical_root, &mut manifests)?;
    }
    Ok(manifests)
}

/// 收集一个项目的全部权威 LC root：manifest 优先；manifest 缺失的 LC
/// （冷启动 / 空成员）回退 `record.json` 的 `aggregate_root`——两份记录都
/// 表示该 LC 占着这个根。墓碑 LC 的 record 仍在 → 同样占根（restore 语义
/// 要求归属保留到 purge）。legacy root 的身份键为
/// `legacy_logical_codebase_id(project_id)`，`logical-codebases/*` 目录用目录名，
/// legacy 同名的迁移副本跳过，避免重复计数。
fn collect_project_lc_roots(
    paths: &ProductAppPaths,
    project_id: &str,
    canonical_root: &Path,
    roots: &mut Vec<ExistingLcManifest>,
) -> Result<(), AggregateRootPreflightError> {
    let legacy_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
    let mut subtree_lc_ids = Vec::new();
    let codebases_root = paths.logical_codebases_root(project_id);
    match std::fs::read_dir(&codebases_root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|error| {
                    inspect_failure(
                        canonical_root,
                        format!("read {} entry: {error}", codebases_root.display()),
                    )
                })?;
                if !entry
                    .file_type()
                    .map_err(|error| {
                        inspect_failure(
                            canonical_root,
                            format!("stat {}: {error}", entry.path().display()),
                        )
                    })?
                    .is_dir()
                {
                    continue;
                }
                let lc_id = entry.file_name().into_string().map_err(|value| {
                    inspect_failure(
                        canonical_root,
                        format!(
                            "logical-codebase directory name is not UTF-8: {}",
                            PathBuf::from(value).display()
                        ),
                    )
                })?;
                if lc_id != legacy_id {
                    subtree_lc_ids.push(lc_id);
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(inspect_failure(
                canonical_root,
                format!("read {}: {error}", codebases_root.display()),
            ));
        }
    }

    // legacy root 永远在列（`None` scope 即 legacy root）。
    let mut scopes: Vec<Option<String>> = vec![None];
    scopes.extend(subtree_lc_ids.into_iter().map(Some));
    for scope in scopes {
        let store = match scope.as_deref() {
            Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
            None => LogicalCodebaseStore::new(paths.clone()),
        };
        let lc_id = scope.unwrap_or_else(|| legacy_id.clone());
        let provider_context_root = match store
            .load_manifest(project_id)
            .map_err(|error| inspect_failure(canonical_root, format!("{error}")))?
        {
            Some(manifest) => manifest.provider_context_root,
            None => {
                // B7：没有 manifest 的 LC（冷启动 / 空成员 / 仅续跑过聚合
                // 初始化）也有 record.json 的 aggregate_root；墓碑 LC 的
                // record 仍在，一样占根。
                let Some(record) = store
                    .load_lc_record_including_tombstoned(project_id, &lc_id)
                    .map_err(|error| inspect_failure(canonical_root, format!("{error}")))?
                else {
                    continue;
                };
                record.aggregate_root
            }
        };
        roots.push(ExistingLcManifest {
            lc_id,
            project_id: project_id.to_string(),
            provider_context_root,
        });
    }
    Ok(())
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

/// 重叠枚举的一条权威 LC root 投影：LC 存储身份键、归属 project 与
/// 权威记录的 root（manifest 优先，缺失时取 `record.json` 的
/// `aggregate_root`）。
struct ExistingLcManifest {
    lc_id: String,
    project_id: String,
    provider_context_root: PathBuf,
}

fn inspect_failure(canonical_root: &Path, detail: String) -> AggregateRootPreflightError {
    preflight_error(
        "aggregate_root_overlap",
        format!(
            "could not inspect existing logical-codebase manifests while validating {}: {detail}; resolve the state-store error before retrying",
            canonical_root.display()
        ),
    )
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
    // 1.100 clippy(needless_bool)：等价化简，行为不变。
    Ok(canonical_gitdir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "worktrees"))
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn preflight_error(code: &'static str, message: impl Into<String>) -> AggregateRootPreflightError {
    AggregateRootPreflightError {
        code,
        message: message.into(),
    }
}
