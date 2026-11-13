//! Wire DTOs for the SC failure evidence / repair package contract
//! (Plan §2.2, frozen).
//!
//! These types freeze the shared Rust/TS wire contract for the strict A4
//! publication path. Every field name below is part of the frozen contract
//! and must stay name-for-name identical to the TypeScript mirror. Evidence
//! bodies are always referenced (`ScEvidenceRef`), never inlined, and a
//! missing reference is explicit — there is no `None`/empty-string fallback
//! for evidence bodies.

use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

/// Workspace-scoped identity of the chain that owns a failure package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScFailureScope {
    pub session_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
}

/// Identity of the single-candidate chain a repair package describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScFailureChainIdentity {
    pub scope: ScFailureScope,
    pub chain_id: String,
    pub business_key: String,
    pub original_node_id: String,
    pub original_role_run_seq: u64,
}

/// Identity of one round (original or teaching) inside the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScRoundIdentity {
    pub chain_id: String,
    pub round_id: String,
    pub node_id: String,
    pub round_ordinal: u32,
    pub provider: ProviderName,
    pub workspace_session_id: String,
    pub role_run_seq: u64,
    pub run_ref: String,
}

/// Reference to evidence bytes held by the strict evidence store.
///
/// Serialized as an internally tagged enum with tag `status`: `available`
/// carries `locator`/`sha256`/`bytes` (sha256 is the 64-char lowercase hex
/// of the exact UTF-8 body bytes; `bytes` is that byte length); `missing`
/// carries the explicit `code`/`detail`/`round_id` triple. There is
/// deliberately no third "present but empty" state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScEvidenceRef {
    Available {
        locator: String,
        sha256: String,
        bytes: u64,
    },
    Missing {
        code: String,
        detail: String,
        round_id: String,
    },
}

/// One entry of the package-level `missing_evidence` list.
///
/// `locator` is `Some` only when the store knows where the evidence should
/// have lived; errors without a known locator keep `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScMissingEvidence {
    pub round_id: String,
    pub kind: String,
    pub code: String,
    pub detail: String,
    pub locator: Option<String>,
}

/// The three terminal SC failure classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScFailureClass {
    Compile,
    IrPreflight,
    StructureGate,
}

/// Where a teaching prompt's source evidence originates from.
///
/// Defined here (Plan §2.2) so the failure-source resolver and the shared
/// source-fence renderer can both consume it without ordering dependencies
/// between their slices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingSourceOrigin {
    CompileSplitOutput,
    IrPreflightSplitOutput,
    StructureRevision,
}

/// Observed launch state of a round; reflects real observation only and is
/// never derived from RoundBound/split ordinals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScRoundLaunchState {
    NotStarted,
    Started,
    Unknown,
}

/// What happened to one teaching prompt.
///
/// `Unavailable` carries its concrete reason inside the prompt's
/// `ScEvidenceRef::Missing` payload; a prompt that was prepared but never
/// sent stays `Prepared` and must not masquerade as `Sent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScPromptDisposition {
    Prepared,
    Sent,
    RejectedBudget,
    Unavailable,
}

/// One teaching prompt with its evidence reference and disposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScTeachingPromptEvidence {
    pub failed_round_id: String,
    pub target_round_ordinal: u32,
    pub builder_id: String,
    pub prompt: ScEvidenceRef,
    pub disposition: ScPromptDisposition,
}

/// Projection of an already-made teaching decision; grants no eligibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScTeachingDecision {
    NotRequested,
    Ineligible,
    Denied,
    Drift,
    Granted,
    Interrupted,
}

/// Evidence for one round of the failure chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScRoundEvidence {
    pub identity: ScRoundIdentity,
    pub launch_state: ScRoundLaunchState,
    pub failure_class: Option<ScFailureClass>,
    pub blocking_reasons: Vec<String>,
    pub repair_instructions: Vec<String>,
    pub raw: ScEvidenceRef,
    pub source: ScEvidenceRef,
    pub existing_failure_id: Option<String>,
    pub attempt_key: Option<String>,
    pub prompts: Vec<ScTeachingPromptEvidence>,
    pub teaching_decision: ScTeachingDecision,
}

/// Whether a package is complete; must agree with `missing_evidence`,
/// otherwise the package is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScPackageCompleteness {
    Complete,
    Incomplete,
}

/// Reference to the source evidence of the last failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScFailureSourceEvidence {
    pub source_round_id: String,
    pub source: ScEvidenceRef,
}

/// The complete, self-contained SC failure repair package (A4 payload).
///
/// `terminal_class` is one of the three failure classes in snake_case or
/// `teaching_interrupted`; the original failure reasons always stay in
/// `terminal_reason`/`rounds` and are never overwritten with operational
/// wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScFailureRepairPackage {
    pub schema_version: u32,
    pub package_id: String,
    pub chain_id: String,
    pub session_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub terminal_class: String,
    pub terminal_reason: String,
    pub completeness: ScPackageCompleteness,
    pub rounds: Vec<ScRoundEvidence>,
    pub last_provider_output: ScEvidenceRef,
    pub last_failure_source: ScFailureSourceEvidence,
    pub teaching_prompts: Vec<ScTeachingPromptEvidence>,
    pub evidence_log_ref: String,
    pub evidence_digest: String,
    pub evidence_through_entry_key: String,
    pub missing_evidence: Vec<ScMissingEvidence>,
}

/// Receipt emitted by the strict publisher once a package is published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScArtifactPublishedReceipt {
    pub schema_version: u32,
    pub package_id: String,
    pub chain_id: String,
    pub node_id: String,
    pub package_digest: String,
    pub version: u32,
}

impl ScFailureRepairPackage {
    /// Structural validation of the frozen §2.2 invariants that serde
    /// cannot express: `schema_version`, digest hex domain, round
    /// order/cross-chain/scope binding, and completeness agreement with
    /// `missing_evidence`.
    ///
    /// Format-domain only: this never touches the evidence store and never
    /// recomputes content digests. Binding locator contents (raw JSON
    /// body, normalized source, prompt body) to their digests and byte
    /// counts belongs to the strict evidence store and publisher layers.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "schema_version: expected 1, got {}",
                self.schema_version
            ));
        }
        Self::check_digest_hex("evidence_digest", &self.evidence_digest)?;
        for (index, round) in self.rounds.iter().enumerate() {
            let at = format!("rounds[{index}]");
            if round.identity.chain_id != self.chain_id {
                return Err(format!("cross-chain round identity at {at}"));
            }
            if round.identity.workspace_session_id != self.session_id {
                return Err(format!("scope mismatch at {at}"));
            }
            if index > 0
                && round.identity.round_ordinal <= self.rounds[index - 1].identity.round_ordinal
            {
                return Err(format!("round order violated at {at}"));
            }
            Self::check_evidence_ref_hex(&round.raw, &format!("{at}.raw"))?;
            Self::check_evidence_ref_hex(&round.source, &format!("{at}.source"))?;
            for (prompt_index, prompt) in round.prompts.iter().enumerate() {
                Self::check_evidence_ref_hex(
                    &prompt.prompt,
                    &format!("{at}.prompts[{prompt_index}].prompt"),
                )?;
            }
        }
        Self::check_evidence_ref_hex(&self.last_provider_output, "last_provider_output")?;
        Self::check_evidence_ref_hex(
            &self.last_failure_source.source,
            "last_failure_source.source",
        )?;
        for (prompt_index, prompt) in self.teaching_prompts.iter().enumerate() {
            Self::check_evidence_ref_hex(
                &prompt.prompt,
                &format!("teaching_prompts[{prompt_index}].prompt"),
            )?;
        }
        match self.completeness {
            ScPackageCompleteness::Complete if !self.missing_evidence.is_empty() => {
                Err("completeness mismatch: complete package carries missing_evidence".to_string())
            }
            ScPackageCompleteness::Incomplete if self.missing_evidence.is_empty() => {
                Err("completeness mismatch: incomplete package lacks missing_evidence".to_string())
            }
            _ => Ok(()),
        }
    }

    fn check_digest_hex(path: &str, value: &str) -> Result<(), String> {
        let is_hex64 = value.len() == 64
            && value
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if is_hex64 {
            Ok(())
        } else {
            Err(format!("invalid digest hex at {path}"))
        }
    }

    fn check_evidence_ref_hex(evidence: &ScEvidenceRef, path: &str) -> Result<(), String> {
        match evidence {
            ScEvidenceRef::Available { sha256, .. } => Self::check_digest_hex(path, sha256),
            ScEvidenceRef::Missing { .. } => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN_ID: &str = "chain-sc-issue-1";
    const SESSION_ID: &str = "ws-session-7";
    const ROUND1_RAW: &str = "原始 provider 输出：compile_error: parse failed at line 3";
    const ROUND1_SOURCE: &str = "normalized author delivery source for compile round";
    const ROUND2_RAW: &str = "教学轮 provider 输出：ir_preflight rejected the repaired plan";
    const TEACHING_PROMPT: &str = "教学提示全文：修复依赖图后重新提交候选";

    fn sha256_hex(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }

    fn available_ref(locator: &str, content: &str) -> ScEvidenceRef {
        ScEvidenceRef::Available {
            locator: locator.to_string(),
            sha256: sha256_hex(content),
            bytes: content.len() as u64,
        }
    }

    fn available_parts(evidence: &ScEvidenceRef) -> (&str, &str, u64) {
        match evidence {
            ScEvidenceRef::Available {
                locator,
                sha256,
                bytes,
            } => (locator, sha256, *bytes),
            ScEvidenceRef::Missing { .. } => panic!("expected available evidence ref"),
        }
    }

    fn missing_parts(evidence: &ScEvidenceRef) -> (&str, &str, &str) {
        match evidence {
            ScEvidenceRef::Missing {
                code,
                detail,
                round_id,
            } => (code, detail, round_id),
            ScEvidenceRef::Available { .. } => panic!("expected missing evidence ref"),
        }
    }

    fn round_identity(ordinal: u32) -> ScRoundIdentity {
        ScRoundIdentity {
            chain_id: CHAIN_ID.to_string(),
            round_id: format!("round-{ordinal}"),
            node_id: format!("node-{ordinal}"),
            round_ordinal: ordinal,
            provider: ProviderName::Fake,
            workspace_session_id: SESSION_ID.to_string(),
            role_run_seq: u64::from(ordinal) + 2,
            run_ref: format!("single-candidate/runs/run-{ordinal}"),
        }
    }

    fn sent_teaching_prompt() -> ScTeachingPromptEvidence {
        ScTeachingPromptEvidence {
            failed_round_id: "round-1".to_string(),
            target_round_ordinal: 2,
            builder_id: "sc-teach-compile-v1".to_string(),
            prompt: available_ref("sc-teach-prompts/round-2.json", TEACHING_PROMPT),
            disposition: ScPromptDisposition::Sent,
        }
    }

    fn complete_package() -> ScFailureRepairPackage {
        let first = ScRoundEvidence {
            identity: round_identity(1),
            launch_state: ScRoundLaunchState::Started,
            failure_class: Some(ScFailureClass::Compile),
            blocking_reasons: vec![
                "compile_failed".to_string(),
                "plan_shape_invalid".to_string(),
            ],
            repair_instructions: vec!["fix dependency edge".to_string()],
            raw: available_ref("work-item-split-runs/issue-1/run-1/run.json", ROUND1_RAW),
            source: available_ref(
                "work-item-split-runs/issue-1/run-1/source.txt",
                ROUND1_SOURCE,
            ),
            existing_failure_id: Some("failure-101".to_string()),
            attempt_key: Some("attempt-7-3".to_string()),
            prompts: Vec::new(),
            teaching_decision: ScTeachingDecision::NotRequested,
        };
        let second = ScRoundEvidence {
            identity: round_identity(2),
            launch_state: ScRoundLaunchState::Started,
            failure_class: None,
            blocking_reasons: vec!["ir_preflight_rejected".to_string()],
            repair_instructions: Vec::new(),
            raw: available_ref("work-item-split-runs/issue-1/run-2/run.json", ROUND2_RAW),
            source: available_ref(
                "work-item-split-runs/issue-1/run-1/source.txt",
                ROUND1_SOURCE,
            ),
            existing_failure_id: None,
            attempt_key: None,
            prompts: vec![sent_teaching_prompt()],
            teaching_decision: ScTeachingDecision::Granted,
        };
        let prompt = sent_teaching_prompt();
        ScFailureRepairPackage {
            schema_version: 1,
            package_id: "pkg-chain-sc-issue-1-1".to_string(),
            chain_id: CHAIN_ID.to_string(),
            session_id: SESSION_ID.to_string(),
            project_id: "proj-1".to_string(),
            issue_id: "issue-1".to_string(),
            plan_id: "plan-1".to_string(),
            terminal_class: "compile".to_string(),
            terminal_reason: "compile failed; teaching round did not repair the candidate"
                .to_string(),
            completeness: ScPackageCompleteness::Complete,
            rounds: vec![first, second],
            last_provider_output: available_ref(
                "work-item-split-runs/issue-1/run-2/run.json",
                ROUND2_RAW,
            ),
            last_failure_source: ScFailureSourceEvidence {
                source_round_id: "round-1".to_string(),
                source: available_ref(
                    "work-item-split-runs/issue-1/run-1/source.txt",
                    ROUND1_SOURCE,
                ),
            },
            teaching_prompts: vec![prompt],
            evidence_log_ref: "sc-failure-evidence/chain-sc-issue-1".to_string(),
            evidence_digest: sha256_hex("evidence log body for chain-sc-issue-1"),
            evidence_through_entry_key: "chain-sc-issue-1/round-2/prompt/sent".to_string(),
            missing_evidence: Vec::new(),
        }
    }

    /// Package used by the wire preservation test: duplicate blocking
    /// reasons, five repair instructions, an explicitly missing teaching
    /// prompt, and incomplete completeness.
    fn wire_preservation_package() -> ScFailureRepairPackage {
        let mut package = complete_package();
        package.completeness = ScPackageCompleteness::Incomplete;
        package.terminal_class = "teaching_interrupted".to_string();
        package.terminal_reason =
            "教学中断：原始编译失败原因保留在 rounds 与 terminal_reason".to_string();
        package.rounds[0].blocking_reasons = vec![
            "compile_failed".to_string(),
            "compile_failed".to_string(),
            "plan_shape_invalid".to_string(),
        ];
        package.rounds[0].repair_instructions = vec![
            "instruction-01-tighten dependency graph".to_string(),
            "instruction-02-keep schema literal".to_string(),
            "instruction-03-re-run preflight".to_string(),
            "instruction-04-attach raw output".to_string(),
            "instruction-05-exact-text-must-survive-wire".to_string(),
        ];
        package.rounds[1].teaching_decision = ScTeachingDecision::Interrupted;
        let pruned_prompt = ScTeachingPromptEvidence {
            failed_round_id: "round-1".to_string(),
            target_round_ordinal: 2,
            builder_id: "sc-teach-compile-v1".to_string(),
            prompt: ScEvidenceRef::Missing {
                code: "prompt_pruned".to_string(),
                detail: "teaching prompt body pruned before send".to_string(),
                round_id: "round-2".to_string(),
            },
            disposition: ScPromptDisposition::Unavailable,
        };
        package.rounds[1].prompts = vec![pruned_prompt.clone()];
        package.teaching_prompts = vec![pruned_prompt];
        package.evidence_through_entry_key =
            "chain-sc-issue-1/round-2/prompt/unavailable".to_string();
        package.missing_evidence = vec![ScMissingEvidence {
            round_id: "round-2".to_string(),
            kind: "prompt".to_string(),
            code: "prompt_pruned".to_string(),
            detail: "teaching prompt body pruned before send".to_string(),
            locator: None,
        }];
        package
    }

    /// Valid incomplete package: one missing source with a known locator.
    fn incomplete_package() -> ScFailureRepairPackage {
        let mut package = complete_package();
        package.completeness = ScPackageCompleteness::Incomplete;
        package.terminal_class = "teaching_interrupted".to_string();
        package.rounds[1].source = ScEvidenceRef::Missing {
            code: "source_run_missing".to_string(),
            detail: "split run directory pruned before evidence capture".to_string(),
            round_id: "round-2".to_string(),
        };
        package.missing_evidence = vec![ScMissingEvidence {
            round_id: "round-2".to_string(),
            kind: "source".to_string(),
            code: "source_run_missing".to_string(),
            detail: "split run directory pruned before evidence capture".to_string(),
            locator: Some("work-item-split-runs/issue-1/run-2/run.json".to_string()),
        }];
        package
    }

    #[test]
    fn sc_failure_dto_evidence_ref_wire_shape_is_status_tagged() {
        // Literal wire JSON from a TS producer: pins the `status` tag and
        // field names without going through our own serializer.
        let available: ScEvidenceRef = serde_json::from_str(
            r#"{"status":"available","locator":"run.json","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","bytes":42}"#,
        )
        .unwrap();
        let (locator, sha256, bytes) = available_parts(&available);
        assert_eq!(locator, "run.json");
        assert_eq!(
            sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(bytes, 42);

        let missing: ScEvidenceRef = serde_json::from_str(
            r#"{"status":"missing","code":"source_run_missing","detail":"run dir pruned","round_id":"round-2"}"#,
        )
        .unwrap();
        let (code, detail, round_id) = missing_parts(&missing);
        assert_eq!(code, "source_run_missing");
        assert_eq!(detail, "run dir pruned");
        assert_eq!(round_id, "round-2");

        let wire = serde_json::to_string(&missing).unwrap();
        assert!(
            wire.contains(r#""status":"missing""#),
            "missing ref must keep the status tag on the wire: {wire}"
        );
    }

    fn wire_name<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }

    #[test]
    fn sc_failure_dto_enum_wire_names_are_snake_case() {
        assert_eq!(wire_name(&ScFailureClass::Compile), "\"compile\"");
        assert_eq!(wire_name(&ScFailureClass::IrPreflight), "\"ir_preflight\"");
        assert_eq!(
            wire_name(&ScFailureClass::StructureGate),
            "\"structure_gate\""
        );
        assert_eq!(
            wire_name(&ScTeachingSourceOrigin::CompileSplitOutput),
            "\"compile_split_output\""
        );
        assert_eq!(
            wire_name(&ScTeachingSourceOrigin::IrPreflightSplitOutput),
            "\"ir_preflight_split_output\""
        );
        assert_eq!(
            wire_name(&ScTeachingSourceOrigin::StructureRevision),
            "\"structure_revision\""
        );
        assert_eq!(
            wire_name(&ScRoundLaunchState::NotStarted),
            "\"not_started\""
        );
        assert_eq!(wire_name(&ScRoundLaunchState::Started), "\"started\"");
        assert_eq!(wire_name(&ScRoundLaunchState::Unknown), "\"unknown\"");
        assert_eq!(wire_name(&ScPromptDisposition::Prepared), "\"prepared\"");
        assert_eq!(wire_name(&ScPromptDisposition::Sent), "\"sent\"");
        assert_eq!(
            wire_name(&ScPromptDisposition::RejectedBudget),
            "\"rejected_budget\""
        );
        assert_eq!(
            wire_name(&ScPromptDisposition::Unavailable),
            "\"unavailable\""
        );
        assert_eq!(
            wire_name(&ScTeachingDecision::NotRequested),
            "\"not_requested\""
        );
        assert_eq!(wire_name(&ScTeachingDecision::Ineligible), "\"ineligible\"");
        assert_eq!(wire_name(&ScTeachingDecision::Denied), "\"denied\"");
        assert_eq!(wire_name(&ScTeachingDecision::Drift), "\"drift\"");
        assert_eq!(wire_name(&ScTeachingDecision::Granted), "\"granted\"");
        assert_eq!(
            wire_name(&ScTeachingDecision::Interrupted),
            "\"interrupted\""
        );
        assert_eq!(wire_name(&ScPackageCompleteness::Complete), "\"complete\"");
        assert_eq!(
            wire_name(&ScPackageCompleteness::Incomplete),
            "\"incomplete\""
        );

        // Consumer direction: decode one representative snake_case name.
        let back: ScPromptDisposition = serde_json::from_str("\"rejected_budget\"").unwrap();
        assert_eq!(back, ScPromptDisposition::RejectedBudget);
    }

    #[test]
    fn sc_failure_dto_preserves_all_rounds_and_explicit_missing() {
        let package = wire_preservation_package();
        let wire = serde_json::to_string(&package).unwrap();
        assert!(
            wire.contains(r#""status":"missing""#),
            "wire must carry the explicit missing status tag"
        );
        assert!(
            wire.contains("\"unavailable\""),
            "wire must carry the snake_case prompt disposition"
        );

        let decoded: ScFailureRepairPackage = serde_json::from_str(&wire).unwrap();
        assert_eq!(decoded.rounds.len(), 2);

        let first = &decoded.rounds[0];
        // Raw provider bytes and normalized source keep independent
        // digests; they are different bodies and must not collapse.
        let (raw_locator, raw_sha, raw_bytes) = available_parts(&first.raw);
        let (source_locator, source_sha, _) = available_parts(&first.source);
        assert_eq!(raw_locator, "work-item-split-runs/issue-1/run-1/run.json");
        assert_eq!(
            source_locator,
            "work-item-split-runs/issue-1/run-1/source.txt"
        );
        assert_eq!(raw_sha, sha256_hex(ROUND1_RAW));
        assert_eq!(source_sha, sha256_hex(ROUND1_SOURCE));
        assert_ne!(raw_sha, source_sha);
        assert_eq!(raw_bytes, ROUND1_RAW.len() as u64);

        // Duplicate blocking reasons keep both order and multiplicity.
        assert_eq!(
            first.blocking_reasons,
            vec!["compile_failed", "compile_failed", "plan_shape_invalid"]
        );
        // The fifth repair instruction survives exactly: no truncation and
        // no four-item cap is applied at the wire layer.
        assert_eq!(first.repair_instructions.len(), 5);
        assert_eq!(
            first.repair_instructions[4],
            "instruction-05-exact-text-must-survive-wire"
        );
        assert_eq!(first.teaching_decision, ScTeachingDecision::NotRequested);

        let second = &decoded.rounds[1];
        // Same split source body ⇒ same digest; the round's own raw stays
        // a distinct body.
        let (_, second_source_sha, _) = available_parts(&second.source);
        let (_, second_raw_sha, _) = available_parts(&second.raw);
        assert_eq!(second_source_sha, source_sha);
        assert_ne!(second_raw_sha, second_source_sha);
        assert_eq!(second.teaching_decision, ScTeachingDecision::Interrupted);

        // Explicit missing evidence keeps every field, including round_id.
        let prompt = &second.prompts[0];
        let (code, detail, round_id) = missing_parts(&prompt.prompt);
        assert_eq!(prompt.failed_round_id, "round-1");
        assert_eq!(prompt.target_round_ordinal, 2);
        assert_eq!(prompt.disposition, ScPromptDisposition::Unavailable);
        assert_eq!(code, "prompt_pruned");
        assert_eq!(detail, "teaching prompt body pruned before send");
        assert_eq!(round_id, "round-2");

        assert_eq!(decoded.missing_evidence.len(), 1);
        let missing = &decoded.missing_evidence[0];
        assert_eq!(missing.round_id, "round-2");
        assert_eq!(missing.kind, "prompt");
        assert_eq!(missing.code, "prompt_pruned");
        assert_eq!(missing.detail, "teaching prompt body pruned before send");
        assert!(missing.locator.is_none());

        assert_eq!(decoded.completeness, ScPackageCompleteness::Incomplete);
        assert_eq!(decoded.terminal_class, "teaching_interrupted");
        assert_eq!(
            decoded.terminal_reason,
            "教学中断：原始编译失败原因保留在 rounds 与 terminal_reason"
        );
        assert_eq!(
            decoded.evidence_digest,
            sha256_hex("evidence log body for chain-sc-issue-1")
        );

        // Full round-trip equality on top of the targeted pins above.
        assert_eq!(decoded, package);
    }

    #[test]
    fn sc_failure_dto_validate_rejects_invalid_schema_version() {
        let mut package = complete_package();
        package.schema_version = 2;
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("schema_version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_rejects_malformed_hex_digests() {
        let mut package = complete_package();
        if let ScEvidenceRef::Available { sha256, .. } = &mut package.rounds[0].raw {
            *sha256 = "A".repeat(64);
        } else {
            panic!("fixture round raw must be available");
        }
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("invalid digest hex") && error.contains("rounds[0].raw"),
            "unexpected error: {error}"
        );

        let mut package = complete_package();
        package.evidence_digest.truncate(63);
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("invalid digest hex") && error.contains("evidence_digest"),
            "unexpected error: {error}"
        );

        let mut package = complete_package();
        package.evidence_digest = "z".repeat(64);
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("invalid digest hex") && error.contains("evidence_digest"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_rejects_round_disorder() {
        let mut package = complete_package();
        package.rounds.reverse();
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("round order violated") && error.contains("rounds[1]"),
            "unexpected error: {error}"
        );

        let mut package = complete_package();
        package.rounds[1].identity.round_ordinal = 1;
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("round order violated") && error.contains("rounds[1]"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_rejects_cross_chain_round_identity() {
        let mut package = complete_package();
        package.rounds[1].identity.chain_id = "chain-other".to_string();
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("cross-chain round identity") && error.contains("rounds[1]"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_rejects_scope_mismatch() {
        let mut package = complete_package();
        package.rounds[0].identity.workspace_session_id = "ws-session-999".to_string();
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("scope mismatch") && error.contains("rounds[0]"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_rejects_complete_with_missing_evidence() {
        let mut package = complete_package();
        package.missing_evidence.push(ScMissingEvidence {
            round_id: "round-2".to_string(),
            kind: "prompt".to_string(),
            code: "prompt_pruned".to_string(),
            detail: "teaching prompt body pruned before send".to_string(),
            locator: None,
        });
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("completeness mismatch"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_rejects_incomplete_without_missing_evidence() {
        let mut package = incomplete_package();
        package.missing_evidence.clear();
        let error = package.validate().unwrap_err();
        assert!(
            error.starts_with("completeness mismatch"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_accepts_valid_complete_package() {
        let package = complete_package();
        let verdict = package.validate();
        assert!(
            verdict.is_ok(),
            "valid complete package must pass validation, got {verdict:?}"
        );
    }

    #[test]
    fn sc_failure_dto_validate_accepts_valid_incomplete_package() {
        let package = incomplete_package();
        let verdict = package.validate();
        assert!(
            verdict.is_ok(),
            "valid incomplete package with explicit missing evidence must pass validation, got {verdict:?}"
        );
    }
}
