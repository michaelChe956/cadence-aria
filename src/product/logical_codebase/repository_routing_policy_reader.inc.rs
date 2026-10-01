/// C2 Task 10（REQ-ENV-C2-POLICY）：受限政策读取错误。resolver 无法唯一
/// 解析／身份或 digest 不一致一律 fail-closed——调用方（evidence mediator
/// 应用服务）落"政策核验"等待事实，MUST NOT 回落到成员仓路径、项目级
/// 历史布局或绝对路径猜测。
#[derive(Debug, thiserror::Error)]
pub enum PolicyReadError {
    /// policy_id 结构不可解析，或引用指向的 LC 子树无 policy artifact
    /// （resolver 不可用口径）。
    #[error("policy_reference_unavailable:{detail}")]
    Unavailable { detail: String },
    /// policy_id／revision／authority root 与 artifact 身份不一致（引用被
    /// 串改或指向错误子树）。
    #[error("policy_identity_mismatch:{detail}")]
    IdentityMismatch { detail: String },
    /// 正文 canonical SHA-256 与引用 digest 不一致（政策已升级，冻结引用
    /// 过期）。
    #[error("policy_digest_mismatch:expected:{expected}:actual:{actual}")]
    DigestMismatch { expected: String, actual: String },
    /// 底层 durable store 读失败。
    #[error("policy_read_store_error:{0}")]
    Store(#[from] ProductStoreError),
}

/// C2 Task 10：受限政策读取结果——与引用同 digest 的政策正文＋三元引用；
/// 不携带宿主绝对路径。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicyTextResult {
    pub policy_id: String,
    pub policy_revision: u64,
    pub policy_digest: String,
    pub text: String,
}

/// C2 Task 10：以 `resolve_for_issue` 产出的 `AuthorityPolicyReference` 为
/// 输入读取同 digest 政策正文。
///
/// `policy_id` 形如 `policy/{project_id}/{manifest 身份}/{revision}`
/// （`AggregatePolicyArtifact` 构造契约；内嵌的是 manifest 身份而非子树
/// 目录键），故解析出 project_id 后在项目全部权威 LC 子树（legacy root＋
/// `logical-codebases/*`）中按 policy_id 恰匹配检索——恰一个匹配才继续，
/// 零个 Unavailable、多个 IdentityMismatch，不做任何路径猜测；随后
/// revision／authority root／digest 逐项复核，任一不一致 fail-closed。
/// 底层 `get` 已复核 digest 是正文 canonical SHA-256。
pub fn read_policy_text_for_reference(
    paths: &ProductAppPaths,
    reference: &AuthorityPolicyReference,
) -> Result<PolicyTextResult, PolicyReadError> {
    let segments: Vec<&str> = reference.policy_id.split('/').collect();
    if segments.len() != 4 || segments[0] != "policy" || segments.iter().any(|s| s.is_empty()) {
        return Err(PolicyReadError::Unavailable {
            detail: format!("malformed policy_id: {}", reference.policy_id),
        });
    }
    let project_id = segments[1];
    validate_relative_id(project_id)?;

    // 候选权威子树：legacy root（`None` scope）＋该项目全部
    // `logical-codebases/{id}` 子树；与 legacy 同名的目录即 legacy root 本身，
    // 跳过避免重复计数。
    let legacy_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
    let mut scopes: Vec<Option<String>> = vec![None];
    if let Ok(entries) = std::fs::read_dir(paths.logical_codebases_root(project_id)) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name != legacy_id
            {
                scopes.push(Some(name.to_string()));
            }
        }
    }

    let mut matched: Option<(Option<String>, AggregatePolicyArtifact)> = None;
    for scope in scopes {
        let policy_store = match scope.as_deref() {
            Some(lc_id) => AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id),
            None => AggregatePolicyArtifactStore::new(paths.clone()),
        };
        let Some(artifact) = policy_store.get(project_id)? else {
            continue;
        };
        if artifact.policy_id != reference.policy_id {
            continue;
        }
        if matched.is_some() {
            return Err(PolicyReadError::IdentityMismatch {
                detail: format!(
                    "policy_id {} matches multiple authority subtrees",
                    reference.policy_id
                ),
            });
        }
        matched = Some((scope, artifact));
    }
    let Some((scope, artifact)) = matched else {
        return Err(PolicyReadError::Unavailable {
            detail: format!("no authority subtree holds policy {}", reference.policy_id),
        });
    };

    if artifact.revision != reference.policy_revision {
        return Err(PolicyReadError::IdentityMismatch {
            detail: format!(
                "reference revision {} does not match artifact revision {}",
                reference.policy_revision, artifact.revision
            ),
        });
    }
    // authority root 复核：artifact 必须来自引用冻结时的同一权威根（manifest
    // 优先，冷启动回退 LC record.aggregate_root，与 `resolve_logical` 同口径）。
    let logical = match scope.as_deref() {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    let manifest = logical.load_manifest(project_id)?;
    let record_root =
        paths.logical_codebase_record_root(project_id, scope.as_deref().unwrap_or(&legacy_id));
    let record: crate::product::logical_codebase::store::LogicalCodebaseRecord =
        crate::product::json_store::read_json(&record_root.join("record.json"))?;
    let expected_root = manifest
        .as_ref()
        .map(|manifest| manifest.provider_context_root.clone())
        .unwrap_or_else(|| record.aggregate_root.clone());
    let expected_root = std::fs::canonicalize(&expected_root).unwrap_or(expected_root);
    if expected_root != reference.artifact_root {
        return Err(PolicyReadError::IdentityMismatch {
            detail: format!(
                "authority root {} does not match reference {}",
                expected_root.display(),
                reference.artifact_root.display()
            ),
        });
    }

    // digest 一致性：引用冻结 digest 必须等于 artifact 正文 digest（get 已
    // 校验 digest 是正文 canonical SHA-256）。
    if artifact.digest != reference.policy_digest {
        return Err(PolicyReadError::DigestMismatch {
            expected: reference.policy_digest.clone(),
            actual: artifact.digest.clone(),
        });
    }
    Ok(PolicyTextResult {
        policy_id: artifact.policy_id,
        policy_revision: artifact.revision,
        policy_digest: artifact.digest,
        text: artifact.policy_text,
    })
}
