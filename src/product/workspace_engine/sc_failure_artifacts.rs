// P11：SC 失败修复包的**严格 publisher**（Plan §2.5）。
//
// `publish_sc_failure_repair_package` 把纯组包器（同目录子模块
// `sc_failure_artifact_builder`）产出的包按真实证据落盘并发布：真实 lifecycle +
// durable SC 会话作用域 + 活动节点 → 证据重建逐字段相等 → 同锁同步写版本表 →
// 节点 artifact_ref 写入并读回 → 发布回执写入并过发布屏障 → 广播 ArtifactUpdate。
//
// 不启动 provider、不调用旧 update_artifact、不写 no-op 引用；任何一步失败都不
// 广播成功帧。纯组包器在本文件之外（子模块），本文件只保留发布面实现与用例挂载。

use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceRead, sc_failure_split_locator,
};
use crate::product::lifecycle_store::sc_failure_publication::{ScLocalTerminal, ScTerminalInput};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::ArtifactRef;
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ScArtifactPublishedReceipt, ScEvidenceRef,
    ScFailureChainIdentity, ScFailureClass, ScFailureRepairPackage, ScFailureScope,
};

use super::{EngineEvent, WorkspaceEngine};

/// 纯组包器子模块（同目录独立文件）：只经 `pub(super)` 内部使用，不反向导出。
#[path = "sc_failure_artifact_builder.rs"]
mod sc_failure_artifact_builder;

use self::sc_failure_artifact_builder::{
    CODE_PACKAGE_INVALID, CODE_PACKAGE_SERIALIZE, CODE_SCOPE_MISMATCH,
    build_sc_failure_repair_package, sha256_hex,
};


// ── P11 严格 publisher（Plan §2.5）────────────────────────────────────
//
// 顺序即持久屏障：真实证据核验 → 同锁同步写版本表 → 节点 artifact_ref 写入并读回
// → 发布回执写入并过发布屏障 → 广播 ArtifactUpdate。
// 不启动 provider、不调用旧 update_artifact、不写 no-op 引用：任何一步失败都不
// 广播成功帧。

const PUBLISH_STEP: &str = "publish_sc_failure_repair_package";
const CODE_LIFECYCLE_MISSING: &str = "sc_failure_lifecycle_missing";
const CODE_ACTIVE_NODE_MISSING: &str = "sc_failure_active_node_missing";
const CODE_PACKAGE_MISMATCH: &str = "sc_failure_package_mismatch";
const CODE_PACKAGE_ID_CONFLICT: &str = "sc_failure_package_id_conflict";
const CODE_EVIDENCE_BODY_MISMATCH: &str = "sc_failure_evidence_body_mismatch";
const CODE_EVIDENCE_INVALID: &str = "sc_failure_evidence_invalid";
const CODE_VERSION_WRITE_FAILED: &str = "sc_failure_version_write_failed";
const CODE_NODE_REF_FAILED: &str = "sc_failure_node_ref_failed";
const CODE_RECEIPT_FAILED: &str = "sc_failure_receipt_failed";
const CODE_BROADCAST_FAILED: &str = "sc_failure_broadcast_failed";
const REASON_PACKAGE_ID_CONFLICT: &str = "sc_failure_package_id_conflict";

impl WorkspaceEngine {
    /// 严格发布 SC 失败修复包（A4）。
    ///
    /// 1. 真实 lifecycle + durable SC 会话作用域 + 活动节点（无未知节点兜底）；
    /// 2. 包自校验 + 真实证据重建逐字段相等（调用方载荷不可信）；
    /// 3. 证据正文核验：raw 逐字读回、教学提示逐字读回、结构 source 走既有
    ///    source store 读回；编译/IR 的归一化 source 是**调用方前置**（P19d 用
    ///    P20 resolver 在交包前核过），product 不重算归一化；
    /// 4. 同锁同步写 artifact_versions.json：同包重放复用同一 version/created_at，
    ///    异文同 id 拒绝；新版本 max+1 且 is_current=false（不动普通 current）；
    /// 5. 节点 artifact_ref 写入 + 读回同版本；
    /// 6. 发布回执写入 + 发布屏障核验；
    /// 7. 广播 ArtifactUpdate（只在回执核验之后）。
    pub(crate) async fn publish_sc_failure_repair_package(
        &mut self,
        package: ScFailureRepairPackage,
    ) -> Result<ArtifactRef, ScFailureEvidenceError> {
        let store = self.lifecycle_store.clone().ok_or_else(|| {
            publish_error(
                CODE_LIFECYCLE_MISSING,
                PUBLISH_STEP,
                None,
                "engine 没有持久 lifecycle store，不能发布 SC 失败修复包",
                false,
            )
        })?;
        let node_id = self.active_node_id.clone().ok_or_else(|| {
            publish_error(
                CODE_ACTIVE_NODE_MISSING,
                PUBLISH_STEP,
                None,
                "没有活动节点，拒绝发布 SC 失败修复包（不做未知节点兜底）",
                false,
            )
        })?;
        if self.session.session_id != package.session_id
            || self.session.project_id != package.project_id
            || self.session.issue_id != package.issue_id
            || self.session.entity_id != package.plan_id
        {
            return Err(publish_error(
                CODE_SCOPE_MISMATCH,
                PUBLISH_STEP,
                None,
                "会话作用域与包作用域不一致",
                false,
            ));
        }
        package.validate().map_err(|detail| {
            publish_error(
                CODE_PACKAGE_INVALID,
                PUBLISH_STEP,
                None,
                format!("包自校验失败（§2.2 不变量）: {detail}"),
                false,
            )
        })?;

        let scope = ScFailureScope {
            session_id: package.session_id.clone(),
            project_id: package.project_id.clone(),
            issue_id: package.issue_id.clone(),
            plan_id: package.plan_id.clone(),
        };

        // 真实证据：strict read → 纯组包器重建 → 与调用方给的包逐字段相等。
        let (identity, rebuilt) = match store
            .read_sc_failure_chain_for_publication(&scope, &package.chain_id)
        {
            Ok((identity, evidence)) => {
                let rebuilt =
                    build_sc_failure_repair_package(&identity, &evidence, ScTerminalInput::Durable)?;
                (identity, rebuilt)
            }
            Err(failure) => {
                // 腐坏日志：只有真实已验证前缀能组真实 Incomplete 包；没有前缀就
                // 原样上浮，不伪造包、不伪造 durable 终局。
                let Some(prefix) = failure.verified_prefix.as_deref() else {
                    return Err(publish_error(
                        &failure.code,
                        PUBLISH_STEP,
                        failure.round_id.clone(),
                        format!("读取真实证据失败: {}", failure.detail),
                        failure.publication_incomplete,
                    ));
                };
                let prefix = prefix.clone();
                let identity = prefix_chain_identity(&prefix, &scope, &package.chain_id)?;
                let local = local_terminal_from_package(&package);
                let rebuilt =
                    build_sc_failure_repair_package(&identity, &prefix, ScTerminalInput::Local(&local))?;
                (identity, rebuilt)
            }
        };
        if rebuilt != package {
            return Err(publish_error(
                CODE_PACKAGE_MISMATCH,
                PUBLISH_STEP,
                None,
                "调用方给的包与真实证据重建结果不一致，拒绝发布",
                false,
            ));
        }
        verify_sc_failure_package_bodies(&store, &identity, &scope, &package)?;

        let digest = sc_failure_package_digest(&package)?;
        let session_id = package.session_id.clone();
        let author_provider = self.session.author_provider.clone();
        let package_for_write = package.clone();
        let node_for_write = node_id.clone();

        // 同锁同步闭环：查包 → 复用或 max+1 → merged save → 字节回读核验。
        let (published_row, merged) = store
            .with_artifact_versions_write(&session_id, |ctx| {
                for entry in ctx.raw() {
                    let ArtifactPayload::ScFailureRepairPackage {
                        sc_failure_repair_package,
                    } = &entry.payload
                    else {
                        continue;
                    };
                    if sc_failure_repair_package.package_id != package_for_write.package_id {
                        continue;
                    }
                    let stored_digest =
                        sc_failure_package_digest(sc_failure_repair_package).map_err(|failure| {
                            ProductStoreError::InvalidRecord {
                                kind: "artifact_versions",
                                reason: failure.detail,
                            }
                        })?;
                    if stored_digest != digest {
                        return Err(ProductStoreError::InvalidRecord {
                            kind: "artifact_versions",
                            reason: REASON_PACKAGE_ID_CONFLICT.to_string(),
                        });
                    }
                    return Ok((entry.clone(), ctx.raw().to_vec()));
                }
                let next = ctx.next_version()?;
                let proposed_row = ArtifactVersion {
                    version: next,
                    payload: ArtifactPayload::ScFailureRepairPackage {
                        sc_failure_repair_package: Box::new(package_for_write.clone()),
                    },
                    generated_by: author_provider,
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: false,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    source_node_id: node_for_write.clone(),
                };
                let mut proposed = ctx.raw().to_vec();
                proposed.push(proposed_row);
                let merged = ctx.save(&proposed)?;
                let bytes = ctx.read_bytes()?;
                let expected = serde_json::to_vec_pretty(&merged).map_err(|failure| {
                    ProductStoreError::Json(format!("序列化合并表失败: {failure}"))
                })?;
                if bytes != expected {
                    return Err(ProductStoreError::Json(
                        "落盘字节与合并表序列化结果不一致".to_string(),
                    ));
                }
                let parsed: Vec<ArtifactVersion> = serde_json::from_slice(&bytes).map_err(|failure| {
                    ProductStoreError::Json(format!("回读解析失败: {failure}"))
                })?;
                if parsed != merged {
                    return Err(ProductStoreError::Json("回读版本表与合并表不一致".to_string()));
                }
                let row = merged
                    .iter()
                    .find(|entry| entry.version == next)
                    .cloned()
                    .ok_or_else(|| {
                        ProductStoreError::Json("合并表缺少新版本行".to_string())
                    })?;
                let ArtifactPayload::ScFailureRepairPackage {
                    sc_failure_repair_package,
                } = &row.payload
                else {
                    return Err(ProductStoreError::Json(
                        "新版本行不是 SC 失败修复包载荷".to_string(),
                    ));
                };
                if sc_failure_repair_package.as_ref() != &package_for_write {
                    return Err(ProductStoreError::Json(
                        "落盘后的包成员与发布包不一致".to_string(),
                    ));
                }
                Ok((row, merged))
            })
            .map_err(|failure| match failure {
                ProductStoreError::InvalidRecord { reason, .. }
                    if reason == REASON_PACKAGE_ID_CONFLICT =>
                {
                    publish_error(
                        CODE_PACKAGE_ID_CONFLICT,
                        PUBLISH_STEP,
                        None,
                        "同一 package_id 已有异文版本，拒绝覆盖",
                        false,
                    )
                }
                other => publish_error(
                    CODE_VERSION_WRITE_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("写入 artifact 版本失败: {other}"),
                    false,
                ),
            })?;

        // 节点 artifact_ref：写入后读回同一版本（无未核验兜底、不写 no-op 引用）。
        // artifact_id 沿用既有 `artifact_version_{version:03}` 约定（与 store 的发布
        // 屏障核验同口径）；package_id 只活在载荷与回执里，不是 artifact 身份。
        let artifact_ref = ArtifactRef {
            artifact_id: format!("artifact_version_{:03}", published_row.version),
            version: published_row.version,
        };
        self.persist_artifact_ref(&node_id, artifact_ref.clone())
            .await
            .map_err(|detail| {
                publish_error(
                    CODE_NODE_REF_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("节点 artifact_ref 落盘失败: {detail}"),
                    true,
                )
            })?;
        let detail = store
            .load_node_detail(&session_id, &node_id)
            .map_err(|failure| {
                publish_error(
                    CODE_NODE_REF_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("节点详情读回失败: {failure}"),
                    true,
                )
            })?;
        if detail.artifact_ref.as_ref() != Some(&artifact_ref) {
            return Err(publish_error(
                CODE_NODE_REF_FAILED,
                PUBLISH_STEP,
                None,
                "节点 artifact_ref 读回与写入不一致",
                true,
            ));
        }

        // 发布回执：写入 + 发布屏障核验（缺回执/摘要不符/节点指向别的版本都 Err）。
        let receipt = ScArtifactPublishedReceipt {
            schema_version: 1,
            package_id: package.package_id.clone(),
            chain_id: package.chain_id.clone(),
            node_id: node_id.clone(),
            package_digest: digest.clone(),
            version: published_row.version,
        };
        store
            .put_sc_artifact_published_receipt(&identity, &receipt)
            .map_err(|failure| {
                publish_error(
                    CODE_RECEIPT_FAILED,
                    PUBLISH_STEP,
                    failure.round_id.clone(),
                    format!("发布回执落盘失败: {}", failure.detail),
                    true,
                )
            })?;
        store
            .validate_sc_artifact_publication(&scope, &published_row)
            .map_err(|failure| {
                publish_error(
                    CODE_RECEIPT_FAILED,
                    PUBLISH_STEP,
                    failure.round_id.clone(),
                    format!("发布屏障核验失败: {}", failure.detail),
                    true,
                )
            })?;

        // 持久屏障齐了才同步引擎内存版本表并广播（普通 current 版本原样保留，
        // session.artifact 不动）。
        self.artifact_versions = merged;
        self.event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: published_row.version,
                payload: ArtifactPayload::ScFailureRepairPackage {
                    sc_failure_repair_package: Box::new(package),
                },
            })
            .await
            .map_err(|failure| {
                publish_error(
                    CODE_BROADCAST_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("广播 ArtifactUpdate 失败: {failure}"),
                    false,
                )
            })?;
        Ok(artifact_ref)
    }
}

/// 已验证前缀的链身份：取首条**真实**条目的 chain（受控首行），并核作用域与
/// chain_id 与请求一致。
fn prefix_chain_identity(
    prefix: &ScFailureEvidenceRead,
    scope: &ScFailureScope,
    chain_id: &str,
) -> Result<ScFailureChainIdentity, ScFailureEvidenceError> {
    let Some(entry) = prefix.entries.first() else {
        return Err(publish_error(
            CODE_EVIDENCE_INVALID,
            PUBLISH_STEP,
            None,
            "已验证前缀里没有任何真实条目，无法确认链身份",
            true,
        ));
    };
    let identity = entry.chain.clone();
    if &identity.scope != scope || identity.chain_id != chain_id {
        return Err(publish_error(
            CODE_SCOPE_MISMATCH,
            PUBLISH_STEP,
            None,
            "已验证前缀的链身份与请求作用域/chain_id 不一致",
            true,
        ));
    }
    Ok(identity)
}

/// Local 终局事实：取包自身的终局字段与结构化 missing（不伪造 durable 终局）。
fn local_terminal_from_package(package: &ScFailureRepairPackage) -> ScLocalTerminal {
    let last_physical_round_id = package
        .rounds
        .iter()
        .find(|round| round.raw == package.last_provider_output)
        .map(|round| round.identity.round_id.clone());
    ScLocalTerminal {
        last_failure_round_id: package.last_failure_source.source_round_id.clone(),
        last_physical_round_id,
        terminal_class: package.terminal_class.clone(),
        terminal_reason: package.terminal_reason.clone(),
        launch_observations: package
            .rounds
            .iter()
            .map(|round| (round.identity.round_id.clone(), round.launch_state.clone()))
            .collect(),
        completeness: package.completeness.clone(),
        missing_evidence: package.missing_evidence.clone(),
    }
}

/// 证据正文核验：raw 走真实 split 输出读回，教学提示走提示正文读回；结构类
/// source 按 typed `failure_class` 走既有 source store 读回（不按字符串猜域）。
///
/// 编译/IR 的归一化 source 不在这里重算：product 不反向 import handler 的
/// resolver、也不复制归一化算法——它的正确性是调用方前置（P19d 交包前用 P20
/// resolver 核过）。任何读不出来的 source 都如实报错（`sc_failure_evidence_invalid`），
/// 绝不伪造 Available，也绝不悄悄改写载荷。
fn verify_sc_failure_package_bodies(
    store: &LifecycleStore,
    identity: &ScFailureChainIdentity,
    scope: &ScFailureScope,
    package: &ScFailureRepairPackage,
) -> Result<(), ScFailureEvidenceError> {
    for round in &package.rounds {
        if let ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } = &round.raw
        {
            let expected_locator = sc_failure_split_locator(
                &round.identity.workspace_session_id,
                round.identity.role_run_seq,
            );
            if locator != &expected_locator {
                return Err(publish_error(
                    CODE_EVIDENCE_BODY_MISMATCH,
                    PUBLISH_STEP,
                    Some(round.identity.round_id.clone()),
                    format!("raw locator {locator} 与该轮真实 split 输出定位符不一致"),
                    true,
                ));
            }
            let body = store.read_sc_split_output(scope, &round.identity)?;
            verify_sc_failure_body(
                locator,
                body.as_bytes(),
                sha256,
                *bytes,
                &round.identity.round_id,
            )?;
        }
        if let ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } = &round.source
            && round.failure_class == Some(ScFailureClass::StructureGate)
        {
            let source_store = WorkItemPlanSourceStore::new(store.app_paths().clone());
            let source_scope = SourceStoreScope {
                project_id: scope.project_id.clone(),
                issue_id: scope.issue_id.clone(),
                plan_id: scope.plan_id.clone(),
            };
            let record = source_store
                .get_source_revision(&source_scope, locator)
                .map_err(|failure| {
                    publish_error(
                        CODE_EVIDENCE_INVALID,
                        PUBLISH_STEP,
                        Some(round.identity.round_id.clone()),
                        format!("结构 source 读回失败: {failure:?}"),
                        true,
                    )
                })?;
            verify_sc_failure_body(
                locator,
                record.source.as_bytes(),
                sha256,
                *bytes,
                &round.identity.round_id,
            )?;
        }
    }
    for prompt in &package.teaching_prompts {
        if let ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } = &prompt.prompt
        {
            let body = store.read_sc_teaching_prompt(identity, locator)?;
            verify_sc_failure_body(
                locator,
                body.as_bytes(),
                sha256,
                *bytes,
                &prompt.failed_round_id,
            )?;
        }
    }
    Ok(())
}

fn verify_sc_failure_body(
    locator: &str,
    body: &[u8],
    expected_sha256: &str,
    expected_bytes: u64,
    round_id: &str,
) -> Result<(), ScFailureEvidenceError> {
    if sha256_hex(body) != expected_sha256 || body.len() as u64 != expected_bytes {
        return Err(publish_error(
            CODE_EVIDENCE_BODY_MISMATCH,
            PUBLISH_STEP,
            Some(round_id.to_string()),
            format!("证据正文与引用不符（locator={locator}）"),
            true,
        ));
    }
    Ok(())
}

/// 发布用摘要：整包紧凑序列化的 SHA-256（含 package_id，不含写入时间）。
fn sc_failure_package_digest(
    package: &ScFailureRepairPackage,
) -> Result<String, ScFailureEvidenceError> {
    let bytes = serde_json::to_vec(package).map_err(|failure| {
        publish_error(
            CODE_PACKAGE_SERIALIZE,
            PUBLISH_STEP,
            None,
            format!("包序列化失败: {failure}"),
            false,
        )
    })?;
    Ok(sha256_hex(&bytes))
}

/// publisher 专用错误构造（可带 publication_incomplete）。
fn publish_error(
    code: &str,
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
    publication_incomplete: bool,
) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: step.to_string(),
        round_id,
        detail: detail.into(),
        publication_incomplete,
        verified_prefix: None,
    }
}

/// P11 严格 publisher 的共享真实 fixture（真实 store / 真实会话 / 真实证据日志）。
/// 单一实例，供发布面用例与后续覆盖用例共用，不重复装配。
#[cfg(test)]
#[path = "tests/sc_failure_publication_fixture.rs"]
mod publication_fixture;

/// P11 严格 publisher 行为用例（真实 store / 真实 engine / 真实文件故障）。
#[cfg(test)]
#[path = "tests/sc_failure_publication.rs"]
mod publication_tests;

/// P16 消费者侧公开视图用例（发布后从 durable 状态重建 → 公开 session_state；
/// pending / 坏回执不外泄）。独立文件承载容量片，与 publisher 用例共用同一份
/// 真实 fixture（super::publication_fixture）与 engine 助手。
#[cfg(test)]
#[path = "tests/sc_failure_publication_rebuild.rs"]
mod publication_rebuild_tests;
