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
        let candidate_markdown = self.current_revision_candidate_markdown().ok_or_else(|| {
            "HUMAN_GATE_REVISION_CANDIDATE_MISSING: current candidate markdown is required"
                .to_string()
        })?;
        let grammar_boundary =
            crate::product::work_item_split_engine::prompts::work_item_plan_markdown_grammar();
        let teaching = self.candidate_compile_failure_author_teaching();
        super::prompts::build_sc_manual_revision_prompt(
            super::prompts::ScManualRevisionPromptInput {
                candidate_markdown: candidate_markdown.as_str(),
                feedback,
                grammar_boundary: &grammar_boundary,
                language_rule: super::prompts::LANGUAGE_RULE_FILE_CONTENT,
                compile_failure_teaching: teaching.as_deref(),
            },
        )
    }

    /// 当前修订基线候选：session artifact 优先；批准链 compile 会把 current
    /// artifact 推成非 Markdown 投影，此时回落到最近一个 Markdown artifact
    /// version（批准时经 SC 流程 update_artifact(Markdown) 持久化的候选文本，
    /// 语义上正是修订基线）。版本列表完全无 Markdown 时返回 None（调用方
    /// 保持既有拒绝语义不变）。
    fn current_revision_candidate_markdown(&self) -> Option<String> {
        self.session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .or_else(|| {
                self.artifact_versions
                    .iter()
                    .rev()
                    .find_map(|version| version.payload.markdown())
            })
            .map(str::to_string)
    }

    /// SG5.3（plan v1.1 Task SG5.3）：SC 修订 resume 的冻结修复子 prompt
    /// delivery 与 fresh builder 分支——有结构 state（活动 grant）时
    /// delivery 读取冻结 ref/digest 精确 join 交付冻结全文，不以复造的原
    /// turn prompt 覆盖当前 instruction；无结构 state（无活动 grant）时原
    /// fresh builder 分支逐字段保持（B2d 同 turn 复验语义）。
    ///
    /// 分支判据是 durable 聚合状态（engine 冻结权威），不是调用方参数：
    /// - 活动 grant：必须携冻结件并逐身份精确 join——grant.prompt_ref/
    ///   prompt_digest == frozen 同名身份（SG4a claim 绑定）、frozen 自身
    ///   经 `adjudicate_provider_budget` fail-closed（缺 ref／digest 不绑定
    ///   内容／超硬限）、当前候选 hash == grant.failing_source_hash（claim
    ///   绑定的失败候选＝合法中间 hash，允许不同于原 turn.source_hash，
    ///   不借旧 turn 权限修订漂移候选）。全部 join 失败都在任何 turn
    ///   CAS／组装落账之前以稳定码拒绝（零 delivery、零 provider start）；
    ///   通过后复用 `assemble_sc_revision_delivery` 现有运输路径交付冻结
    ///   全文（复用 build_sc_manual_revision_prompt 路径，无新 run kind）。
    /// - 无活动 grant（含无聚合状态）：原 fresh builder 逐字段不变。
    /// - 活动 grant 绑定其它 turn/command：fail-closed 拒绝，不回落原
    ///   builder（防复造原 turn prompt 覆盖另一授权窗口的当前 instruction）。
    ///
    /// 尚未接线生产 caller（SG6a.2 handler／SG6b recovery 消费）：本包只
    /// 落 delivery 接口，不发布自动能力。
    #[allow(dead_code)]
    pub(crate) fn build_sc_revision_delivery_for_resume(
        &self,
        command_id: &str,
        turn: &crate::product::models::HumanGateTurn,
        frozen: Option<&super::prompts::FrozenStructuralPrompt>,
    ) -> Result<String, String> {
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let record = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let Some(grant) = record
            .structural_governance_state
            .as_ref()
            .and_then(|state| state.current_grant.as_ref())
        else {
            // 无结构授权窗口：原 fresh builder 分支（逐字段不变）。
            return self.build_sc_manual_revision_prompt_for_fresh_turn(turn);
        };

        // 活动 grant 面的精确 join（全部先于任何 turn CAS／组装落账）。
        match grant.turn_id.as_deref() {
            Some(grant_turn_id) if grant_turn_id == turn.turn_id => {}
            Some(grant_turn_id) => {
                return Err(format!(
                    "STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH: structural grant is bound to turn {grant_turn_id} but resume carries turn {}; refusing to deliver a revision prompt across an unjoined grant",
                    turn.turn_id
                ));
            }
            None => {
                return Err(
                    "STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH: active structural grant has no bound turn; refusing to deliver a revision prompt across an unjoined grant"
                        .to_string(),
                );
            }
        }
        if let Some(grant_command_id) = grant.command_id.as_deref()
            && grant_command_id != turn.command_id
        {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH: structural grant is bound to command {grant_command_id} but resume carries command {}; refusing to deliver a revision prompt across an unjoined grant",
                turn.command_id
            ));
        }
        let Some(frozen) = frozen else {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH: grant binds prompt ref {} / digest {} but no frozen structural prompt was supplied; delivery must read the frozen identity, not reassemble the original turn prompt",
                grant.prompt_ref.as_deref().unwrap_or("<missing>"),
                grant.prompt_digest.as_deref().unwrap_or("<missing>"),
            ));
        };
        // provider 预算 fail-closed（SG5.2 裁决：缺 ref／digest 不绑定内容／
        // 超硬限——Operational，不驱 author、零 delivery/start）。
        let budget = super::prompts::sc_provider_input_budget(&self.session.author_provider);
        if let Err(rejection) = frozen.adjudicate_provider_budget(&budget) {
            return Err(render_structural_prompt_rejection(&rejection));
        }
        if grant.prompt_ref.as_deref() != Some(frozen.prompt_ref.as_str())
            || grant.prompt_digest.as_deref() != Some(frozen.prompt_digest.as_str())
        {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH: grant binds prompt ref {} / digest {} but frozen prompt carries ref {} / digest {}; refusing to deliver across an unbound prompt identity",
                grant.prompt_ref.as_deref().unwrap_or("<missing>"),
                grant.prompt_digest.as_deref().unwrap_or("<missing>"),
                frozen.prompt_ref,
                frozen.prompt_digest,
            ));
        }

        // 候选精确 join：claim 绑定的失败候选＝合法中间 hash（允许不同于
        // 原 turn.source_hash）；grant 未绑定（旧记录）时回落原
        // turn.source_hash 口径。空 hash（legacy 记录）不可判定，同码
        // fail-closed（先例 B2d）。
        use sha2::Digest;
        let candidate_markdown = self.current_revision_candidate_markdown().ok_or_else(|| {
            "HUMAN_GATE_REVISION_CANDIDATE_MISSING: current candidate markdown is required"
                .to_string()
        })?;
        let current_hash = hex::encode(sha2::Sha256::digest(candidate_markdown.as_bytes()));
        let expected_hash = grant
            .failing_source_hash
            .as_deref()
            .filter(|hash| !hash.is_empty())
            .unwrap_or(turn.source_hash.as_str());
        if expected_hash.is_empty()
            || current_hash != expected_hash
            || !frozen.full_prompt.contains(&candidate_markdown)
        {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_CANDIDATE_DRIFTED: grant binds failing source sha256:{expected_hash} but the current candidate is sha256:{current_hash}; the frozen prompt no longer matches its business candidate"
            ));
        }

        // 交付冻结全文：复用现完整组装/传输路径（inline/分块 artifact 落
        // 账口径与 C2 Task 11 一致，无新 run kind）。
        match self.assemble_sc_revision_delivery(command_id, frozen.full_prompt.clone()) {
            Ok(delivered) => Ok(delivered),
            Err((code, reason)) => Err(format!("{code}: {reason}")),
        }
    }

    /// SG6b（plan v1.1 Task SG6b / REQ-SG-05「显式 fresh 同 attempt
    /// None-resume/子 prompt」）：claim 绑定的冻结机器指令子 prompt 的
    /// 内容寻址持久化——session 分区 `structural-prompts/<digest>.json`
    ///（幂等：同 digest 覆写同字节）。跨 spawn 的显式 fresh 重入据此读回
    /// 冻结全文并以 grant.prompt_ref/prompt_digest 精确 join，不依赖
    /// in-flight run 的内存携带。持久化失败上抛（claim 前调用方停等，
    /// 不留无冻结证据的 claim）。
    pub(crate) fn persist_frozen_structural_prompt(
        paths: &crate::product::app_paths::ProductAppPaths,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        frozen: &super::prompts::FrozenStructuralPrompt,
    ) -> Result<(), String> {
        let digest_hex = frozen
            .prompt_digest
            .trim_start_matches("sha256:")
            .to_string();
        if digest_hex.is_empty() {
            return Err(
                "STRUCTURAL_PROMPT_PERSIST_REJECTED: frozen prompt carries no digest".to_string(),
            );
        }
        let dir = paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-sessions")
            .join(session_id)
            .join("structural-prompts");
        std::fs::create_dir_all(&dir).map_err(|error| format!("{error:?}"))?;
        crate::product::json_store::write_json(&dir.join(format!("{digest_hex}.json")), frozen)
            .map_err(|error| format!("{error:?}"))
    }

    /// 上一函数的读回（内容寻址）；文件缺失/不可读/身份不符一律 Err（调用方
    /// 以稳定码拒绝，0 start、预算不改写，不臆测冻结内容）。
    pub(crate) fn load_frozen_structural_prompt(
        paths: &crate::product::app_paths::ProductAppPaths,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        prompt_ref: &str,
    ) -> Result<super::prompts::FrozenStructuralPrompt, String> {
        let digest_hex = prompt_ref
            .trim_start_matches("structural-prompt://")
            .to_string();
        if digest_hex.is_empty() || digest_hex.len() != 64 {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH: frozen prompt ref {prompt_ref} is not a content-addressed identity"
            ));
        }
        let path = paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-sessions")
            .join(session_id)
            .join("structural-prompts")
            .join(format!("{digest_hex}.json"));
        let frozen: super::prompts::FrozenStructuralPrompt =
        crate::product::json_store::read_json(&path).map_err(|error| {
            format!(
                "STRUCTURAL_REVISION_DELIVERY_CANDIDATE_DRIFTED: frozen prompt {prompt_ref} is not readable from the durable store ({error:?})"
            )
        })?;
        let expected_ref = format!("structural-prompt://{digest_hex}");
        if frozen.prompt_ref != expected_ref {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH: persisted frozen prompt carries ref {} but was addressed as {prompt_ref}",
                frozen.prompt_ref
            ));
        }
        Ok(frozen)
    }

    /// B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：显式 fresh 的同
    /// turn prompt 重装——重读同 turn（feedback 全量）并对 candidate hash
    /// 复验（turn.source_hash 与当前候选 markdown 不一致＝业务漂移，稳定
    /// 码 `HUMAN_GATE_REVISION_CANDIDATE_DRIFTED` 拒绝，不以旧 turn 权限
    /// 强行修订漂移后的候选；空 source_hash 的 legacy 记录不可判定，同码
    /// fail-closed）；复验通过仍走现完整 SC manual prompt 装配（原预算/
    /// transport 契约不变，零新持久化）。
    ///
    /// SG6b（REQ-SG-05「显式 fresh 同 attempt None-resume/子 prompt」）：
    /// 活动 grant 绑定本 turn 且 phase=Attempt2Claimed 时，fresh 重入驱动
    /// 的就是 claim 绑定的冻结机器指令子 prompt——从 durable 冻结件读回
    /// 并以 grant 身份精确 join 后交付冻结全文（resume=None 由 fresh 输入
    /// 组装结构保证），不得复造原反馈 prompt 覆盖当前 instruction
    ///（REQ-CG-03 机器指令驱动）；身份/候选漂移＝稳定码拒绝（0 start、
    /// 预算不改写）。其余形态（无 grant/未 claim/绑其它 turn 之外的合法
    /// 相位）保持下方原 fresh 路径逐字段不变；绑定其它 turn 的活动 grant
    /// fail-closed 拒绝（不跨授权窗口交付）。
    pub(crate) fn build_sc_manual_revision_prompt_for_fresh_turn(
        &self,
        turn: &crate::product::models::HumanGateTurn,
    ) -> Result<String, String> {
        use crate::product::models::workspace::StructuralGrantPhase;

        // SG6b：结构授权窗口 join（先于任何组装/落账）。
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let record = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        match record
            .structural_governance_state
            .as_ref()
            .and_then(|state| state.current_grant.as_ref())
        {
            Some(grant) => {
                match grant.turn_id.as_deref() {
                    Some(grant_turn_id) if grant_turn_id == turn.turn_id => {}
                    Some(grant_turn_id) => {
                        return Err(format!(
                            "STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH: structural grant is bound to turn {grant_turn_id} but fresh re-entry carries turn {}; refusing to deliver a revision prompt across an unjoined grant",
                            turn.turn_id
                        ));
                    }
                    None => {
                        return Err(
                        "STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH: active structural grant has no bound turn; refusing to deliver a revision prompt across an unjoined grant"
                            .to_string(),
                    );
                    }
                }
                if grant.phase == Some(StructuralGrantPhase::Attempt2Claimed) {
                    return self
                        .deliver_claimed_frozen_prompt_for_fresh_recovery(&record, turn, grant);
                }
                // 其余相位（未 claim 的 AuthorRunning/Assessing/…/已收口终
                // 态）无冻结子 prompt 通道——原 fresh 路径。
            }
            None => {
                // 无结构授权窗口：原 fresh 分支（逐字段不变）。
            }
        }
        self.build_sc_manual_revision_prompt_for_fresh_turn_original(turn)
    }

    /// SG6b：原 fresh 路径（SG6b 结构窗口分支拆出后逐字段保持）。
    fn build_sc_manual_revision_prompt_for_fresh_turn_original(
        &self,
        turn: &crate::product::models::HumanGateTurn,
    ) -> Result<String, String> {
        use sha2::Digest;

        let candidate_markdown = self.current_revision_candidate_markdown().ok_or_else(|| {
            "HUMAN_GATE_REVISION_CANDIDATE_MISSING: current candidate markdown is required"
                .to_string()
        })?;
        if turn.source_hash.is_empty() {
            return Err(
            "HUMAN_GATE_REVISION_CANDIDATE_DRIFTED: turn carries no candidate hash (legacy record); cannot re-assemble a fresh revision prompt"
                .to_string(),
        );
        }
        let current_hash = hex::encode(sha2::Sha256::digest(candidate_markdown.as_bytes()));
        if current_hash != turn.source_hash {
            return Err(format!(
                "HUMAN_GATE_REVISION_CANDIDATE_DRIFTED: turn was reserved for candidate sha256:{} but the current candidate is sha256:{current_hash}; the turn no longer matches its business candidate",
                turn.source_hash
            ));
        }
        self.build_sc_manual_revision_prompt_for_turn(&turn.feedback_text)
    }

    /// SG6b：Attempt2Claimed 窗口的显式 fresh 冻结子 prompt 交付——durable
    /// 冻结件读回＋grant 身份/命令/失败候选证据全量 join（全部先于任何
    /// turn CAS/组装落账，稳定码拒绝）。
    fn deliver_claimed_frozen_prompt_for_fresh_recovery(
        &self,
        record: &crate::product::models::WorkspaceSessionRecord,
        turn: &crate::product::models::HumanGateTurn,
        grant: &crate::product::models::workspace::StructuralRevisionGrant,
    ) -> Result<String, String> {
        let (Some(prompt_ref), Some(prompt_digest)) = (
            grant
                .prompt_ref
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            grant
                .prompt_digest
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
        ) else {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH: claimed grant binds prompt ref {} / digest {}; the frozen sub-prompt identity is incomplete",
                grant.prompt_ref.as_deref().unwrap_or("<missing>"),
                grant.prompt_digest.as_deref().unwrap_or("<missing>"),
            ));
        };
        if let Some(grant_command_id) = grant.command_id.as_deref()
            && grant_command_id != turn.command_id
        {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH: structural grant is bound to command {grant_command_id} but fresh re-entry carries command {}",
                turn.command_id
            ));
        }
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let frozen = Self::load_frozen_structural_prompt(
            &store.app_paths(),
            &record.project_id,
            &record.issue_id,
            &record.id,
            prompt_ref,
        )?;
        // provider 预算 fail-closed（SG5.2 裁决同款）。
        let budget = super::prompts::sc_provider_input_budget(&self.session.author_provider);
        if let Err(rejection) = frozen.adjudicate_provider_budget(&budget) {
            return Err(render_structural_prompt_rejection(&rejection));
        }
        if frozen.prompt_digest != prompt_digest {
            return Err(format!(
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH: grant binds prompt ref {prompt_ref} / digest {prompt_digest} but the persisted frozen prompt carries digest {}; refusing to deliver across an unbound prompt identity",
                frozen.prompt_digest
            ));
        }
        // claim 绑定的失败候选证据包含校验（SG5.3⑧4：候选逐字节包含不可
        // 放松；fresh 恢复按 durable 证据对账，不读内存投影）。
        if let Some(failed_hash) = grant
            .failing_source_hash
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: record.project_id.clone(),
                issue_id: record.issue_id.clone(),
                plan_id: record.entity_id.clone(),
            };
            let source_store =
                crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
                    store.app_paths(),
                );
            let evidence_ref = format!(
                "project/{}/issue/{}/plan/{}/source_revision/source-{}",
                record.project_id,
                record.issue_id,
                record.entity_id,
                &failed_hash[..failed_hash.len().min(16)]
            );
            let evidence = source_store
            .get_source_revision(&scope, &evidence_ref)
            .map_err(|error| {
                format!(
                    "STRUCTURAL_REVISION_DELIVERY_CANDIDATE_DRIFTED: claimed failure candidate sha256:{failed_hash} evidence is unreadable ({error:?})"
                )
            })?;
            if !frozen.full_prompt.contains(&evidence.source) {
                return Err(format!(
                    "STRUCTURAL_REVISION_DELIVERY_CANDIDATE_DRIFTED: frozen prompt no longer embeds the claimed failure candidate sha256:{failed_hash}"
                ));
            }
        }
        // 交付冻结全文：复用现完整组装/传输路径（无新 run kind）。
        match self.assemble_sc_revision_delivery(&turn.command_id, frozen.full_prompt.clone()) {
            Ok(delivered) => Ok(delivered),
            Err((code, reason)) => Err(format!("{code}: {reason}")),
        }
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
        let path = self.sc_revision_partition_root()?.join(artifact_ref);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, prompt).map_err(|error| error.to_string())?;
        let readback = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        if readback != prompt || super::prompts::assembly_digest_of(&readback) != assembly_digest {
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
        if let Err(error) = crate::product::json_store::write_json(&path, &record) {
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
        if let Err(error) = crate::product::json_store::write_json(&path, &record) {
            tracing::warn!(%error, "sc revision blocked record write failed");
        }
    }

    /// 成功开回合清除停等等待事实（等待项闭合）。
    fn clear_sc_revision_blocked(&self) {
        let Ok(root) = self.sc_revision_partition_root() else {
            return;
        };
        let path = root.join(SC_REVISION_BLOCKED_FILE);
        if path.exists()
            && let Err(error) = std::fs::remove_file(&path)
        {
            tracing::warn!(%error, "remove sc revision blocked record failed");
        }
    }
}

/// SG5.3：SG5.2 `adjudicate_provider_budget` 拒绝（单 Operational 失败）→
/// "稳定码: cause" 字符串（与本文件既有错误口径一致；无稳定码时
/// fail-closed 泛化拒收，不吞错）。
fn render_structural_prompt_rejection(
    set: &crate::product::planning_failure::PlanningFailureSet,
) -> String {
    use crate::product::planning_failure::{PlanningFailure, PlanningFailureKind};
    match set.failures.first() {
        Some(PlanningFailure {
            kind: PlanningFailureKind::Operational { stable_code, cause },
            ..
        }) => format!("{stable_code}: {cause}"),
        _ => {
            "STRUCTURAL_REVISION_PROMPT_REJECTED: adjudication returned no stable operational code"
                .to_string()
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
