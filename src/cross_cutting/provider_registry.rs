use std::collections::HashMap;
use std::sync::Arc;

use crate::cross_cutting::provider_availability_gate::{
    GatedStreamingProviderAdapter, ProviderAvailabilityGate,
};
use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector;
use crate::product::models::ProviderName;

/// registry 原子注册错误(Task 1a)。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("provider_registry: {0}")]
pub struct ProviderRegistryError(pub String);

pub struct ProviderRegistry {
    providers: HashMap<ProviderName, Arc<dyn StreamingProviderAdapter>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// 普通注册(legacy/test 边界,§0.4):只写 adapter、不携带 projector,
    /// 因此**不再生产可用**——生产消费面(`register_gated` 原子三件套与
    /// `projector` getter)对这种半注册 entry 不可见。
    pub fn register(&mut self, name: ProviderName, provider: Arc<dyn StreamingProviderAdapter>) {
        self.providers.insert(name, provider);
    }

    /// 原子注册同一 provider key 的 adapter、policy projector 与 availability
    /// gate(Task 1a 冻结签名)。三件套要么同 key 一次写入、要么都不写入,
    /// 不允许出现「有 adapter 无 projector」的半注册生产 entry。
    ///
    /// 🔴 Task 1a 阶段 1 桩:projector 尚未原子入表(同 key 三件套存储在
    /// 阶段 2 实现,红点见 `lcg_t01_registry_*`);gate 装饰行为沿用现状。
    pub fn register_gated(
        &mut self,
        name: ProviderName,
        adapter: Arc<dyn StreamingProviderAdapter>,
        projector: Arc<dyn ProviderPolicyProjector>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Result<(), ProviderRegistryError> {
        let _ = &projector;
        if name == ProviderName::Fake {
            self.register(name, adapter);
            return Ok(());
        }
        let gated = Arc::new(GatedStreamingProviderAdapter::new(
            name.clone(),
            adapter,
            gate,
        ));
        self.register(name, gated);
        Ok(())
    }

    /// test-only:LC 测试 fixture 注册 adapter+projector 对(§0.4;不带
    /// availability gate)。生产装配必须走 `register_gated` 三件套。
    ///
    /// 🔴 Task 1a 阶段 1 桩:projector 尚未入表(阶段 2 实现)。
    pub fn register_test_pair(
        &mut self,
        name: ProviderName,
        adapter: Arc<dyn StreamingProviderAdapter>,
        projector: Arc<dyn ProviderPolicyProjector>,
    ) -> Result<(), ProviderRegistryError> {
        let _ = &projector;
        self.register(name, adapter);
        Ok(())
    }

    /// 生产 projector getter:只对 `register_gated`/`register_test_pair`
    /// 原子写入的 entry 可见;普通 `register` 的半注册 entry 一律 `None`。
    ///
    /// 🔴 Task 1a 阶段 1 桩:恒返回 `None`(阶段 2 实现原子 entry 读取)。
    pub fn projector(&self, _name: &ProviderName) -> Option<Arc<dyn ProviderPolicyProjector>> {
        None
    }

    pub fn get(&self, name: &ProviderName) -> Option<Arc<dyn StreamingProviderAdapter>> {
        self.providers.get(name).cloned()
    }

    pub fn available_names(&self) -> Vec<ProviderName> {
        [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
            ProviderName::Fake,
        ]
        .into_iter()
        .filter(|name| self.providers.contains_key(name))
        .collect()
    }

    pub fn executable_names(&self, gate: &ProviderAvailabilityGate) -> Vec<ProviderName> {
        self.available_names()
            .into_iter()
            .filter(|name| gate.ensure_available(name).is_ok())
            .collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{
        ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
    };
    use crate::cross_cutting::streaming_provider::{
        FakeStreamingProvider, ProviderPermissionMode, StreamingProviderInput,
    };
    use crate::product::logical_codebase::provider_projection::{
        ProviderPolicyProjection, ProviderPolicyProjector, ProviderProjectionError,
        ProviderProjectionInput, UnprovisionedProviderPolicyProjector,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};
    use crate::protocol::provider_errors::ProviderErrorCode;

    struct RegistryHealthSource(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for RegistryHealthSource {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    fn registry_gate() -> Arc<ProviderAvailabilityGate> {
        let checked_at = Utc::now();
        Arc::new(ProviderAvailabilityGate::new(Arc::new(
            RegistryHealthSource(Arc::new(ProviderHealthSnapshot {
                schema_version: 1,
                generation: 1,
                checked_at,
                providers: vec![
                    ProviderHealthEntry {
                        provider: ProviderName::ClaudeCode,
                        command: "claude --version".to_string(),
                        available: false,
                        version: None,
                        reason_code: Some(ProviderHealthReasonCode::CommandMissing),
                        reason: Some("not found".to_string()),
                        checked_at,
                    },
                    ProviderHealthEntry {
                        provider: ProviderName::Codex,
                        command: "codex --version".to_string(),
                        available: true,
                        version: Some("1.0".to_string()),
                        reason_code: None,
                        reason: None,
                        checked_at,
                    },
                ],
            })),
        )))
    }

    #[test]
    fn provider_registry_returns_registered_fake_provider() {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));

        assert!(registry.get(&ProviderName::Fake).is_some());
        assert!(registry.get(&ProviderName::ClaudeCode).is_none());
    }

    #[test]
    fn provider_registry_available_names_use_stable_provider_order() {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::Fake, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::Codex, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::Pi, Arc::new(FakeStreamingProvider));
        registry.register(ProviderName::KimiCode, Arc::new(FakeStreamingProvider));

        assert_eq!(
            registry.available_names(),
            vec![
                ProviderName::ClaudeCode,
                ProviderName::Codex,
                ProviderName::Pi,
                ProviderName::KimiCode,
                ProviderName::Fake
            ]
        );
    }

    #[test]
    fn provider_availability_gate_registry_distinguishes_registered_and_executable_names() {
        let gate = registry_gate();
        let mut registry = ProviderRegistry::new();
        registry.register_gated(
            ProviderName::ClaudeCode,
            Arc::new(FakeStreamingProvider),
            Arc::new(UnprovisionedProviderPolicyProjector),
            gate.clone(),
        );
        registry.register_gated(
            ProviderName::Codex,
            Arc::new(FakeStreamingProvider),
            Arc::new(UnprovisionedProviderPolicyProjector),
            gate.clone(),
        );
        registry.register_gated(
            ProviderName::Pi,
            Arc::new(FakeStreamingProvider),
            Arc::new(UnprovisionedProviderPolicyProjector),
            gate.clone(),
        );
        registry.register_gated(
            ProviderName::Fake,
            Arc::new(FakeStreamingProvider),
            Arc::new(UnprovisionedProviderPolicyProjector),
            gate.clone(),
        );

        assert_eq!(
            registry.available_names(),
            vec![
                ProviderName::ClaudeCode,
                ProviderName::Codex,
                ProviderName::Pi,
                ProviderName::Fake
            ]
        );
        assert_eq!(
            registry.executable_names(&gate),
            vec![ProviderName::Codex, ProviderName::Fake]
        );
    }

    /// registry 测试用 projector fixture:project 恒拒绝(断言只用实例
    /// 指针 identity,不真正投影)。
    struct RegistryProjector(&'static str);

    impl ProviderPolicyProjector for RegistryProjector {
        fn project(
            &self,
            _input: &ProviderProjectionInput,
        ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
            Err(ProviderProjectionError::Unsupported(format!(
                "registry 测试 projector {} 不产出投影",
                self.0
            )))
        }
    }

    fn registry_streaming_input() -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            prompt: "registry gated probe".to_string(),
            working_dir: std::env::current_dir().expect("cwd"),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 1,
        }
    }

    /// Task 1a(lcg_t01):普通 `register` 只写 adapter、不带 projector,是
    /// 半注册——生产 projector 视图必须对它不可见;完整注册(带 projector)
    /// 后同 key 可见(计划 Task 1 Step 1「半注册无可见 entry」)。
    #[test]
    fn lcg_t01_registry_rejects_half_registered_adapter_without_projector() {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));

        // 半注册无可见 entry:生产 projector getter 拒绝提供
        assert!(
            registry.projector(&ProviderName::ClaudeCode).is_none(),
            "无 projector 的半注册 adapter 不得进入生产 projector 视图"
        );

        // 对照:register_test_pair 原子写 adapter+projector 后同 key 可见
        registry
            .register_test_pair(
                ProviderName::ClaudeCode,
                Arc::new(FakeStreamingProvider),
                Arc::new(RegistryProjector("pair")),
            )
            .expect("register_test_pair 原子注册");
        assert!(
            registry.projector(&ProviderName::ClaudeCode).is_some(),
            "完整注册(adapter+projector)后 projector 必须可见"
        );

        // 其它 key 不受影响
        assert!(registry.projector(&ProviderName::Codex).is_none());
    }

    /// Task 1a(lcg_t01):`register_gated` 把 adapter、projector、availability
    /// gate 以同一 provider key 原子保存;取用时复验注册时的同一 gate。
    #[tokio::test]
    async fn lcg_t01_registry_keeps_adapter_projector_and_gate_same_key() {
        let gate = registry_gate();
        let mut registry = ProviderRegistry::new();
        let projector: Arc<dyn ProviderPolicyProjector> = Arc::new(RegistryProjector("claude"));
        registry
            .register_gated(
                ProviderName::ClaudeCode,
                Arc::new(FakeStreamingProvider),
                projector.clone(),
                gate,
            )
            .expect("register_gated 原子注册 adapter/projector/gate");

        // 同 key:adapter 与 projector 同时可见,且是注册时的同一 projector 实例
        assert!(registry.get(&ProviderName::ClaudeCode).is_some());
        let stored = registry
            .projector(&ProviderName::ClaudeCode)
            .expect("projector 与 adapter 同 key 可见");
        assert!(
            Arc::ptr_eq(&stored, &projector),
            "projector getter 必须返回注册时的同一实例"
        );
        // 无串扰:未注册 key 不可见
        assert!(registry.projector(&ProviderName::Codex).is_none());

        // availability recheck 使用注册时的同一 gate:health 不可用的 provider
        // 经 registry 取出的 gated adapter 启动被拒(零 spawn)
        let gated_adapter = registry
            .get(&ProviderName::ClaudeCode)
            .expect("gated adapter");
        let error = gated_adapter
            .start(registry_streaming_input(), CancellationToken::new())
            .await
            .err()
            .expect("gate 不可用时必须拒绝启动");
        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
    }

    /// Task 1a(lcg_t01):projector 与 adapter 同一 entry 生命周期——同 key
    /// 重新原子注册整体替换(旧 projector 不残留);半注册 `register` 覆盖后
    /// 生产 projector 视图随之消失,不形成「新 adapter+旧 projector」错配。
    #[test]
    fn lcg_t01_registry_projector_lifetime_matches_adapter() {
        let gate = registry_gate();
        let mut registry = ProviderRegistry::new();
        let first: Arc<dyn ProviderPolicyProjector> = Arc::new(RegistryProjector("first"));
        registry
            .register_gated(
                ProviderName::ClaudeCode,
                Arc::new(FakeStreamingProvider),
                first.clone(),
                gate.clone(),
            )
            .expect("首次原子注册");

        let second: Arc<dyn ProviderPolicyProjector> = Arc::new(RegistryProjector("second"));
        registry
            .register_gated(
                ProviderName::ClaudeCode,
                Arc::new(FakeStreamingProvider),
                second.clone(),
                gate,
            )
            .expect("同 key 重新原子注册整体替换");

        let stored = registry
            .projector(&ProviderName::ClaudeCode)
            .expect("覆盖注册后 projector 仍与 adapter 同 key 可见");
        assert!(Arc::ptr_eq(&stored, &second), "新 projector 生效");
        assert!(!Arc::ptr_eq(&stored, &first), "旧 projector 不得残留");

        // 半注册覆盖同 key:整体替换(adapter 仍可取,projector 视图消失)
        registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
        assert!(registry.get(&ProviderName::ClaudeCode).is_some());
        assert!(
            registry.projector(&ProviderName::ClaudeCode).is_none(),
            "半注册不得保留旧 projector 形成「新 adapter+旧 projector」错配"
        );
    }
}
