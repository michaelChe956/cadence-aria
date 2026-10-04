//! Web 层 `LogicalCodebaseGatewayFactory`:为指定 project 组装 gateway。

use std::path::PathBuf;
use std::sync::Arc;

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseProviderGateway,
    LogicalCodebaseStore, ProductionPolicyTargetResolver, ProviderCapabilityProbeService,
    ProviderCapabilityStore, ProviderGatewayError, StoreBackedProviderCapabilitySource,
};

/// 为指定 project 组装 `LogicalCodebaseProviderGateway` 的 Web 层工厂。
///
/// 持有持久化路径、provider registry、同步 adapter、availability gate 与共享启动
/// 审计。`build` 会先 bootstrap policy 与 capability,再用 `with_audit` 组装 gateway,
/// 使同一 factory 构造出的多个 gateway 共享同一份 `GatewayRunAudit`。
pub struct LogicalCodebaseGatewayFactory {
    paths: ProductAppPaths,
    registry: Arc<ProviderRegistry>,
    sync_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
    availability_gate: Arc<ProviderAvailabilityGate>,
    audit: Arc<GatewayRunAudit>,
}

impl LogicalCodebaseGatewayFactory {
    pub fn new(
        paths: ProductAppPaths,
        registry: Arc<ProviderRegistry>,
        sync_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
        availability_gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            paths,
            registry,
            sync_adapter,
            availability_gate,
            audit: Arc::new(GatewayRunAudit::new()),
        }
    }

    /// Task 1c-factory:LC 生产组装——gateway 的同步槽注入 LC gateway sync
    /// bridge(`GatewaySyncProvider`,streaming→sync validated 桥)。LC 同步
    /// 栈从此只接受 prepared validated launch,raw `run` 由 bridge
    /// fail-closed;单仓 direct 的裸 `CliProviderAdapter` 两槽 routing 不经
    /// 本工厂,保持原契约。不扩大 normal admission 写权限(material prep 与
    /// readonly 的区分仍由 `build_for_lc_material_prep`/`build_readonly_for_lc`
    /// 承担)。
    pub fn with_lc_sync_bridge(
        paths: ProductAppPaths,
        registry: Arc<ProviderRegistry>,
        availability_gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self::new(
            paths,
            registry.clone(),
            Arc::new(
                crate::cross_cutting::gateway_sync_provider::GatewaySyncProvider::new(registry),
            ),
            availability_gate,
        )
    }

    pub fn audit(&self) -> Arc<GatewayRunAudit> {
        self.audit.clone()
    }

    /// 为指定 project 构造 gateway(兼容别名):等价
    /// `build_for_lc_material_prep(project_id, None)`。normal admission 调用方
    /// 应迁移 `build_readonly_for_lc`(Task 3b)。
    pub fn build(
        &self,
        project_id: &str,
    ) -> Result<LogicalCodebaseProviderGateway, ProviderGatewayError> {
        self.build_for_lc_material_prep(project_id, None)
    }

    /// v1.3 legacy 兼容别名:等价 `build_for_lc_material_prep`。#8/recipe
    /// material prep 之外的调用方(normal admission/GET/early)必须迁移
    /// `build_readonly_for_lc`,禁止借用 material prep 的写入语义。
    pub fn build_for_lc(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<LogicalCodebaseProviderGateway, ProviderGatewayError> {
        self.build_for_lc_material_prep(project_id, lc_id)
    }

    /// Task 3b:**只读组装**——early/GET/action eligibility 读取面专用。
    /// 不调用 `ensure_bootstrap`、不写 policy/capability/trust/audit;缺
    /// #8 发布材料时由 admission/check 返回可操作 waiting,而不是在读取
    /// 路径上静默物化自举桩。
    pub fn build_readonly_for_lc(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<LogicalCodebaseProviderGateway, ProviderGatewayError> {
        self.build_scoped(project_id, lc_id, false)
    }

    /// Task 3b:**material prep 组装**——仅供 #8/recipe material prep 的
    /// 显式调用(`ensure_bootstrap` 写 recipe 材料),normal admission
    /// 不能借用。
    pub fn build_for_lc_material_prep(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<LogicalCodebaseProviderGateway, ProviderGatewayError> {
        self.build_scoped(project_id, lc_id, true)
    }

    /// v1.3:按 issue 所属代码库构造 gateway——lc_id Some 时 policy/
    /// capability/manifest 全部解析到 `logical-codebases/{lc_id}/` 子树
    /// (C4 Task 2 legacy 别名统一走 `for_lc`)。Task 3b:`ensure_stores`
    /// 区分只读组装与 material prep(写 ensure_bootstrap)。
    fn build_scoped(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
        ensure_stores: bool,
    ) -> Result<LogicalCodebaseProviderGateway, ProviderGatewayError> {
        let lc_id = self.resolve_lc_scope(project_id, lc_id)?;
        let logical = match lc_id.as_deref() {
            Some(lc_id) => LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id),
            None => LogicalCodebaseStore::new(self.paths.clone()),
        };
        let manifest = logical
            .load_manifest(project_id)?
            .ok_or_else(|| ProviderGatewayError::PolicyMissing(project_id.to_string()))?;

        let policies = match lc_id.as_deref() {
            Some(lc_id) => AggregatePolicyArtifactStore::for_lc(self.paths.clone(), lc_id),
            None => AggregatePolicyArtifactStore::new(self.paths.clone()),
        };
        if ensure_stores {
            // material prep(#8/recipe):允许写 recipe 材料;readonly 组装
            // 绝不在读取路径上物化自举桩。
            policies.ensure_bootstrap(&manifest)?;
        }

        let capabilities = match lc_id.as_deref() {
            Some(lc_id) => ProviderCapabilityStore::for_lc(self.paths.clone(), lc_id),
            None => ProviderCapabilityStore::new(self.paths.clone()),
        };
        if ensure_stores {
            capabilities.ensure_bootstrap(project_id)?;
        }

        // 权威根 = manifest.provider_context_root(聚合根 cwd)。若为相对路径,
        // 构造时 canonicalize;失败回退原值(生产 manifest 应已指向存在目录)。
        let authority_root = std::fs::canonicalize(&manifest.provider_context_root)
            .unwrap_or_else(|_| manifest.provider_context_root.clone());

        // Task 2.8（映射 tasks.md 2.2；REQ-ENV-10 双工厂 root assertion）：
        // manifest authority root 与登记工厂（record.aggregate_root）、
        // aggregate 生产 driver（root recipe receipt 冻结的 canonical root）
        // 的投影 canonical 一致才组装 gateway；任一不一致 fail-closed
        // （zero spawn——gateway 不产出，后续所有启动为零）。投影缺失
        // （bootstrap 早期/legacy 无 receipt/纯单仓无 LC 作用域）不视为
        // 不一致；envelope cwd 投影 = manifest root（LC 会话 cwd 契约）。
        let registration_root = self.load_registration_root(project_id, lc_id.as_deref())?;
        let aggregate_root = self.load_aggregate_driver_root(project_id, lc_id.as_deref())?;
        if registration_root.is_some() || aggregate_root.is_some() {
            crate::product::logical_codebase::assert_canonical_lc_root_consistent(
                registration_root.as_deref(),
                aggregate_root.as_deref(),
                &authority_root,
                &manifest.provider_context_root,
            )?;
        }

        Ok(LogicalCodebaseProviderGateway::with_audit(
            policies,
            // Task 1c-factory(carry③):LC 作用域用 `for_lc` 构造 capability
            // source——同时建立 root-recipe 凭据的 durable Running 重核验
            // 通道(capability store 与 operation store 同一 lc 子树);无
            // LC 作用域(legacy)保持 `with_store` 原语义。
            Arc::new(match lc_id.as_deref() {
                Some(lc_id) => StoreBackedProviderCapabilitySource::for_lc(
                    self.paths.clone(),
                    project_id.to_string(),
                    lc_id.to_string(),
                ),
                None => {
                    StoreBackedProviderCapabilitySource::with_store(capabilities, project_id.to_string())
                }
            }),
            Arc::new(match lc_id.as_deref() {
                // R9 fix round 1【Important-1】：resolver 同步按 lc_id 作用域解析 checkout
                // 目标，否则非 legacy 新 LC 的 coding session 启动会 fail-closed。
                Some(lc_id) => ProductionPolicyTargetResolver::for_lc(self.paths.clone(), lc_id),
                None => ProductionPolicyTargetResolver::new(self.paths.clone()),
            }),
            self.registry.clone(),
            self.sync_adapter.clone(),
            self.availability_gate.clone(),
            self.audit.clone(),
            authority_root,
        )
        .with_readonly_lc_facts(
            match lc_id.as_deref() {
                Some(lc_id) => {
                    crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                        self.paths.clone(),
                        lc_id,
                    )
                }
                None => crate::product::logical_codebase::RootRecipeReceiptStore::new(
                    self.paths.clone(),
                ),
            },
            Arc::new(
                crate::product::logical_codebase::provider_trust::ReadonlyProviderTrustSource::production_for_scope(
                    self.paths.clone(),
                    lc_id.clone(),
                ),
            ),
            lc_id.clone(),
        ))
    }

    /// Task 2d:durable probe writer 注入(owner 门 §0.3)。按 `build_for_lc`
    /// 同一 LC 作用域解析构造 `ProviderCapabilityProbeService::with_durable_
    /// writer`,使「已通过三方一致性校验的 evidence 导入 durable
    /// Confirmed」写入的子树与 gateway 读取的 capability store 完全同源。
    /// 导入通道本身不重新 probe、不 spawn(provider spawn count 恒 0);
    /// Task 3 admission 消费本通道。
    pub fn durable_probe_writer_for_lc(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<ProviderCapabilityProbeService, ProviderGatewayError> {
        let lc_id = self.resolve_lc_scope(project_id, lc_id)?;
        let capabilities = match lc_id.as_deref() {
            Some(lc_id) => ProviderCapabilityStore::for_lc(self.paths.clone(), lc_id),
            None => ProviderCapabilityStore::new(self.paths.clone()),
        };
        Ok(ProviderCapabilityProbeService::with_durable_writer(
            capabilities,
        ))
    }

    /// 解析 LC 作用域:显式 `lc_id` 原样使用;`None`(legacy 别名)解析
    /// alias record 存在性——存在则用别名 id,否则保持 project 级路径
    /// (C4 Task 2 语义,与 `build_for_lc` 历史行为逐字节等价)。
    fn resolve_lc_scope(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<Option<String>, ProviderGatewayError> {
        match lc_id {
            Some(lc_id) => Ok(Some(lc_id.to_string())),
            None => {
                let alias_id =
                    crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
                let alias_record_exists = self
                    .paths
                    .logical_codebase_record_root(project_id, &alias_id)
                    .join("record.json")
                    .try_exists()
                    .map_err(|error| {
                        ProviderGatewayError::PolicyMissing(format!(
                            "{project_id}: read alias record: {error}"
                        ))
                    })?;
                Ok(alias_record_exists.then_some(alias_id))
            }
        }
    }
}

impl LogicalCodebaseGatewayFactory {
    /// 登记工厂的 root 投影：LC 作用域内 `record.json` 的
    /// `aggregate_root`（登记时冻结）。record 缺失/无 LC 作用域 → `Ok(None)`
    /// （不视为不一致）；record 在场但不可读/损坏 → fail-closed（不把损坏
    /// 投影当缺失放行）。
    fn load_registration_root(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<Option<PathBuf>, ProviderGatewayError> {
        let Some(lc_id) = lc_id else {
            return Ok(None);
        };
        let record_path = self
            .paths
            .logical_codebase_record_root(project_id, lc_id)
            .join("record.json");
        if !record_path.try_exists().map_err(|error| {
            ProviderGatewayError::PolicyMissing(format!(
                "{project_id}: read registration record existence: {error}"
            ))
        })? {
            return Ok(None);
        }
        let record = crate::product::json_store::read_json::<
            crate::product::logical_codebase::store::LogicalCodebaseRecord,
        >(&record_path)
        .map_err(ProviderGatewayError::policy)?;
        Ok(Some(record.aggregate_root))
    }

    /// aggregate 生产 driver 的 root 投影：LC 作用域内全部已 finalize 的
    /// root recipe receipt 冻结的 `canonical_root`（源自 aggregate
    /// preflight snapshot root）。receipt 缺失/无 LC 作用域 → `Ok(None)`；
    /// 在场 receipt 不可读/损坏，或多个 receipt 根彼此不一致（LC 换根残留）
    /// → fail-closed。
    fn load_aggregate_driver_root(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<Option<PathBuf>, ProviderGatewayError> {
        let Some(lc_id) = lc_id else {
            return Ok(None);
        };
        let scope = crate::product::logical_codebase::store::lc_scope_root(
            &self.paths,
            project_id,
            &Some(lc_id.to_string()),
        )
        .map_err(ProviderGatewayError::policy)?;
        let receipts_dir = scope.join("aggregate-recipe-receipts");
        if !receipts_dir.try_exists().map_err(|error| {
            ProviderGatewayError::PolicyMissing(format!(
                "{project_id}: read aggregate receipts existence: {error}"
            ))
        })? {
            return Ok(None);
        }
        let entries = std::fs::read_dir(&receipts_dir).map_err(|error| {
            ProviderGatewayError::Target(format!(
                "read aggregate receipts {}: {error}",
                receipts_dir.display()
            ))
        })?;
        let mut roots: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProviderGatewayError::Target(format!("read aggregate receipt entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let receipt: crate::product::logical_codebase::RootRecipeReceipt =
                crate::product::json_store::read_json(&path)
                    .map_err(ProviderGatewayError::policy)?;
            if !roots.contains(&receipt.canonical_root) {
                roots.push(receipt.canonical_root);
            }
        }
        match roots.len() {
            0 => Ok(None),
            1 => Ok(roots.into_iter().next()),
            _ => Err(ProviderGatewayError::TargetMismatch {
                field: "aggregate_root".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use chrono::Utc;
    use tempfile::tempdir;

    use crate::cross_cutting::provider_adapter::ProviderAdapter;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::streaming_provider::{
        ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalCodebaseManifest, LogicalCodebaseStore, PolicyTarget, ProviderGatewayError,
        ProviderRef, SessionLaunchRequest,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

    struct StubSyncAdapter;

    impl ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            Ok(AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    struct NoopStreamingAdapter;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for NoopStreamingAdapter {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(ProviderSession {
                events,
                commands,
                native_session_id: None,
            })
        }
    }

    fn fake_registry() -> Arc<ProviderRegistry> {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, Arc::new(NoopStreamingAdapter));
        Arc::new(registry)
    }

    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }

            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
            snapshot,
        ))))
    }

    fn factory_fixture() -> (
        tempfile::TempDir,
        ProductAppPaths,
        Arc<LogicalCodebaseGatewayFactory>,
    ) {
        let root = tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let factory = Arc::new(LogicalCodebaseGatewayFactory::new(
            paths.clone(),
            fake_registry(),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
        ));
        (root, paths, factory)
    }

    fn register_manifest(paths: &ProductAppPaths, project_id: &str) {
        let manifest = LogicalCodebaseManifest::new(project_id, paths.root().to_path_buf(), vec![]);
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(project_id, &manifest)
            .expect("save manifest");
    }

    #[test]
    fn build_assembles_gateway_and_shares_audit_instance() {
        let (_root, paths, factory) = factory_fixture();
        register_manifest(&paths, "project_0001");

        let gateway = factory.build("project_0001").unwrap();

        let audit = factory.audit();
        assert!(Arc::ptr_eq(&audit, &gateway.audit()));

        let aggregate_root = paths.root().join("aggregate");
        std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
        let request = SessionLaunchRequest::planning(
            "project_0001",
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::aggregate_root(aggregate_root),
            vec![paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = gateway.validate(request).unwrap();
        assert_eq!(validated.envelope().policy_revision, 1);
    }

    #[test]
    fn build_returns_policy_missing_for_unregistered_project() {
        let (_root, _paths, factory) = factory_fixture();

        let error = match factory.build("project_missing") {
            Err(error) => error,
            Ok(_) => panic!("expected PolicyMissing, got a gateway"),
        };

        assert!(matches!(
            error,
            ProviderGatewayError::PolicyMissing(ref id) if id == "project_missing"
        ));
    }

    #[test]
    fn build_for_lc_scopes_policy_and_capability_to_lc_subtree() {
        let (_root, paths, factory) = factory_fixture();
        let lc_id = "lc_second";

        // 仅在 lc 子树注册 manifest（不写 project 级 legacy 路径）。
        let manifest =
            LogicalCodebaseManifest::new("project_0001", paths.root().to_path_buf(), vec![]);
        LogicalCodebaseStore::for_lc(paths.clone(), lc_id)
            .save_manifest("project_0001", &manifest)
            .expect("save lc manifest");

        // 按 lc_id 构建成功（policy/capability 解析到 lc 子树）。
        let gateway = factory
            .build_for_lc("project_0001", Some(lc_id))
            .expect("build lc gateway");

        // legacy（None）路径无 manifest → PolicyMissing，证明未串扰到 project 级。
        let legacy = factory.build_for_lc("project_0001", None);
        assert!(matches!(
            legacy,
            Err(ProviderGatewayError::PolicyMissing(ref id)) if id == "project_0001"
        ));

        // policy/capability 落盘在 lc 子树，而非 legacy project 级。
        let lc_root = paths.logical_codebases_root("project_0001").join(lc_id);
        assert!(lc_root.join("manifest.json").is_file());
        assert!(lc_root.join("aggregate-policy.json").is_file());
        assert!(lc_root.join("capabilities.json").is_file());
        let legacy_root = paths.logical_codebase_root("project_0001");
        assert!(!legacy_root.join("manifest.json").exists());
        assert!(!legacy_root.join("aggregate-policy.json").exists());
        assert!(!legacy_root.join("capabilities.json").exists());

        // gateway 可用：validate 通过并回读 lc 子树 policy revision。
        let aggregate_root = paths.root().join("aggregate");
        std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
        let request = SessionLaunchRequest::planning(
            "project_0001",
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::aggregate_root(aggregate_root),
            vec![paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = gateway.validate(request).unwrap();
        assert_eq!(validated.envelope().policy_revision, 1);
    }

    /// Task 1c-factory(lcg_t01,carry③):`build_for_lc` 的 LC 作用域组装注入
    /// root-recipe 凭据重核验通道(`StoreBackedProviderCapabilitySource::for_lc`)
    /// ——root recipe 相位的 gateway 校验不再 fail-closed 于
    /// `root_recipe_credential_recheck_unavailable`,而是真实消费 durable 凭据
    /// 重核验(无 Running operation → denied)。
    #[test]
    fn lcg_t01_build_for_lc_wires_root_recipe_credential_recheck_channel() {
        let (_root, paths, factory) = factory_fixture();
        let lc_id = "lc_recipe_recheck";

        let manifest =
            LogicalCodebaseManifest::new("project_0001", paths.root().to_path_buf(), vec![]);
        LogicalCodebaseStore::for_lc(paths.clone(), lc_id)
            .save_manifest("project_0001", &manifest)
            .expect("save lc manifest");

        let gateway = factory
            .build_for_lc("project_0001", Some(lc_id))
            .expect("build lc gateway");

        // 材料齐备(manifest/policy/capability 由 material prep 物化),但 lc
        // 子树没有该凭据对应的 Running operation——重核验通道在场时结果必须是
        // denied(而非无通道的 unavailable)。
        let credential =
        crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential::for_test(
            "project_0001",
            lc_id,
            "operation_missing_0001",
            crate::product::logical_codebase::AggregateInitializationStepKind::PreCheck,
            "sha256:recheck-input",
            paths.root().to_path_buf(),
        );
        let aggregate_root = paths.root().join("aggregate");
        std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
        let request = SessionLaunchRequest::planning(
            "project_0001",
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::aggregate_root(aggregate_root),
            vec![paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let error = gateway
            .validate_root_recipe_request(request, &credential)
            .expect_err("credential without a Running operation must be denied");
        let text = error.to_string();
        assert!(
            !text.contains("root_recipe_credential_recheck_unavailable"),
            "for_lc 组装必须携带凭据重核验通道, got: {text}"
        );
        assert!(
            text.contains("root_recipe_credential_recheck_denied"),
            "无 Running operation 的凭据必须被真实重核验拒绝, got: {text}"
        );
    }

    /// Task 1c-factory(lcg_t01):`with_lc_sync_bridge` 组装的 gateway 同步槽是
    /// LC gateway sync bridge——raw(未经 prepare 的)同步 `run` 由 bridge
    /// fail-closed 拒绝,绝不回落裸同步直连。
    #[test]
    fn lcg_t01_lc_sync_bridge_factory_rejects_raw_sync_run() {
        let root = tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let factory = LogicalCodebaseGatewayFactory::with_lc_sync_bridge(
            paths.clone(),
            fake_registry(),
            always_available_gate(),
        );
        register_manifest(&paths, "project_0001");
        let gateway = factory.build("project_0001").expect("build gateway");

        let aggregate_root = paths.root().join("aggregate");
        std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
        let request = SessionLaunchRequest::planning(
            "project_0001",
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::aggregate_root(aggregate_root.clone()),
            vec![paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = gateway.validate(request).expect("validate policy");
        let input = crate::protocol::contracts::AdapterInput {
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::WorkItemSplitter,
            working_directory: Some(aggregate_root),
            worktree_path: None,
            provider_stream_log_dir: None,
            prompt: "lc sync bridge raw run probe".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 5,
            max_retries: 1,
        };
        let launch =
            crate::cross_cutting::session_launch::ValidatedAdapterInput::new(input, validated);
        let error = gateway
            .run_sync(launch)
            .expect_err("raw sync run must be rejected by the LC gateway sync bridge");
        assert!(
            error
                .to_string()
                .contains("lc gateway sync bridge only accepts validated launches"),
            "sync slot must be the LC gateway sync bridge, got: {error}"
        );
    }

    #[test]
    fn web_app_state_installs_factory_and_supports_injection() {
        let root = tempdir().expect("root");
        let state = crate::web::state::WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        );
        assert!(state.gateway_factory().is_some());

        let injected = Arc::new(LogicalCodebaseGatewayFactory::new(
            ProductAppPaths::new(root.path().join(".aria")),
            fake_registry(),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
        ));
        let state = state.with_gateway_factory(injected.clone());
        assert!(Arc::ptr_eq(
            state.gateway_factory().expect("injected factory"),
            &injected
        ));
    }

    #[test]
    fn web_app_state_injected_registry_rebuilds_logical_gateway_factory() {
        let root = tempdir().expect("root");
        let injected = fake_registry();
        let state = crate::web::state::WebAppState::with_events_and_provider_registry(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
            crate::web::events::EventHub::new(),
            injected.clone(),
        );

        let factory = state.gateway_factory().expect("factory").clone();
        assert!(Arc::ptr_eq(&factory.registry, &injected));
    }
}
