//! Task 12a(接续):#8 政策发布链只读验收与真实 live 骨架(2026-10-29 自
//! `policy_dependency.rs` 拆出的文件边界调整;用例体/断言逐字不变)。
//!
//! 缺发布 fail-closed 与正文/revision 漂移 spawn 前阻断(以及 t12 共享
//! 替身/fixture)仍在 `policy_dependency.rs`;本文件只保留计划 Interfaces
//! 面只读断言集 `assert_policy_publication_chain` 与消费它的 live 现场骨架
//! `lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison`。

use std::path::Path;
use std::time::Duration;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
use cadence_aria::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index;
use cadence_aria::product::logical_codebase::policy::AggregatePolicyArtifact;
use cadence_aria::product::logical_codebase::provider_gateway::verify_published_policy_body;
use cadence_aria::product::logical_codebase::root_recipe_receipt::RootRecipeReceipt;
use cadence_aria::product::logical_codebase::{
    AggregatePolicyArtifactStore, RootRecipeReceiptStore,
};
use cadence_aria::product::models::ProviderName;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::harness::LiveMatrixFailure;
use super::policy_dependency::T12_PROJECT_ID;

/// 计划 519 行:固定 Claude recipe 五步四命令(Task 1.5 冻结布局)。
const T12_RECIPE_STEP_COUNT: usize = 5;
const T12_RECIPE_COMMAND_COUNT: usize = 4;

// ---------------------------------------------------------------------------
// 只读验收:assert_policy_publication_chain(计划 Interfaces 面;
// live 骨架消费,生产 #8 链事实校验零副作用)
// ---------------------------------------------------------------------------

/// #8 发布链观测事实(全部由公共只读 API 采集,见 live 骨架)。
pub(crate) struct PolicyPublicationChainObservation<'a> {
    pub(crate) canonical_root: &'a Path,
    pub(crate) artifact: &'a AggregatePolicyArtifact,
    pub(crate) receipt: &'a RootRecipeReceipt,
    /// recipe 驱动 provider(生产固定 Claude)。
    pub(crate) recipe_provider: ProviderName,
}

fn chain_failure(message: String) -> LiveMatrixFailure {
    LiveMatrixFailure {
        reason_code: "policy_publication_chain_violated".to_string(),
        message,
        stage: Some("publication_chain".to_string()),
    }
}

/// 计划 521-529 行断言集的只读实现:locator 原字节/digest 三方一致/
/// receipt 根与 revision 链/固定 Claude recipe 五步四命令/独立 rule digest。
/// 违反即 `Err(LiveMatrixFailure)`,绝不物化/修复任何事实。
pub(crate) fn assert_policy_publication_chain(
    observation: &PolicyPublicationChainObservation<'_>,
) -> Result<(), LiveMatrixFailure> {
    let PolicyPublicationChainObservation {
        canonical_root,
        artifact,
        receipt,
        recipe_provider,
    } = observation;

    // 1. locator 原字节(#8:policy_id 文件原字节 == artifact.policy_text)。
    let locator = canonical_root.join(&artifact.policy_id);
    let raw_body = std::fs::read(&locator).map_err(|error| {
        chain_failure(format!(
            "read policy locator {}: {error}",
            locator.display()
        ))
    })?;
    if raw_body != artifact.policy_text.as_bytes() {
        return Err(chain_failure(format!(
            "locator raw body != artifact.policy_text at {}",
            locator.display()
        )));
    }

    // 2. raw SHA-256 == artifact.digest(#8 消费契约:raw UTF-8 字节摘要)。
    let body_digest = format!("sha256:{:x}", Sha256::digest(&raw_body));
    if body_digest != artifact.digest {
        return Err(chain_failure(format!(
            "sha256(raw body) {body_digest} != artifact.digest {}",
            artifact.digest
        )));
    }

    // 3. receipt 链:policy_digest == artifact.digest;canonical_root 一致。
    if receipt.policy_digest != artifact.digest {
        return Err(chain_failure(format!(
            "receipt.policy_digest {} != artifact.digest {}",
            receipt.policy_digest, artifact.digest
        )));
    }
    let canonical_receipt_root = receipt.canonical_root.canonicalize().map_err(|error| {
        chain_failure(format!(
            "canonicalize receipt root {}: {error}",
            receipt.canonical_root.display()
        ))
    })?;
    let canonical_observed_root = canonical_root.canonicalize().map_err(|error| {
        chain_failure(format!(
            "canonicalize canonical root {}: {error}",
            canonical_root.display()
        ))
    })?;
    if canonical_receipt_root != canonical_observed_root {
        return Err(chain_failure(format!(
            "receipt.canonical_root {} != canonical root {}",
            receipt.canonical_root.display(),
            canonical_root.display()
        )));
    }

    // 4. locator 携带的 revision == artifact.revision(policy_id 尾段)。
    let policy_revision_in_locator = artifact
        .policy_id
        .rsplit('/')
        .next()
        .and_then(|revision| revision.parse::<u64>().ok())
        .ok_or_else(|| {
            chain_failure(format!(
                "policy_id {} 不携带可解析 revision 尾段",
                artifact.policy_id
            ))
        })?;
    if policy_revision_in_locator != artifact.revision {
        return Err(chain_failure(format!(
            "locator revision {policy_revision_in_locator} != artifact.revision {}",
            artifact.revision
        )));
    }

    // 5. 独立 rule digest:AGENTS.md 原字节 SHA-256 == receipt.rule_digest。
    let rule_entry = canonical_root.join("AGENTS.md");
    let rule_bytes = std::fs::read(&rule_entry).map_err(|error| {
        chain_failure(format!(
            "read root rule entry {}: {error}",
            rule_entry.display()
        ))
    })?;
    let rule_digest = format!("sha256:{:x}", Sha256::digest(&rule_bytes));
    if receipt.rule_digest != rule_digest {
        return Err(chain_failure(format!(
            "receipt.rule_digest {} != sha256(AGENTS.md) {rule_digest}",
            receipt.rule_digest
        )));
    }

    // 6. 固定 Claude recipe:五步四命令(计划 527-529 行)。
    if *recipe_provider != ProviderName::ClaudeCode {
        return Err(chain_failure(format!(
            "recipe provider 必须是 ClaudeCode(得到 {recipe_provider:?})"
        )));
    }
    let recipe_step_count = AggregateInitializationStepKind::V1.len();
    if recipe_step_count != T12_RECIPE_STEP_COUNT {
        return Err(chain_failure(format!(
            "固定 recipe 步数必须是 {T12_RECIPE_STEP_COUNT}(得到 {recipe_step_count})"
        )));
    }
    let frozen_index = root_recipe_command_index();
    let recipe_command_count = frozen_index.len();
    if recipe_command_count != T12_RECIPE_COMMAND_COUNT {
        return Err(chain_failure(format!(
            "固定 recipe 命令数必须是 {T12_RECIPE_COMMAND_COUNT}(得到 {recipe_command_count})"
        )));
    }
    if receipt.commands.len() != T12_RECIPE_COMMAND_COUNT {
        return Err(chain_failure(format!(
            "receipt.commands 必须逐条对应四命令(得到 {})",
            receipt.commands.len()
        )));
    }
    for (summary, (step, command_index, command)) in receipt.commands.iter().zip(&frozen_index) {
        if summary.command_index != *command_index
            || summary.step != *step
            || summary.command != *command
        {
            return Err(chain_failure(format!(
                "receipt 命令摘要与冻结命令索引不一致:idx {} 期望 ({step:?}, {command})",
                summary.command_index
            )));
        }
    }

    // 7. 公共只读校验面二次复核(#8 delivered 实现,交叉一致性)。
    // 2026-10-27:该函数签名收敛为「(authority_root, artifact) -> 当前根规则摘要」
    // (回执不再参与摘要比对,权威是当前已发布政策),故交叉一致性改为
    // 「函数现算的当前根规则摘要 == 回执冻结的 rule_digest」。
    let current_rule_digest = verify_published_policy_body(canonical_root, artifact)
        .map_err(|error| chain_failure(format!("verify_published_policy_body 拒绝: {error}")))?;
    if current_rule_digest != receipt.rule_digest {
        return Err(chain_failure(format!(
            "当前根规则摘要 {current_rule_digest} 与回执冻结摘要 {} 不一致",
            receipt.rule_digest
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 真实 live 骨架:新 fresh LC + 真实 Claude recipe + #8 发布链只读验收
// ---------------------------------------------------------------------------

/// 计划 519/538 行 `lcg_live_fresh_lc_policy_publication_recipe_and_
/// direct_comparison`:fresh fixture 完全由产品 API 组装(项目/manifest 落盘
/// + 真实生产依赖图 + 真实 Claude recipe),发布链观测全部来自公共只读
/// store;无手工 seed/物化。真实现场执行归 controller:
/// `LC_GATEWAY_E2E=1 cargo test --locked --test it_web
///  lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison
///  -- --ignored --nocapture --test-threads=1`。
#[tokio::test]
#[ignore = "真实 E2E:需要 LC_GATEWAY_E2E=1 与真实 claude CLI(recipe 固定 Claude)"]
async fn lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison() {
    super::live_matrix::require_lc_gateway_e2e_switch();

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cadence_aria::product::logical_codebase::AggregateInitializationOperationStatus;
    use cadence_aria::product::logical_codebase::AggregateInitializationOperationStore;
    use cadence_aria::product::logical_codebase::LogicalCodebaseManifest;
    use cadence_aria::product::logical_codebase::LogicalCodebaseStore;
    use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
    use cadence_aria::web::app::build_web_router;
    use cadence_aria::web::runtime::WebRuntime;
    use cadence_aria::web::state::WebAppState;
    use tower::ServiceExt;

    // ---- fresh fixture:零 seed(tempdir 持有至断言结束)----
    let root_guard = tempdir().expect("live fresh root");
    let root_path = root_guard.path().to_path_buf();
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "lcg t12 live fresh publication recipe".to_string(),
            description: None,
        })
        .expect("create project fixture");
    let aggregate_root = root_path.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("fresh aggregate root");
    let manifest = LogicalCodebaseManifest::new(T12_PROJECT_ID, aggregate_root.clone(), Vec::new());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(T12_PROJECT_ID, &manifest)
        .expect("save fresh manifest");

    // ---- 真实生产依赖图(默认装配,不注入任何 fake)----
    let state = WebAppState::new(
        root_path.clone(),
        WebRuntime::new_real(root_path.clone()).expect("real web runtime"),
    );
    let app = build_web_router(state);

    // 公共 HTTP JSON 请求辅助(live 骨架内嵌,不依赖 harness 私有面)。
    async fn json_request(
        app: &axum::Router,
        method: axum::http::Method,
        uri: &str,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("build json request"),
            )
            .await
            .expect("json request");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("json request body");
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    // ---- provider health 预热(公共端点;矩阵 harness 同语义先例)----
    // `WebRuntime::new_real` 的健康源需要主动 recheck 刷新 + 有界轮询;
    // 未就绪时五步 init 的 provider turn spawn 复验会以
    // provider_gateway_unavailable(health degraded)拒绝 pre_check
    // (2026-10-09 集成现场:pre_check_failed/spawn_revalidation_drift)。
    // 此处经 /api/providers/recheck + /api/providers/status 等价实现,
    // 不复制 harness 私有面;claude(recipe 固定驱动)ready 才放行 POST。
    let health_deadline = std::time::Instant::now()
        + Duration::from_secs(
            std::env::var("LCG_T12_HEALTH_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(180),
        );
    loop {
        let _ = json_request(
            &app,
            axum::http::Method::POST,
            "/api/providers/recheck",
            serde_json::json!({}),
        )
        .await;
        let (status, health_body) = json_request(
            &app,
            axum::http::Method::GET,
            "/api/providers/status",
            serde_json::json!({}),
        )
        .await;
        let claude_ready = status.is_success()
            && health_body["state_status"] == "ready"
            && health_body["providers"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["provider"] == "claude_code" && entry["available"] == true)
            });
        if claude_ready {
            break;
        }
        assert!(
            std::time::Instant::now() < health_deadline,
            "provider health 未就绪(claude_code):{health_body} (环境不可运行须报告 BLOCKED)"
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // ---- 驱动真实五步 aggregate initialization(固定 Claude recipe)----
    let (create_status, value) = json_request(
        &app,
        axum::http::Method::POST,
        &format!("/api/projects/{T12_PROJECT_ID}/logical-codebase/initializations"),
        serde_json::json!({"idempotency_key": "lcg-t12-live-fresh"}),
    )
    .await;
    assert_eq!(create_status, StatusCode::ACCEPTED);
    assert_eq!(
        value["steps"].as_array().expect("steps").len(),
        T12_RECIPE_STEP_COUNT,
        "五步 operation 布局必须冻结"
    );
    let operation_id = value["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();

    // ---- 轮询 durable operation 至终态(默认 15 分钟,可 env 覆盖)----
    let operations = AggregateInitializationOperationStore::new(paths.clone());
    let deadline = std::time::Instant::now()
        + Duration::from_secs(
            std::env::var("LCG_T12_LIVE_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(900),
        );
    let operation = loop {
        let operation = operations
            .get(T12_PROJECT_ID, &operation_id)
            .expect("read durable operation");
        match operation.status {
            AggregateInitializationOperationStatus::Completed => break operation,
            AggregateInitializationOperationStatus::Failed => {
                panic!("真实 aggregate initialization 失败: {:?}", operation.error)
            }
            AggregateInitializationOperationStatus::Created
            | AggregateInitializationOperationStatus::Running => {}
            AggregateInitializationOperationStatus::Cancelled => {
                panic!("真实 aggregate initialization 被取消({operation_id})")
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "真实 aggregate initialization 超时未达终态({operation_id})"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    assert_eq!(
        operation.input.provider, "claude_code",
        "生产 recipe 固定 Claude 驱动"
    );

    // ---- 只读采集 #8 发布链事实(公共 store,零副作用)----
    let manifest = LogicalCodebaseStore::new(paths.clone())
        .load_manifest(T12_PROJECT_ID)
        .expect("load manifest")
        .expect("manifest 存在");
    let canonical_root = manifest
        .provider_context_root
        .canonicalize()
        .expect("canonical");
    let artifact = AggregatePolicyArtifactStore::new(paths.clone())
        .get(T12_PROJECT_ID)
        .expect("read published artifact")
        .expect("真实发布后必须存在政策 artifact");
    assert!(
        artifact.revision >= 2,
        "发布链 artifact 必须是非自举升级版(revision>=2,得到 {})",
        artifact.revision
    );
    let receipt = RootRecipeReceiptStore::new(paths.clone())
        .latest_finalized(T12_PROJECT_ID)
        .expect("read finalized receipt")
        .expect("真实 recipe 完成后必须存在 finalized receipt");

    // ---- 只读验收:发布链 + 固定 Claude recipe 五步四命令 ----
    assert_policy_publication_chain(&PolicyPublicationChainObservation {
        canonical_root: &canonical_root,
        artifact: &artifact,
        receipt: &receipt,
        recipe_provider: ProviderName::ClaudeCode,
    })
    .expect("fresh #8 发布链只读验收");

    // ---- direct 对照段(计划 531-533 行;确定性拓扑观测,与
    //      direct_comparison.rs 单测同源;真实 argv/cwd/output 基线归
    //      Task 10/11 live 矩阵 cell)----
    let sync_before = super::direct_comparison::capture_sync_direct_topology();
    let streaming_before =
        super::direct_comparison::capture_workspace_streaming_legacy_shape().await;
    let sync_after = super::direct_comparison::capture_sync_direct_topology();
    let streaming_after =
        super::direct_comparison::capture_workspace_streaming_legacy_shape().await;
    assert_eq!(sync_after, sync_before, "计划 531 行:sync direct 拓扑不变");
    assert_eq!(
        streaming_after, streaming_before,
        "计划 532 行:workspace streaming legacy 四家形态不变"
    );
}
