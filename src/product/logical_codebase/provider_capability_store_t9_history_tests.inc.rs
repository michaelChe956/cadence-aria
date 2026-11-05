// provider capability store 的 **Task 9 全开证据语义换代**用例
// （schema 3 / `execution_semantics`）。
//
// 纯搬移：原内联在 `provider_capability_store.rs` 的 `mod tests` 里，为满足
// `tests/it_core/large_file_guard.rs` 的行数门禁（该文件的登记上限不可增长）
// 而拆出——用例体逐字不变，只换承载文件。

    // ===== Task 9:全开证据语义换代(schema 3/execution_semantics)=====

    /// v2 历史 JSON(三格 Confirmed + 无执行语义):读取时当前行必须全
    /// Unknown(旧 Confirmed 不铸新证据),历史文件字节不得被读取改写。
    const T9_V2_CONFIRMED_HISTORY: &str = r#"[
  {
    "provider_type": "claude_code",
    "schema_version": 2,
    "version": "1.2.3",
    "adapter_dialect": "claude_code_cli_v1",
    "wire_dialect": "claude-stream-json",
    "capability_snapshot_ref": "cap_managed_snapshot",
    "evidence": "production_verified",
    "resume_evidence": "confirmed",
    "supported_actions": ["coding_target_write"],
    "action_matrix": [
      {
        "action": "coding_target_write",
        "launch": {"state": "confirmed"},
        "resume": {"state": "confirmed"},
        "write_boundary": {"state": "confirmed"},
        "projection_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "evidence_ref": "probe://boundary/history-v2"
      },
      {
        "action": "planning_read_only",
        "launch": {"state": "confirmed"},
        "resume": {"state": "confirmed"},
        "write_boundary": {"state": "confirmed"},
        "projection_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "evidence_ref": "probe://boundary/history-v2"
      }
    ],
    "trust": {"state": "confirmed"},
    "probed_at": "2026-10-01T00:00:00Z",
    "probe_artifact_ref": "probe://boundary/history-v2",
    "root_recipe_evidence": {"kind": "none"}
  }
]"#;

    #[test]
    fn t9_v2_confirmed_history_reads_unknown_and_keeps_file_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));
        lcg_t02_write_capabilities_json(&temp, T9_V2_CONFIRMED_HISTORY);
        let path = temp
            .path()
            .join("projects/project_0001/logical-codebase/capabilities.json");
        let bytes_before = std::fs::read(&path).unwrap();

        let record = store
            .get("project_0001", ProviderRefType::ClaudeCode)
            .unwrap()
            .expect("v2 历史记录可解码(不报错、不删除)");

        // v2 三格 Confirmed 不能成为当前 Confirmed:当前行视图全 Unknown。
        for action in [
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::ReviewReadOnly,
        ] {
            let row = record.current_action_row(
                ProviderWireDialect::ClaudeCodeStreamJson,
                "1.2.3",
                action,
            );
            assert_eq!(
                row.launch,
                ProviderCapabilityEvidence::Unknown,
                "{action:?}"
            );
            assert_eq!(
                row.resume,
                ProviderCapabilityEvidence::Unknown,
                "{action:?}"
            );
            assert_eq!(
                row.write_boundary,
                ProviderCapabilityEvidence::Unknown,
                "{action:?}"
            );
        }
        // 矩阵本体也读取为 Unknown(任何矩阵消费面不得看到旧 Confirmed)。
        for row in record.action_matrix.rows() {
            assert_eq!(row.launch, ProviderCapabilityEvidence::Unknown);
            assert_eq!(row.resume, ProviderCapabilityEvidence::Unknown);
            assert_eq!(row.write_boundary, ProviderCapabilityEvidence::Unknown);
        }

        // 历史文件字节不变(读取不重写历史含义)。
        let bytes_after = std::fs::read(&path).unwrap();
        assert_eq!(
            bytes_before, bytes_after,
            "读取 v2 历史记录不得改写历史文件字节"
        );
    }

    /// schema 3 缺 execution_semantics(或语义非 host-open-v1):行 Unknown,
    /// 不报错、不冒充(新语义未知必须 Unknown)。
    #[test]
    fn t9_v3_missing_or_foreign_semantics_reads_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let store = ProviderCapabilityStore::new(ProductAppPaths::new(temp.path()));
        let row = r#"
      {
        "action": "coding_target_write",
        "launch": {"state": "confirmed"},
        "resume": {"state": "confirmed"},
        "write_boundary": {"state": "confirmed"},
        "projection_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "evidence_ref": "probe://boundary/semantics"
      }"#;
        for (label, semantics_json) in [
            ("missing", String::new()),
            (
                "foreign",
                r#""execution_semantics": "bwrap-phys-deny-v0","#.to_string(),
            ),
        ] {
            let json = format!(
                r#"[{{
  "provider_type": "claude_code",
  "schema_version": 3,
  {semantics_json}
  "version": "1.2.3",
  "adapter_dialect": "claude_code_cli_v1",
  "wire_dialect": "claude-stream-json",
  "capability_snapshot_ref": "cap_managed_snapshot",
  "evidence": "production_verified",
  "resume_evidence": "confirmed",
  "supported_actions": [],
  "action_matrix": [{row}],
  "trust": {{"state": "confirmed"}},
  "probed_at": "2026-10-01T00:00:00Z",
  "probe_artifact_ref": "probe://boundary/semantics",
  "root_recipe_evidence": {{"kind": "none"}}
}}]"#
            );
            lcg_t02_write_capabilities_json(&temp, &json);
            let record = store
                .get("project_0001", ProviderRefType::ClaudeCode)
                .unwrap_or_else(|error| panic!("{label} 语义缺位/未知应可解码为 Unknown: {error}"))
                .expect("记录在场");
            let coding = record
                .action_matrix
                .row(&SessionPolicyAction::CodingTargetWrite);
            assert_eq!(
                coding.launch,
                ProviderCapabilityEvidence::Unknown,
                "{label}: 缺语义/未知语义不得成为当前 Confirmed"
            );
            assert_eq!(
                coding.write_boundary,
                ProviderCapabilityEvidence::Unknown,
                "{label}: 缺语义/未知语义不得成为当前 Confirmed"
            );
        }
    }
