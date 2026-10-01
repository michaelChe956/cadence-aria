impl super::WorkspaceEngine {
    /// 校验当前 session 是否可在 amendment 上下文下重开原对话门（REQ-GCE-03）。
    /// 仅当：本 session 是单候选 WorkItemPlan、已过首次 approve（Confirmed +
    /// phase Completed）、门快照仍在（预算接续 manual_repairs_remaining）、存在
    /// 指向本 session 的 Open/Applying PlanAmendmentContext、且其 group attempt
    /// 处于 AwaitingPlanAmendment。任何一项不满足都保持既有 stage 拒绝路径。
    fn probe_amendment_gate_context(&self) -> Result<Option<PlanAmendmentContext>, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.stage == super::WorkspaceStage::HumanConfirm
        {
            return Ok(None);
        }
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let record = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if record.status != WorkspaceSessionStatus::Confirmed
            || record.single_candidate_phase != Some(SingleCandidatePhase::Completed)
            || record.human_gate_snapshot.is_none()
        {
            return Ok(None);
        }
        let coding_store = CodingAttemptStore::new(store.app_paths());
        // I-1 round3（F-B）：与 workspace decision routing 的 amendment 门判别
        // 复用同一完整谓词（find_awaiting_plan_amendment_context_for_plan_session）：
        // Open/Applying context + group attempt AwaitingPlanAmendment + group
        // identity。无命中（含 attempt 已离开 AwaitingPlanAmendment 的应用窗口）
        // 保持既有 stage 拒绝路径（结构化 WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID），
        // 不上抛泛型 Err；否则 ws 层会把 7.2 之前结构化 Rejected 的同类输入映射
        // 成泛型 Error。
        let context = coding_store
            .find_awaiting_plan_amendment_context_for_plan_session(
                &record.project_id,
                &record.issue_id,
                &record.entity_id,
                &record.id,
            )
            .map_err(|error| error.to_string())?;
        Ok(context)
    }

    pub(crate) fn build_sc_manual_revision_prompt_for_turn(
        &self,
        feedback: &str,
    ) -> Result<String, String> {
        let candidate_markdown = self
            .session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            // 批准链 compile 会把 current artifact 推成非 Markdown 投影；修订基线
            // 回落到最近一个 Markdown artifact version（批准时经 SC 流程
            // update_artifact(Markdown) 持久化的候选文本，语义上正是修订基线）。
            // 版本列表完全无 Markdown 时保持既有拒绝语义不变。
            .or_else(|| {
                self.artifact_versions
                    .iter()
                    .rev()
                    .find_map(|version| version.payload.markdown())
            })
            .ok_or_else(|| {
                "HUMAN_GATE_REVISION_CANDIDATE_MISSING: current candidate markdown is required"
                    .to_string()
            })?;
        let grammar_boundary =
            crate::product::work_item_split_engine::prompts::work_item_plan_markdown_grammar();
        super::prompts::build_sc_manual_revision_prompt(
            super::prompts::ScManualRevisionPromptInput {
                candidate_markdown,
                feedback,
                grammar_boundary: &grammar_boundary,
                language_rule: super::prompts::LANGUAGE_RULE_FILE_CONTENT,
            },
        )
    }

    /// C2 Task 11（REQ-CG-03，#15）：SC 修订完整预算组装与交付（HumanGateTurn
    /// CAS 之前）。≤inline 返回原 prompt（零新增持久化，现行为）；超 inline 但
    /// 硬限内持久化完整组装 artifact（readback digest 校验）并以 ArtifactRef
    /// 传输记录组装 digest；超硬限或 artifact 不可读返回稳定码（调用方
    /// rejected，此前已落 sc-revision-blocked.json 大候选停等等待事实）。
    fn assemble_sc_revision_delivery(
        &self,
        command_id: &str,
        prompt: String,
    ) -> Result<String, (String, String)> {
        use super::prompts::ScCandidateTransport;

        let provider = self.session.author_provider.clone();
        let budget = super::prompts::sc_provider_input_budget(&provider);
        let assembled = match super::prompts::assemble_sc_revision_input(&prompt, &budget) {
            Ok(assembled) => assembled,
            Err(error) => {
                let (code, reason) = split_stable_error(&error);
                self.land_sc_revision_blocked(
                    command_id,
                    &code,
                    &error,
                    prompt.len(),
                    budget.hard_limit_bytes,
                );
                return Err((code, reason.to_string()));
            }
        };
        match assembled.transport {
            ScCandidateTransport::Inlined => Ok(prompt),
            ScCandidateTransport::OrderedChunks {
                assembly_digest, ..
            } => {
                let artifact_ref = format!(
                    "sc-revision-inputs/{}.md",
                    assembly_digest.trim_start_matches("sha256:")
                );
                if let Err(io_error) =
                    self.persist_sc_revision_artifact(&artifact_ref, &prompt, &assembly_digest)
                {
                    let code = "HUMAN_GATE_REVISION_INPUT_ARTIFACT_UNREADABLE".to_string();
                    let reason = format!("persisted assembly artifact is unreadable: {io_error}");
                    self.land_sc_revision_blocked(
                        command_id,
                        &code,
                        &reason,
                        prompt.len(),
                        budget.hard_limit_bytes,
                    );
                    return Err((code, reason));
                }
                self.land_sc_revision_assembly(
                    command_id,
                    assembled.total_bytes,
                    ScCandidateTransport::ArtifactRef {
                        artifact_ref,
                        assembly_digest,
                    },
                );
                Ok(prompt)
            }
            ScCandidateTransport::ArtifactRef { .. } => Ok(prompt),
        }
    }

    /// session 分区目录（`workspace-sessions/{session_id}/`，与
    /// human-gate-turns 同层）。
    fn sc_revision_partition_root(&self) -> Result<std::path::PathBuf, String> {
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        Ok(store
            .app_paths()
            .issue_root(&self.session.project_id, &self.session.issue_id)
            .join("workspace-sessions")
            .join(&self.session.session_id))
    }

    /// 持久化完整组装 artifact 并 readback 校验（写失败/读回缺失/digest 不一致
    /// 均 fail-closed——交付不降级、不猜测）。
    fn persist_sc_revision_artifact(
        &self,
        artifact_ref: &str,
        prompt: &str,
        assembly_digest: &str,
    ) -> Result<(), String> {
        let path = self
            .sc_revision_partition_root()?
            .join(artifact_ref);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, prompt).map_err(|error| error.to_string())?;
        let readback = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        if readback != prompt
            || super::prompts::assembly_digest_of(&readback) != assembly_digest
        {
            return Err("readback digest mismatch".to_string());
        }
        Ok(())
    }

    /// 组装记录（"组装 digest 记录"；Task 12 消费）。inline 交付不落账
    /// （零新增持久化，现行为零回归）。
    fn land_sc_revision_assembly(
        &self,
        command_id: &str,
        total_bytes: usize,
        transport: super::prompts::ScCandidateTransport,
    ) {
        let record = ScRevisionAssemblyRecord {
            session_id: self.session.session_id.clone(),
            command_id: command_id.to_string(),
            total_bytes,
            transport,
            created_at: Utc::now().to_rfc3339(),
        };
        let path = match self.sc_revision_partition_root() {
            Ok(root) => root.join(SC_REVISION_ASSEMBLY_FILE),
            Err(error) => {
                tracing::warn!(%error, "sc revision assembly record path unavailable");
                return;
            }
        };
        if let Err(error) =
            crate::product::json_store::write_json(&path, &record)
        {
            tracing::warn!(%error, "sc revision assembly record write failed");
        }
    }

    /// 大候选停等等待事实（Task 12 投影 kind `large_candidate_blocked`）。
    /// 携带"分段返修／重试"操作；成功开回合后由 land_sc_revision_assembly
    /// 分区的新事实覆盖等待语义（blocked 文件删除见 clear）。
    fn land_sc_revision_blocked(
        &self,
        command_id: &str,
        reason_code: &str,
        detail: &str,
        total_bytes: usize,
        hard_limit_bytes: usize,
    ) {
        let record = ScRevisionBlockedRecord {
            session_id: self.session.session_id.clone(),
            command_id: command_id.to_string(),
            reason_code: reason_code.to_string(),
            detail: detail.to_string(),
            total_bytes,
            hard_limit_bytes,
            actions: vec!["segmented_revision".to_string(), "retry".to_string()],
            created_at: Utc::now().to_rfc3339(),
        };
        let path = match self.sc_revision_partition_root() {
            Ok(root) => root.join(SC_REVISION_BLOCKED_FILE),
            Err(error) => {
                tracing::warn!(%error, "sc revision blocked record path unavailable");
                return;
            }
        };
        if let Err(error) =
            crate::product::json_store::write_json(&path, &record)
        {
            tracing::warn!(%error, "sc revision blocked record write failed");
        }
    }

    /// 成功开回合清除停等等待事实（等待项闭合）。
    fn clear_sc_revision_blocked(&self) {
        let Ok(root) = self.sc_revision_partition_root() else {
            return;
        };
        let path = root.join(SC_REVISION_BLOCKED_FILE);
        if path.exists() && let Err(error) = std::fs::remove_file(&path) {
            tracing::warn!(%error, "remove sc revision blocked record failed");
        }
    }
}

/// C2 Task 12（REQ-CG-03 收口）：只读读取 session 分区的大候选停等等待
/// 事实（`large_candidate_blocked` 投影数据源）。无文件返回 `None`；
/// 文件存在但不可读按 Io 错误上抛（不吞为无事实）。
pub(crate) fn read_sc_revision_blocked_fact(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<Option<ScRevisionBlockedRecord>, ProductStoreError> {
    let path = paths
        .issue_lifecycle_root(project_id, issue_id)
        .join("workspace-sessions")
        .join(session_id)
        .join(SC_REVISION_BLOCKED_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    crate::product::json_store::read_json(&path).map(Some)
}

pub(crate) fn trim_provider_preamble(source: &str) -> &str {
    let document_heading = format!("{}\n", grammar::DOCUMENT_HEADING);
    source
        .find(&document_heading)
        .map(|offset| &source[offset..])
        .unwrap_or(source)
}

/// SC 修订交付进入 compiler 前的确定性净化（结构标题归一化 + EARS 关键字空白
/// 归一化 + 前言修剪）。
///
/// 与 author 路径共用 [`crate::product::work_item_plan_compiler::normalize_delivery_before_compile`]
/// 同一单点实现；归一化先于修剪（前言锚定规范英文标题），表外未知标题不改，
/// 由 compiler fail-closed。
pub(crate) fn prepare_revision_delivery_for_compile(
    raw: &str,
) -> crate::product::work_item_plan_compiler::NormalizedPlanSource {
    let normalized =
        crate::product::work_item_plan_compiler::normalize_delivery_before_compile(raw);
    crate::product::work_item_plan_compiler::NormalizedPlanSource {
        source: trim_provider_preamble(&normalized.source).to_string(),
        normalized_heading_lines: normalized.normalized_heading_lines,
        normalized_ears_lines: normalized.normalized_ears_lines,
    }
}
