use super::anchor_t10_support::*;
use super::*;
use crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent;
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;

fn provider_start_records(
    sink: &RecordingToolPolicyAuditSink,
) -> Vec<crate::cross_cutting::tool_policy_audit::ProviderStartAudit> {
    sink.events()
        .iter()
        .filter_map(|event| match event {
            DurableToolPolicyEvent::ProviderStart(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}

/// Task 3.7 Step 1 用例一:四家公共 fixture 汇总——full 唯一 start 路径
/// 无 unsupported/basis 旁路;oldbasis/missingfull 拒等待(unavailable
/// 稳定原因码);投影/audit/fingerprint 同源;wrongID 如实 kill/reap;
/// TTL 三计数真实;Pi extension 早写拒收/Kimi native-source 与 Aria
/// bundle 区分/Codex 受限 sandbox/Claude --resume 事实逐家消费。
#[tokio::test]
async fn anchor_t10_full_projection_all_providers_no_basis_resume() {
    for family in AnchorT10Family::ALL {
        let tag = family.tag();
        let harness = anchor_t10_harness(family);
        harness.install_policy();
        let sink = RecordingToolPolicyAuditSink::new();

        // -- 默认 unsupported 不可达正例的前置:未接入 adapter(默认
        //    trait)在 full 路径只得到 typed 拒,registry start 零触达。
        let mut bare_registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
        bare_registry.register(
            family.name(),
            Arc::new(CountingStreamingAdapter::new()) as Arc<dyn StreamingProviderAdapter>,
        );
        let bare_error = match harness
            .gateway_with_registry(bare_registry)
            .prepare_lc_streaming_candidate(
                harness.streaming_input(None),
                harness.request.clone(),
                harness.audit_context(sink.clone()),
                CancellationToken::new(),
            )
            .await
        {
            Err(ProviderGatewayError::Adapter(error)) => error,
            Err(other) => panic!("[{tag}] unexpected rejection shape: {other}"),
            Ok(_) => panic!("[{tag}] default adapter must fail closed with typed unsupported"),
        };
        assert!(
            bare_error
                .stderr
                .contains("lc prepare facts is not supported"),
            "[{tag}] unexpected default rejection: {bare_error:?}"
        );

        // -- 正例:真实 facts prepare(口径①恰一次)。
        let candidate = harness
            .gateway
            .prepare_lc_streaming_candidate(
                harness.streaming_input(None),
                harness.request.clone(),
                harness.audit_context(sink.clone()),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("[{tag}] prepare failed: {error}"));
        assert_eq!(harness.adapter.probe_calls(), 1, "[{tag}] ① probe");
        assert_eq!(candidate.facts().exact_version(), family.exact_version());
        assert_eq!(
            candidate.facts().version_probe_key().provider,
            family.name()
        );
        assert_eq!(
            candidate.facts().approval_policy(),
            family.approval_policy()
        );
        assert_eq!(
            candidate.full_projection().mcp_bundle_digest(),
            family.aria_mcp_bundle_digest()
        );

        // -- full 指纹与 prepared full map(basis=FullLcProjectionV2)同源。
        let fingerprint = candidate.full_context_fingerprint();
        assert!(!fingerprint.digest.is_empty());
        let persisted_prepared = crate::product::models::LogicalLaunchFingerprintRecord {
            schema_version: 1,
            basis: crate::product::models::LogicalLaunchBasis::FullLcProjectionV2,
            role: crate::product::models::ProviderConversationRole::Author,
            provider: family.name(),
            role_run_seq: 1,
            node_id: "node-t10".to_string(),
            native_session_id: None,
            fingerprint_digest: fingerprint.digest.clone(),
            planning_access_fingerprint: fingerprint.digest.clone(),
            state: crate::product::models::LogicalLaunchFingerprintState::Prepared,
            created_at: "2026-10-11T00:00:00Z".to_string(),
            updated_at: "2026-10-11T00:00:00Z".to_string(),
        };

        // -- full-matching resume:Resume,validated 携带冻结 full 投影。
        let disposition = harness
            .gateway
            .resume_or_start_prepared_lc_candidate(
                &candidate,
                PreparedLcResumeBasis::Full {
                    previous_fingerprint: fingerprint.clone(),
                    previous_session_id: "native-t10-previous".to_string(),
                },
            )
            .expect("[{tag}] full-matching resume decision");
        match &disposition {
            GatewaySessionDisposition::Resume(validated) => {
                assert_eq!(
                    validated
                        .lc_projection()
                        .map(|projection| projection.projection_digest()),
                    Some(candidate.full_projection().projection_digest()),
                    "[{tag}] Resume validated must carry the frozen full projection"
                );
                assert_eq!(
                    validated.fingerprint().digest,
                    persisted_prepared.fingerprint_digest
                );
            }
            _ => panic!("[{tag}] full-matching fingerprint must resume"),
        }

        // -- 旧 basis(无 v2 schema 前缀的 legacy digest)不得与 full 混比
        //    (fix-loop P2:spec 禁止旧 basis 进 full 比对/对从未可信的
        //    会话写 supersede):误传 Full 臂也必须 fail-closed 为
        //    unavailable 等待,零 supersede。
        let supersede_before = harness.audit.supersede_count();
        let legacy = harness
            .gateway
            .resume_or_start_prepared_lc_candidate(
                &candidate,
                PreparedLcResumeBasis::Full {
                    previous_fingerprint: SessionResumeFingerprint {
                        digest: "sha256:legacy-v1-basis-t10".to_string(),
                    },
                    previous_session_id: "native-t10-legacy".to_string(),
                },
            )
            .expect("[{tag}] legacy basis disposition");
        match legacy {
            GatewaySessionDisposition::StartNew {
                waiting, validated, ..
            } => {
                assert_eq!(
                    waiting.reason_code, RESUME_FINGERPRINT_UNAVAILABLE,
                    "[{tag}] legacy digest fed to the Full arm must fail closed to \
                     resume_fingerprint_unavailable waiting, never a mixed comparison"
                );
                assert!(validated.lc_projection().is_some());
            }
            _ => panic!("[{tag}] legacy basis must wait for explicit fresh"),
        }
        assert_eq!(
            harness.audit.supersede_count(),
            supersede_before,
            "[{tag}] legacy digest must not supersede a never-trusted session"
        );

        // -- 真 v2 漂移(带 schema 前缀、单维度不同):full 对 full 比对,
        //    mismatch 等待 + supersede 审计(既有语义保持)。
        let drifted_v2 = SessionResumeFingerprint::from_envelope(
            candidate.validated().envelope(),
            candidate.full_projection(),
            "projection-digest-t10-drifted-evidence",
            &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
        );
        let drifted = harness
            .gateway
            .resume_or_start_prepared_lc_candidate(
                &candidate,
                PreparedLcResumeBasis::Full {
                    previous_fingerprint: drifted_v2,
                    previous_session_id: "native-t10-drifted".to_string(),
                },
            )
            .expect("[{tag}] genuine v2 drift disposition");
        match drifted {
            GatewaySessionDisposition::StartNew { waiting, .. } => {
                assert_eq!(waiting.reason_code, "resume_fingerprint_mismatch");
            }
            _ => panic!("[{tag}] genuine v2 drift must wait for explicit fresh"),
        }
        assert_eq!(harness.audit.supersede_count(), supersede_before + 1);
        assert_eq!(
            harness.adapter.adapter_starts(),
            0,
            "[{tag}] ② after dispositions"
        );
        assert_eq!(
            harness.adapter.session_children(),
            0,
            "[{tag}] ③ after dispositions"
        );

        // -- 缺 full map/旧 basis:Unavailable → resume_fingerprint_unavailable
        //    等待,不 supersede 未知会话。
        let unavailable = harness
            .gateway
            .resume_or_start_prepared_lc_candidate(&candidate, PreparedLcResumeBasis::Unavailable)
            .expect("[{tag}] unavailable basis disposition");
        match unavailable {
            GatewaySessionDisposition::StartNew {
                waiting, validated, ..
            } => {
                assert_eq!(
                    waiting.reason_code, RESUME_FINGERPRINT_UNAVAILABLE,
                    "[{tag}] missing full map must wait with resume_fingerprint_unavailable"
                );
                assert!(validated.lc_projection().is_some());
            }
            _ => panic!("[{tag}] unavailable basis must wait for explicit fresh"),
        }
        assert_eq!(
            harness.audit.supersede_count(),
            supersede_before + 1,
            "[{tag}] unavailable must not supersede an unknown session"
        );
        assert_eq!(harness.adapter.adapter_starts(), 0);
        assert_eq!(harness.adapter.session_children(), 0);

        // -- full 唯一 start 路径:成功 spawn,三计数真实(TTL 命中,①不增)。
        let stream_launches_before = harness.audit.stream_launches();
        let session = harness
            .gateway
            .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
            .await
            .unwrap_or_else(|error| panic!("[{tag}] unique start failed: {error}"));
        assert_eq!(
            session.native_session_id.as_deref(),
            Some(format!("native-{tag}-t10").as_str())
        );
        assert_eq!(
            harness.adapter.probe_calls(),
            1,
            "[{tag}] TTL hit at revalidation"
        );
        assert_eq!(
            harness.adapter.adapter_starts(),
            1,
            "[{tag}] ② unique start"
        );
        assert_eq!(
            harness.adapter.session_children(),
            1,
            "[{tag}] ③ unique start"
        );
        assert_eq!(harness.adapter.children_killed(), 0);
        assert_eq!(
            harness.adapter.starts_without_projection(),
            0,
            "[{tag}] no basis bypass"
        );
        assert_eq!(harness.audit.stream_launches(), stream_launches_before + 1);

        // 投影/audit/fingerprint 同源:adapter 消费冻结 full 投影,audit
        // 字面量 v2 完整且 resume 指纹 == full 指纹 == prepared map digest。
        assert_eq!(
            harness.adapter.last_start_projection_digest().as_deref(),
            Some(candidate.full_projection().projection_digest())
        );
        let literal = harness
            .adapter
            .last_start_audit_literal()
            .expect("[{tag}] audit literal assembled");
        assert!(literal.is_v2_complete());
        assert_eq!(literal.resume_fingerprint, fingerprint.digest);
        assert_eq!(
            literal.resume_fingerprint,
            persisted_prepared.fingerprint_digest
        );
        assert_eq!(literal.wire_dialect, family.wire_text());

        // durable provider_start:lc_projection 五字段与本家标记同源。
        let records = provider_start_records(&sink);
        assert_eq!(records.len(), 1, "[{tag}] one provider_start");
        let record = &records[0];
        assert_eq!(record.provider, family.audit_provider_text());
        assert_eq!(record.provider_version, family.exact_version());
        assert_eq!(record.adapter_dialect, family.wire_text());
        assert_eq!(record.sandbox, family.audit_sandbox());
        assert_eq!(record.approval_policy, family.audit_approval());
        let audit_projection = record
            .lc_projection
            .as_ref()
            .expect("[{tag}] lc projection persisted");
        assert_eq!(
            audit_projection.projection_digest,
            candidate.full_projection().projection_digest()
        );
        assert_eq!(
            audit_projection.capability_projection_digest,
            candidate.full_projection().capability_projection_digest()
        );

        // -- 本家事实消费(公共断言,不跨 provider 文件):
        match family {
            AnchorT10Family::Claude => {
                assert!(!record.argv.contains(&"--resume".to_string()));
            }
            AnchorT10Family::Codex => {
                assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
                assert!(record.argv.contains(&"danger-full-access".to_string()));
            }
            AnchorT10Family::Pi => {
                assert_eq!(
                    harness.adapter.extension_writes(),
                    1,
                    "[pi] extension 写只在 start 器械段(prepare/判定阶段零写)"
                );
            }
            AnchorT10Family::Kimi => {
                assert!(
                    candidate
                        .full_projection()
                        .trust_digest()
                        .starts_with(&format!("lc-trust-v1:{:?}:", family.name()))
                );
                // native-source 与 Aria bundle 区分:换 native 标记后重
                // prepare,facts/projection digest 随 MCP 来源变化。
                harness
                    .adapter
                    .set_mcp_bundle_digest("kimi-native-project-config");
                let native_candidate = harness
                    .gateway
                    .prepare_lc_streaming_candidate(
                        harness.streaming_input(None),
                        harness.request.clone(),
                        harness.audit_context(sink.clone()),
                        CancellationToken::new(),
                    )
                    .await
                    .expect("[kimi] native-source prepare");
                assert_eq!(
                    native_candidate.full_projection().mcp_bundle_digest(),
                    "kimi-native-project-config"
                );
                assert_ne!(
                    native_candidate.full_projection().projection_digest(),
                    candidate.full_projection().projection_digest(),
                    "[kimi] native-source 与 Aria bundle 不得混源"
                );
            }
        }

        // kimi 的 native-source 复验不得污染 wrongID 段:恢复 aria
        // bundle,使存档/当前投影回到同源形态(错 ID 断言的是 handshake
        // 失败的 kill/reap,不是 MCP 漂移拒绝)。
        if family == AnchorT10Family::Kimi {
            harness
                .adapter
                .set_mcp_bundle_digest(family.aria_mcp_bundle_digest());
        }

        // -- wrongID 如实 kill/reap(child 后 handshake 失败,不伪称零 child)。
        let resume_id = format!("native-{tag}-t10");
        anchor_t10_preset_resume_archive(&sink, family, &resume_id, candidate.full_projection());
        harness
            .adapter
            .set_handshake_native_id(Some(format!("native-{tag}-wrong")));
        let resume_candidate = harness
            .gateway
            .prepare_lc_streaming_candidate(
                harness.streaming_input(Some(resume_id.clone())),
                harness.request.clone(),
                harness.audit_context(sink.clone()),
                CancellationToken::new(),
            )
            .await
            .expect("[{tag}] resume prepare");
        let children_before = harness.adapter.session_children();
        let killed_before = harness.adapter.children_killed();
        let provider_starts_before = provider_start_records(&sink).len();
        let wrong = harness
            .gateway
            .start_lc_prepared_streaming(resume_candidate, CancellationToken::new())
            .await;
        assert!(
            wrong.is_err(),
            "[{tag}] wrong native id must fail the resume"
        );
        assert_eq!(harness.adapter.session_children(), children_before + 1);
        assert_eq!(harness.adapter.children_killed(), killed_before + 1);
        assert_eq!(
            harness.adapter.session_children() - harness.adapter.children_killed(),
            1,
            "[{tag}] 只有成功路径的 child 存活;错 ID child 已 kill/reap"
        );
        assert_eq!(
            provider_start_records(&sink).len(),
            provider_starts_before,
            "[{tag}] 错 ID 不写 fresh provider_start"
        );

        // -- Claude --resume 事实:同 ID resume 成功,argv 携 --resume <id>。
        if family == AnchorT10Family::Claude {
            harness.adapter.set_handshake_native_id(None);
            let claude_resume_candidate = harness
                .gateway
                .prepare_lc_streaming_candidate(
                    harness.streaming_input(Some(resume_id.clone())),
                    harness.request.clone(),
                    harness.audit_context(sink.clone()),
                    CancellationToken::new(),
                )
                .await
                .expect("[claude] resume prepare");
            let claude_session = harness
                .gateway
                .start_lc_prepared_streaming(claude_resume_candidate, CancellationToken::new())
                .await
                .expect("[claude] same-id resume start");
            assert_eq!(
                claude_session.native_session_id.as_deref(),
                Some(resume_id.as_str())
            );
            let claude_records = provider_start_records(&sink);
            let last = claude_records
                .last()
                .expect("[claude] resume provider_start");
            assert!(last.argv.contains(&"--resume".to_string()));
            assert!(last.argv.contains(&resume_id));
        }
    }
}

/// Task 3.7 Step 1 用例二:spawn 前复验矩阵——prepared facts 在 session
/// start 前重验;version/trust/spawn 前变化受门、混合 provider 矩阵与
/// audit v2 完整性门全部 fail-closed 在真实 adapter start 之前(②③=0)。
#[tokio::test]
async fn anchor_t10_prepared_facts_revalidate_before_session_start() {
    // A. 四家:prepare 后 executable identity+version 漂移 → typed 拒。
    for family in AnchorT10Family::ALL {
        let tag = family.tag();
        let harness = anchor_t10_harness(family);
        harness.install_policy();
        let sink = RecordingToolPolicyAuditSink::new();
        let candidate = harness
            .gateway
            .prepare_lc_streaming_candidate(
                harness.streaming_input(None),
                harness.request.clone(),
                harness.audit_context(sink.clone()),
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| panic!("[{tag}] prepare failed: {error}"));
        assert_eq!(harness.adapter.probe_calls(), 1);
        harness
            .adapter
            .set_executable_identity(&format!("sha256:t10-{tag}-exe-v2"));
        harness
            .adapter
            .set_observed_version(&format!("{tag} 9.9.9-drifted"));
        let drift = harness
            .gateway
            .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
            .await
            .err()
            .expect("[{tag}] version drift must typed-reject before spawn");
        assert!(
            matches!(
                &drift,
                ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_version"
            ),
            "[{tag}] unexpected drift shape: {drift}"
        );
        assert_eq!(
            harness.adapter.probe_calls(),
            2,
            "[{tag}] 重观测换 key 再探测"
        );
        assert_eq!(harness.adapter.adapter_starts(), 0, "[{tag}] ② mismatch=0");
        assert_eq!(
            harness.adapter.session_children(),
            0,
            "[{tag}] ③ mismatch=0"
        );
        assert_eq!(harness.adapter.starts_without_projection(), 0);
        assert_eq!(harness.audit.stream_launches(), 0);
    }

    // B. 混合 provider 矩阵:prepare→start 间换绑同家新实例/跨家 adapter。
    let claude_harness = anchor_t10_harness(AnchorT10Family::Claude);
    claude_harness.install_policy();
    let sink = RecordingToolPolicyAuditSink::new();
    let candidate = claude_harness
        .gateway
        .prepare_lc_streaming_candidate(
            claude_harness.streaming_input(None),
            claude_harness.request.clone(),
            claude_harness.audit_context(sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("claude prepare");
    let swapped_adapter = AnchorT10FamilyAdapter::new(
        AnchorT10Family::Claude,
        Arc::new(crate::cross_cutting::streaming_provider::LcVersionCache::new()),
    );
    let mut swapped_registry = claude_harness.registry();
    swapped_registry.register_gated(
        crate::product::models::ProviderName::ClaudeCode,
        swapped_adapter.clone(),
        Arc::new(
            crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector,
        ),
        always_available_gate(),
    )
    .expect("anchor t10 register swapped adapter");
    let swapped_error = claude_harness
        .gateway_with_registry(swapped_registry)
        .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
        .await
        .err()
        .expect("swapped same-family adapter must typed-reject");
    assert!(
        matches!(
            &swapped_error,
            ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_adapter"
        ),
        "unexpected swapped shape: {swapped_error}"
    );

    let cross_adapter = AnchorT10FamilyAdapter::new(
        AnchorT10Family::Codex,
        Arc::new(crate::cross_cutting::streaming_provider::LcVersionCache::new()),
    );
    let mut cross_registry = claude_harness.registry();
    cross_registry.register_gated(
        crate::product::models::ProviderName::ClaudeCode,
        cross_adapter.clone(),
        Arc::new(
            crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector,
        ),
        always_available_gate(),
    )
    .expect("anchor t10 register cross adapter");
    let cross_error = claude_harness
        .gateway_with_registry(cross_registry)
        .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
        .await
        .err()
        .expect("cross-family adapter must typed-reject");
    assert!(
        matches!(
            &cross_error,
            ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_adapter"
        ),
        "unexpected cross shape: {cross_error}"
    );
    assert_eq!(swapped_adapter.adapter_starts(), 0);
    assert_eq!(cross_adapter.adapter_starts(), 0);
    assert_eq!(claude_harness.adapter.adapter_starts(), 0);
    assert_eq!(claude_harness.adapter.session_children(), 0);

    // C. audit v2 材料不完整:会话投影摘要为空的 projector 可 prepare
    //    冻结,但不可达生产 spawn(gateway 门先于 adapter start)。
    let v2_harness = anchor_t10_harness(AnchorT10Family::Pi);
    v2_harness.install_policy();
    let v2_sink = RecordingToolPolicyAuditSink::new();
    v2_harness
        .adapter
        .set_projector_overrides(None, Some(String::new()));
    let v2_candidate = v2_harness
        .gateway
        .prepare_lc_streaming_candidate(
            v2_harness.streaming_input(None),
            v2_harness.request.clone(),
            v2_harness.audit_context(v2_sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("v2-incomplete projector can freeze a candidate");
    let v2_error = v2_harness
        .gateway
        .start_lc_prepared_streaming(v2_candidate, CancellationToken::new())
        .await
        .err()
        .expect("v2-incomplete audit material must not spawn");
    assert!(
        matches!(&v2_error, ProviderGatewayError::UnsupportedCapability(message)
            if message.contains("v2 material incomplete")),
        "unexpected v2 shape: {v2_error}"
    );
    assert_eq!(v2_harness.adapter.adapter_starts(), 0);
    assert_eq!(v2_harness.adapter.session_children(), 0);

    // D. trust spawn 前变化受门(Codex 需 workspace trust):prepare 冻结
    //    后 trust 失效 → typed 拒,零 spawn。
    let codex_harness = anchor_t10_harness(AnchorT10Family::Codex);
    codex_harness.install_policy();
    let codex_sink = RecordingToolPolicyAuditSink::new();
    let codex_candidate = codex_harness
        .gateway
        .prepare_lc_streaming_candidate(
            codex_harness.streaming_input(None),
            codex_harness.request.clone(),
            codex_harness.audit_context(codex_sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("codex prepare under trusted source");
    assert!(
        codex_candidate
            .full_projection()
            .trust_digest()
            .contains("sha256:t10-trust-record-codex")
    );
    codex_harness.trust.set_trusted(false);
    let trust_error = codex_harness
        .gateway
        .start_lc_prepared_streaming(codex_candidate, CancellationToken::new())
        .await
        .err()
        .expect("trust drift before spawn must reject");
    assert!(
        trust_error.to_string().contains("trust"),
        "unexpected trust shape: {trust_error}"
    );
    assert_eq!(codex_harness.adapter.adapter_starts(), 0);
    assert_eq!(codex_harness.adapter.session_children(), 0);

    // E. 相位收口:非 PlanningReadOnly 候选不经唯一 start 路径(Coding/
    //    Recipe/Review 等既有相位仍走同名 gateway API)。ReviewReadOnly
    //    与 planning 同为 read-only 形态,只差 action,可直接 prepare。
    let phase_harness = anchor_t10_harness(AnchorT10Family::Claude);
    phase_harness.install_policy();
    let phase_sink = RecordingToolPolicyAuditSink::new();
    let mut review_request = phase_harness.request.clone();
    review_request.action = SessionPolicyAction::ReviewReadOnly;
    let phase_candidate = phase_harness
        .gateway
        .prepare_lc_streaming_candidate(
            phase_harness.streaming_input(None),
            review_request,
            phase_harness.audit_context(phase_sink),
            CancellationToken::new(),
        )
        .await
        .expect("review candidate can be prepared");
    let phase_error = phase_harness
        .gateway
        .start_lc_prepared_streaming(phase_candidate, CancellationToken::new())
        .await
        .err()
        .expect("non-planning candidate must not use the converged start path");
    assert!(
        matches!(&phase_error, ProviderGatewayError::UnsupportedCapability(message)
            if message.contains("PlanningReadOnly")),
        "unexpected phase shape: {phase_error}"
    );
    assert_eq!(phase_harness.adapter.adapter_starts(), 0);
}
