/// 唯一显式 kind/authority resolver。只读：任何冲突只返回结构化
/// `ProductStoreError`，不写任何 store，也不调用 `default_logical_codebase_id`
/// 之类的 fallback。
pub struct RepositoryAuthorityResolver {
    paths: ProductAppPaths,
}

impl RepositoryAuthorityResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// C4 Task 2：issue 归属驱动的显式 authority 解析（各读取入口迁移的唯一助手）。
    ///
    /// - issue 持久化归属 LC（`IssueRecord.logical_codebase_id = Some`）→ 显式
    ///   `LogicalCodebase` 请求，kind/身份/重复来源/legacy 布局冲突全部由
    ///   `resolve` fail-closed 校验。
    /// - issue 无归属但 legacy 别名 LC record 存在（`ProjectStore::get/list`
    ///   幂等 `migrate_legacy` 的产物）→ 显式解析 legacy 别名 LC，其子树与旧
    ///   project 级布局字节等价；不猜“最新可用记录”。
    /// - 两者皆无（单仓 issue、未迁移旧数据）→ `Ok(None)`：调用方保留
    ///   single-repo/legacy 兼容路径，但不得把物理仓伪装为 LC target。
    pub fn resolve_for_issue(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<RepositoryAuthorityResolution>, ProductStoreError> {
        // 与 `resolve_issue_logical_codebase_id` 同语义的宽容读取：issue 记录
        // 不存在时不伪造归属（legacy 兼容：旧数据/裸 fixture 只带 issue 字符串
        // id），其余读取错误原样传播。
        let issue_record = crate::product::issue_store::IssueStore::new(self.paths.clone())
            .get(project_id, issue_id);
        let (issue_exists, attributed) = match issue_record {
            Ok(issue) => (true, issue.logical_codebase_id),
            Err(ProductStoreError::NotFound { .. }) => (false, None),
            Err(error) => return Err(error),
        };
        // 项目记录存在时先幂等触发 `migrate_legacy`（`ProjectStore::get` 内
        // 置，与所有既有入口一致）：legacy 布局存在则别名 LC record 必在。
        // 无项目记录的裸 fixture/旧数据保持 `None` 兼容分支，不伪造 LC。
        match crate::product::project_store::ProjectStore::new(self.paths.clone()).get(project_id) {
            Ok(_) => {}
            Err(ProductStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error),
        }
        let alias_record_exists = self
            .paths
            .logical_codebase_record_root(
                project_id,
                &crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id),
            )
            .join("record.json")
            .try_exists()
            .map_err(|error| ProductStoreError::Io(format!("try_exists alias record: {error}")))?;
        let lc_id = attributed.or_else(|| {
            alias_record_exists.then(|| {
                crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id)
            })
        });
        let Some(lc_id) = lc_id else {
            return Ok(None);
        };
        // `resolve` 要求携带的 issue 必须存在（Task 1 语义）；issue 记录缺失的
        // legacy 数据不带 issue_id 解析，随后从同一 LC 子树宽容补读 selection，
        // 保持 `load_for_issue` 时代 (manifest, selection) 成对判定的兼容性。
        let mut resolution = self.resolve(RepositoryRoutingRequest {
            project_id: project_id.to_string(),
            issue_id: issue_exists.then(|| issue_id.to_string()),
            kind: RepositoryTargetKind::LogicalCodebase,
            repository_id: None,
            logical_codebase_id: Some(lc_id.clone()),
            logical_repository_id: None,
            checkout_id: None,
        })?;
        if !issue_exists && resolution.selection.is_none() {
            resolution.selection =
                crate::product::logical_codebase::IssueCodebaseSelectionStore::for_lc(
                    self.paths.clone(),
                    &lc_id,
                )
                .load(project_id, issue_id)?;
        }
        Ok(Some(resolution))
    }

    pub fn resolve(
        &self,
        request: RepositoryRoutingRequest,
    ) -> Result<RepositoryAuthorityResolution, ProductStoreError> {
        validate_relative_id(&request.project_id)?;
        crate::product::project_store::ProjectStore::new(self.paths.clone())
            .get(&request.project_id)?;

        // issue attribution：请求携带 issue 时必须存在，且其持久化的
        // codebase 归属与请求 kind 一致；不符一律 kind_mismatch。
        let mut attributed_lc: Option<String> = None;
        if let Some(issue_id) = request.issue_id.as_deref() {
            let issue = crate::product::issue_store::IssueStore::new(self.paths.clone())
                .get(&request.project_id, issue_id)?;
            attributed_lc = issue.logical_codebase_id.clone();
        }

        match request.kind {
            RepositoryTargetKind::LogicalCodebase => {
                self.resolve_logical(&request, attributed_lc.as_deref())
            }
            RepositoryTargetKind::SingleRepo => {
                self.resolve_single_repo(&request, attributed_lc.as_deref())
            }
        }
    }

    fn resolve_logical(
        &self,
        request: &RepositoryRoutingRequest,
        attributed_lc: Option<&str>,
    ) -> Result<RepositoryAuthorityResolution, ProductStoreError> {
        let Some(lc_id) = request.logical_codebase_id.as_deref() else {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!("logical_codebase_id:missing:issue:{:?}", request.issue_id),
            });
        };
        validate_relative_id(lc_id)?;
        if request.repository_id.is_some() {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "repository_id:{}:logical_codebase:{lc_id}",
                    request.repository_id.clone().unwrap_or_default()
                ),
            });
        }
        if let Some(attributed) = attributed_lc
            && attributed != lc_id
        {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "issue:{}:attributed_logical_codebase:{attributed}:requested:{lc_id}",
                    request.issue_id.clone().unwrap_or_default()
                ),
            });
        }

        // LC record 直读（不经 `migrate_legacy`，resolver 保持零写入）。
        let record_root = self
            .paths
            .logical_codebase_record_root(&request.project_id, lc_id);
        let record_path = record_root.join("record.json");
        let record_exists = record_path.try_exists().map_err(|error| {
            ProductStoreError::Io(format!("try_exists {}: {error}", record_path.display()))
        })? && !record_root.join("tombstone.json").exists();
        if !record_exists {
            return Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                id: lc_id.to_string(),
            });
        }
        let record: crate::product::logical_codebase::store::LogicalCodebaseRecord =
            crate::product::json_store::read_json(&record_path)?;

        let logical = LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id);
        let manifest = logical.load_manifest(&request.project_id)?;
        // authority root 与 `SessionPolicyEnvelope.authority_root` 同义：LC 的
        // 聚合政策权威根 = manifest.provider_context_root（冷启动无 manifest 时
        // 取 LC record 的 aggregate_root）。manifest/record 均来自请求 LC 子树，
        // 误读他 LC 立即产生不同 root。目录尚不存在（冷启动 LC）时保留原始
        // 路径——与 `LogicalCodebaseGatewayFactory` 的构造语义一致；spawn 前
        // 复验由 provider admission 负责，不在此 fail-closed。
        let authority_root_path = manifest
            .as_ref()
            .map(|manifest| manifest.provider_context_root.clone())
            .unwrap_or_else(|| record.aggregate_root.clone());
        let authority_root =
            std::fs::canonicalize(&authority_root_path).unwrap_or(authority_root_path);
        let members = logical.list_members(&request.project_id)?;
        let checkouts = logical.list_checkouts(&request.project_id)?;

        // 同一 git-dir/来源两个别名成员：重复候选 fail-closed。
        let mut seen_sources = std::collections::BTreeMap::new();
        for member in &members {
            if member.status != crate::product::logical_codebase::types::MemberStatus::Active {
                continue;
            }
            let digest = member.source_identity.key_digest.clone();
            if seen_sources
                .insert(digest.clone(), member.logical_repository_id)
                .is_some()
            {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_source_identity_mismatch",
                    id: digest,
                });
            }
        }

        // 旧 project-level 布局与新 LC 子树来源身份冲突：不猜、不回退。
        let legacy_root = self.paths.logical_codebase_root(&request.project_id);
        let legacy_manifest_exists = legacy_root.join("manifest.json").exists();
        if legacy_manifest_exists
            && lc_id
                != crate::product::logical_codebase::store::legacy_logical_codebase_id(
                    &request.project_id,
                )
        {
            let legacy_members =
                LogicalCodebaseStore::new(self.paths.clone()).list_members(&request.project_id)?;
            let requested_sources: std::collections::BTreeSet<&str> = members
                .iter()
                .filter(|member| {
                    member.status == crate::product::logical_codebase::types::MemberStatus::Active
                })
                .map(|member| member.source_identity.key_digest.as_str())
                .collect();
            if let Some(conflict) = legacy_members.iter().find(|member| {
                member.status == crate::product::logical_codebase::types::MemberStatus::Active
                    && requested_sources.contains(member.source_identity.key_digest.as_str())
            }) {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_legacy_conflict",
                    id: conflict.source_identity.key_digest.clone(),
                });
            }
        }

        let (canonical_path, source_identity_digest, member_id, checkout_id) =
            resolve_logical_target(
                request,
                &manifest,
                &members,
                &checkouts,
                &record.aggregate_root,
            )?;

        let selection = match request.issue_id.as_deref() {
            Some(issue_id) => IssueCodebaseSelectionStore::for_lc(self.paths.clone(), lc_id)
                .load(&request.project_id, issue_id)?,
            None => None,
        };

        let policy = AggregatePolicyArtifactStore::for_lc(self.paths.clone(), lc_id)
            .get(&request.project_id)?
            .map(|artifact| {
                if let Some(manifest) = manifest.as_ref()
                    && artifact.logical_codebase_id != manifest.logical_codebase_id.to_string()
                {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "repository_routing",
                        reason: format!(
                            "repository_routing_inconsistent: policy artifact {} does not belong to manifest {}",
                            artifact.logical_codebase_id, manifest.logical_codebase_id
                        ),
                    });
                }
                Ok(AuthorityPolicyReference {
                    policy_id: artifact.policy_id,
                    policy_revision: artifact.revision,
                    policy_digest: artifact.digest,
                    artifact_root: authority_root.clone(),
                })
            })
            .transpose()?;

        let aggregate_index =
            read_authority_aggregate_index(&self.paths, &request.project_id, lc_id)?;

        Ok(RepositoryAuthorityResolution {
            authority_root,
            target: ResolvedTargetIdentity {
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc_id.to_string()),
                logical_repository_id: member_id,
                checkout_id,
                canonical_path,
                source_identity_digest,
            },
            manifest,
            selection,
            policy,
            aggregate_index,
        })
    }

    fn resolve_single_repo(
        &self,
        request: &RepositoryRoutingRequest,
        attributed_lc: Option<&str>,
    ) -> Result<RepositoryAuthorityResolution, ProductStoreError> {
        let Some(repository_id) = request.repository_id.as_deref() else {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!("repository_id:missing:issue:{:?}", request.issue_id),
            });
        };
        if request.logical_codebase_id.is_some()
            || request.logical_repository_id.is_some()
            || request.checkout_id.is_some()
        {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!("repository_id:{repository_id}:logical_target_present"),
            });
        }
        if let Some(attributed) = attributed_lc {
            return Err(ProductStoreError::Conflict {
                kind: "repository_routing_kind_mismatch",
                id: format!(
                    "issue:{}:attributed_logical_codebase:{attributed}:repository:{repository_id}",
                    request.issue_id.clone().unwrap_or_default()
                ),
            });
        }

        let repository_store =
            crate::product::repository_store::RepositoryStore::new(self.paths.clone());
        let records = repository_store.list(&request.project_id)?;
        let record = records
            .iter()
            .find(|record| record.id == repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: repository_id.to_string(),
            })?;
        let canonical_path =
            crate::product::repository_store::canonicalize_repo_path(&record.path)?;

        // 同一 git-dir 两个别名：其余物理记录解析到同一 canonical path 即冲突。
        for other in &records {
            if other.id == repository_id {
                continue;
            }
            if let Ok(other_canonical) =
                crate::product::repository_store::canonicalize_repo_path(&other.path)
                && other_canonical == canonical_path
            {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_source_identity_mismatch",
                    id: format!("{repository_id}:{}", other.id),
                });
            }
        }

        let source = crate::product::repository_store::resolve_repository_source(&canonical_path)?;

        // 旧 project-level 布局与物理仓来源身份冲突。
        let legacy_root = self.paths.logical_codebase_root(&request.project_id);
        if legacy_root.join("manifest.json").exists() {
            let legacy_members =
                LogicalCodebaseStore::new(self.paths.clone()).list_members(&request.project_id)?;
            if let Some(conflict) = legacy_members.iter().find(|member| {
                member.status == crate::product::logical_codebase::types::MemberStatus::Active
                    && member.source_identity.key_digest == source.key_digest
            }) {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_routing_legacy_conflict",
                    id: conflict.source_identity.key_digest.clone(),
                });
            }
        }

        Ok(RepositoryAuthorityResolution {
            authority_root: canonical_path.clone(),
            target: ResolvedTargetIdentity {
                kind: RepositoryTargetKind::SingleRepo,
                repository_id: Some(repository_id.to_string()),
                logical_codebase_id: None,
                logical_repository_id: None,
                checkout_id: None,
                canonical_path,
                source_identity_digest: source.key_digest,
            },
            manifest: None,
            selection: None,
            policy: None,
            aggregate_index: AuthorityAggregateIndexReference {
                aggregate_index_id: None,
                membership_revision: None,
                status: None,
            },
        })
    }
}

/// 解析 LC kind 的 canonical path/source digest/member/checkout 身份。
/// 显式 member/checkout 必须在请求 LC 子树内命中，否则 target unknown。
fn resolve_logical_target(
    request: &RepositoryRoutingRequest,
    manifest: &Option<LogicalCodebaseManifest>,
    members: &[crate::product::logical_codebase::types::CodebaseMemberRecord],
    checkouts: &[crate::product::logical_codebase::types::RepositoryCheckoutRecord],
    record_root: &PathBuf,
) -> Result<
    (
        PathBuf,
        String,
        Option<LogicalRepositoryId>,
        Option<RepositoryCheckoutId>,
    ),
    ProductStoreError,
> {
    match request.logical_repository_id {
        Some(member_id) => {
            let member = members
                .iter()
                .find(|member| member.logical_repository_id == member_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_repository",
                    id: member_id.0.to_string(),
                })?;
            let checkout = match request.checkout_id {
                Some(checkout_id) => {
                    if !member.checkout_ids.contains(&checkout_id) {
                        return Err(ProductStoreError::NotFound {
                            kind: "repository_checkout",
                            id: checkout_id.0.to_string(),
                        });
                    }
                    checkouts
                        .iter()
                        .find(|checkout| checkout.checkout_id == checkout_id)
                        .ok_or_else(|| ProductStoreError::NotFound {
                            kind: "repository_checkout",
                            id: checkout_id.0.to_string(),
                        })?
                }
                None => checkouts
                    .iter()
                    .find(|checkout| {
                        member.checkout_ids.contains(&checkout.checkout_id)
                            && checkout.kind
                                == crate::product::logical_codebase::types::CheckoutKind::Main
                    })
                    .ok_or_else(|| ProductStoreError::NotFound {
                        kind: "repository_checkout",
                        id: member
                            .checkout_ids
                            .first()
                            .map(|id| id.0.to_string())
                            .unwrap_or_default(),
                    })?,
            };
            Ok((
                checkout.canonical_path.clone(),
                member.source_identity.key_digest.clone(),
                Some(member_id),
                Some(checkout.checkout_id),
            ))
        }
        None => {
            // 冷启动（无 manifest）LC：聚合根回退到 record.aggregate_root，
            // 使 bootstrap/成员等 GET 投影在 manifest 尚未生成时仍可解析身份。
            let root = manifest
                .as_ref()
                .map(|manifest| manifest.provider_context_root.clone())
                .unwrap_or_else(|| record_root.clone());
            // 聚合 source digest：root + 排序后的成员 source digests。
            let mut digests: Vec<&str> = members
                .iter()
                .filter(|member| {
                    member.status == crate::product::logical_codebase::types::MemberStatus::Active
                })
                .map(|member| member.source_identity.key_digest.as_str())
                .collect();
            digests.sort_unstable();
            let mut payload = root.to_string_lossy().into_owned();
            for digest in digests {
                payload.push('\0');
                payload.push_str(digest);
            }
            let source_identity_digest =
                format!("sha256:{:x}", sha2::Sha256::digest(payload.as_bytes()));
            Ok((root, source_identity_digest, None, None))
        }
    }
}

/// 只读读取 LC 子树的 aggregate index 引用：优先 active；无 active 时
/// 保留最新一代非 superseded 记录的状态事实（如 Failed）；全空返回 None 组合。
fn read_authority_aggregate_index(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> Result<AuthorityAggregateIndexReference, ProductStoreError> {
    let store = AggregateIndexStore::for_lc(paths.clone(), lc_id);
    let map_error = |error: AggregateIndexError| -> ProductStoreError {
        ProductStoreError::InvalidRecord {
            kind: "aggregate_index",
            reason: error.to_string(),
        }
    };
    if let Some(active) = store.active(project_id).map_err(map_error)? {
        return Ok(AuthorityAggregateIndexReference {
            aggregate_index_id: Some(active.aggregate_index_id),
            membership_revision: Some(active.membership_revision),
            status: Some(AggregateIndexStatus::Active),
        });
    }
    let mut latest: Option<
        crate::product::logical_codebase::aggregate_index::AggregateIndexRecord,
    > = None;
    for record in store.records(project_id).map_err(map_error)? {
        if record.status == AggregateIndexStatus::Superseded {
            continue;
        }
        if latest
            .as_ref()
            .is_none_or(|current| record.updated_at > current.updated_at)
        {
            latest = Some(record);
        }
    }
    Ok(match latest {
        None => AuthorityAggregateIndexReference {
            aggregate_index_id: None,
            membership_revision: None,
            status: None,
        },
        Some(record) => AuthorityAggregateIndexReference {
            aggregate_index_id: Some(record.aggregate_index_id),
            membership_revision: Some(record.membership_revision),
            status: Some(record.status),
        },
    })
}
