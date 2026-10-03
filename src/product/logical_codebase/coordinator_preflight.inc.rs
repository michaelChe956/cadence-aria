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

    /// Task 3（aggregate-policy-root-publication，REQ-BOOT-06）：producer-only
    /// 重跑预检的 receipt 证据枚举。仅枚举本 scope（lc_id 缺省时为 legacy
    /// project 布局）旧 Completed operations，读取对应最终 receipt 与
    /// `list_commands`，核验四条固定索引 Allowed、身份/root、最终 summaries
    /// 与各 command snapshot 引用、AGENTS 的最终 rule digest/末条 after
    /// digest/当前字节三方一致；CLAUDE 在场则需同末条 snapshot 独立摘要。
    /// 证据选择只接受全部成立的一组（不拼接不同旧 operation、不读取其他
    /// scope）；无任何成立证据返回 `None`，调用方保持原 strict validate。
    /// store 材料解析错误按现有 store 失败关闭（可重试）。
    fn proven_recipe_entry_digests(
        &self,
        project_id: &str,
        canonical_root: &std::path::Path,
    ) -> Result<Option<std::collections::BTreeMap<String, String>>, AggregateInitializationError>
    {
        use crate::product::logical_codebase as lc;

        let store_failure = |error: crate::product::json_store::ProductStoreError| {
            AggregateInitializationError::Preflight {
                reason: format!(
                    "recipe replay evidence for root {} could not be read: {error}",
                    canonical_root.display()
                ),
                retryable: true,
            }
        };
        let (operations, receipts) = match &self.lc_id {
            Some(lc_id) => (
                AggregateInitializationOperationStore::for_lc(self.paths.clone(), lc_id.clone()),
                lc::RootRecipeReceiptStore::for_lc(self.paths.clone(), lc_id.clone()),
            ),
            None => (
                AggregateInitializationOperationStore::new(self.paths.clone()),
                lc::RootRecipeReceiptStore::new(self.paths.clone()),
            ),
        };
        for operation in operations.list(project_id).map_err(store_failure)? {
            if operation.status != AggregateInitializationOperationStatus::Completed {
                continue;
            }
            match receipts.get(project_id, &operation.operation_id) {
                // 该 operation 无最终 receipt：证据不成立，继续尝试更旧的
                // Completed operation（不拼接、不借用其他 scope）。
                Ok(None) => continue,
                Ok(Some(receipt)) => {
                    if let Some(proven) = proven_digests_from_receipt(
                        &receipts,
                        project_id,
                        &receipt,
                        canonical_root,
                        store_failure,
                    )? {
                        return Ok(Some(proven));
                    }
                }
                // store 材料读取/解析错误与 list_commands 一致：经
                // store_failure 可重试失败关闭，绝不静默当作无证据。
                Err(error) => return Err(store_failure(error)),
            }
        }
        Ok(None)
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
        // Task 3（REQ-BOOT-06）：producer-only 重跑预检——本 scope 有完整
        // receipt 证明时走窄复验入口（只豁免复验通过的 AGENTS/CLAUDE），
        // 其余一切（含 `.aria`）与无证明场景保持公开 strict validate。
        let root_preflight = AggregateRootPreflight::new(self.paths.clone());
        let root_validation = match self.proven_recipe_entry_digests(project_id, &canonical_root)? {
            Some(proven_entry_digests) => root_preflight.validate_recipe_replay(
                project_id,
                &manifest.provider_context_root,
                &candidate_paths,
                &proven_entry_digests,
            ),
            None => root_preflight.validate(
                project_id,
                &manifest.provider_context_root,
                &candidate_paths,
            ),
        };
        root_validation.map_err(|error| AggregateInitializationError::Preflight {
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

/// 核验一个旧 Completed operation 的最终 receipt 是否构成当前
/// `canonical_root` 的完整归属证明；成立时返回 AGENTS（与在场 CLAUDE）
/// 各自旧 snapshot 原字节 digest 的窄 map，任一环节不成立返回 `None`
/// （该 operation 的证据整体不采用，绝不拼接）。store 材料解析错误
/// fail-closed（经 `store_failure` 转可重试 preflight 错误）。
fn proven_digests_from_receipt<F>(
    receipts: &crate::product::logical_codebase::RootRecipeReceiptStore,
    project_id: &str,
    receipt: &crate::product::logical_codebase::RootRecipeReceipt,
    canonical_root: &std::path::Path,
    store_failure: F,
) -> Result<Option<std::collections::BTreeMap<String, String>>, AggregateInitializationError>
where
    F: Fn(crate::product::json_store::ProductStoreError) -> AggregateInitializationError,
{
    use crate::product::logical_codebase as lc;

    // 身份/root：最终 receipt 必须证明本次重跑的 canonical root。
    if receipt.canonical_root != canonical_root {
        return Ok(None);
    }
    // 四条固定索引的命令摘要全部 Allowed 且身份一致。
    let index = lc::root_recipe_command_index();
    if receipt.commands.len() != index.len() {
        return Ok(None);
    }
    for (summary, (step, command_index, command)) in receipt.commands.iter().zip(index.iter()) {
        if summary.command_index != *command_index
            || summary.step != *step
            || summary.command != *command
            || summary.verdict != lc::RootRecipeCommandVerdict::Allowed
        {
            return Ok(None);
        }
    }
    // 命令级 receipts：四条在场、身份/Allowed/root 一致，最终 summaries
    // 与各 command 前后 snapshot 引用一致。
    let operation_id = receipt.operation_id.as_str();
    let commands = receipts
        .list_commands(project_id, operation_id)
        .map_err(&store_failure)?;
    if commands.len() != index.len() {
        return Ok(None);
    }
    let mut final_command = None;
    for (summary, (step, command_index, command)) in receipt.commands.iter().zip(index.iter()) {
        let Some(command_receipt) = commands
            .iter()
            .find(|candidate| candidate.command_index == *command_index)
        else {
            return Ok(None);
        };
        if command_receipt.operation_id != operation_id
            || command_receipt.step != *step
            || command_receipt.command != *command
            || command_receipt.canonical_root != canonical_root
            || command_receipt.verdict != lc::RootRecipeCommandVerdict::Allowed
            || summary.before_snapshot_digest != command_receipt.before_snapshot.snapshot_digest
            || summary.after_snapshot_digest != command_receipt.after_snapshot.snapshot_digest
        {
            return Ok(None);
        }
        final_command = Some(command_receipt);
    }
    let final_command = final_command.expect("the frozen index always yields the final command");

    // AGENTS 三方一致：最终 rule digest == 末条 after snapshot 摘要 == 当前字节。
    let Some(agents_entry) = final_command
        .after_snapshot
        .entries
        .iter()
        .find(|entry| entry.path == "AGENTS.md")
    else {
        return Ok(None);
    };
    if agents_entry.kind != lc::RootRecipeSnapshotEntryKind::File {
        return Ok(None);
    }
    let Some(agents_digest) = agents_entry.content_digest.as_deref() else {
        return Ok(None);
    };
    if receipt.rule_digest != agents_digest {
        return Ok(None);
    }
    let Some(current_rule_digest) =
        lc::root_recipe_receipt::root_rule_digest(canonical_root).map_err(&store_failure)?
    else {
        return Ok(None);
    };
    if current_rule_digest != agents_digest {
        return Ok(None);
    }
    let mut proven =
        std::collections::BTreeMap::from([("AGENTS.md".to_string(), agents_digest.to_string())]);

    // CLAUDE 在场才需要同末条 snapshot 的独立摘要并复验当前字节；
    // 缺失不要求生成（recipe 不代用户造文件）。
    let claude_path = canonical_root.join("CLAUDE.md");
    match std::fs::symlink_metadata(&claude_path) {
        Ok(_) => {
            let Some(claude_entry) = final_command
                .after_snapshot
                .entries
                .iter()
                .find(|entry| entry.path == "CLAUDE.md")
            else {
                // 在场但旧 snapshot 无对应常规文件：未证明的新文件，冲突。
                return Ok(None);
            };
            if claude_entry.kind != lc::RootRecipeSnapshotEntryKind::File {
                return Ok(None);
            }
            let Some(claude_digest) = claude_entry.content_digest.as_deref() else {
                return Ok(None);
            };
            let claude_bytes = std::fs::read(&claude_path).map_err(|error| {
                store_failure(crate::product::json_store::ProductStoreError::Io(format!(
                    "read proven CLAUDE.md {}: {error}",
                    claude_path.display()
                )))
            })?;
            let current_claude_digest = format!("sha256:{:x}", Sha256::digest(&claude_bytes));
            if current_claude_digest != claude_digest {
                return Ok(None);
            }
            proven.insert("CLAUDE.md".to_string(), claude_digest.to_string());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(store_failure(
                crate::product::json_store::ProductStoreError::Io(format!(
                    "inspect proven CLAUDE.md {}: {error}",
                    claude_path.display()
                )),
            ));
        }
    }
    Ok(Some(proven))
}
