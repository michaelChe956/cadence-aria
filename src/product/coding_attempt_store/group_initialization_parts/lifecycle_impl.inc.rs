impl super::CodingAttemptStore {
    pub fn acquire_group_initialization_arbitration(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CodingGroupInitializationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(CodingGroupInitializationGuard {
            _lock: ExclusiveFileLock::acquire(
                &self.group_initialization_arbitration_path(project_id, issue_id),
            )?,
        })
    }

    pub async fn acquire_group_initialization_arbitration_async(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CodingGroupInitializationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(CodingGroupInitializationGuard {
            _lock: ExclusiveFileLock::acquire_async(
                &self.group_initialization_arbitration_path(project_id, issue_id),
            )
            .await?,
        })
    }

    pub fn prepare_group_initialization(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        self.prepare_group_initialization_with_admission(
            input,
            bound_plan_revision_id,
            unit_bindings,
            CodingAdmissionKind::LegacyGroup,
        )
    }

    pub fn prepare_group_initialization_with_admission(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        self.prepare_group_initialization_with_admission_for_target(
            input,
            bound_plan_revision_id,
            unit_bindings,
            admission_kind,
            None,
        )
    }

    /// REQ-MTG-02（OQ2 journal 路径规则，WP2 分流创建消费）：多 target 拆分的
    /// per-target journal 变体。`journal_target=Some(t)` 时 journal 落
    /// `group-initializations/{plan_id}/{t}.json` 子目录（单 target 保持原路径
    /// `{plan_id}.json` 零迁移——本函数对既有调用方（`None`）行为零变化）。
    /// attempt 级前置（唯一性/单 active）按输入快照 target 桶细化，与存储路径
    /// 键无关。
    ///
    /// N1 依赖同源（REQ-CS-03/08）：`journal_target=Some(t)` 且 ScAdvance 时
    /// `unit_bindings` 期望是**权威 plan 全 units**——先全图校验（真 unknown/
    /// 重复/self/全局环失败关闭），再仅按桶内边排序 `t` 桶，journal units 保留
    /// 全部原 depends（外部 depends 不删，plan gate 消费）。`None` 保持全集
    /// 拓扑序旧语义。
    pub fn prepare_group_initialization_with_admission_for_target(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        // D2.1c：journal 准备/重放入口持 owning Plan scope 共享准入（含读
        // 重放命中路径——删除窗口内的 journal 恢复不得继续走到 ensure）。
        // advance 编排在 arbitration 锁内调用时本 try 准入仍 fail-closed
        // 拒墓碑（D2.1d 将在 advance 入口外层持同 Plan lease）。
        self.with_plan_live_write(&input.project_id, &input.issue_id, &input.plan_id, || {
            self.prepare_group_initialization_with_admission_for_target_inner(
                input,
                bound_plan_revision_id,
                unit_bindings,
                admission_kind,
                journal_target,
            )
        })
    }

    fn prepare_group_initialization_with_admission_for_target_inner(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        let routing =
            // C4 Task 2：issue 身份先经唯一 authority resolver 冻结（conflict
            // fail-closed）；无 LC 归属的旧数据一律 unbound_issue()（fail-closed）。
            match crate::product::logical_codebase::RepositoryAuthorityResolver::new(
                self.paths.clone(),
            )
            .resolve_for_issue(&input.project_id, &input.issue_id)?
            {
                None => RepositoryRouting::unbound_issue(),
                Some(resolution) => RepositoryRouting::classify(
                    resolution.manifest,
                    resolution.selection,
                ),
            };
        let ordered_unit_bindings = if admission_kind == CodingAdmissionKind::ScAdvance {
            match journal_target {
                // N1：全图校验 → 桶内边排序，外部 depends 保留。
                Some(target) => topologically_order_target_unit_bindings(unit_bindings, target)?,
                None => topologically_order_unit_bindings(unit_bindings)?,
            }
        } else {
            unit_bindings.to_vec()
        };
        validate_group_initialization_input(
            input,
            bound_plan_revision_id,
            &ordered_unit_bindings,
            &routing,
        )?;
        let path = match journal_target {
            None => self.group_initialization_journal_path(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
            ),
            Some(target) => self.group_initialization_journal_path_for_target(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
                &target,
            ),
        };
        if super::path_is_regular_file(&path)? {
            let journal: CodingGroupInitializationJournal = read_json(&path)?;
            validate_group_initialization_journal(&journal)?;
            if journal_matches_request(
                &journal,
                input,
                bound_plan_revision_id,
                &ordered_unit_bindings,
                admission_kind,
            ) {
                return Ok(journal);
            }
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "initialization journal identity differs from the authoritative request",
            ));
        }

        let input_target = super::CodingAttemptStore::attempt_target_bucket_from_input(input);
        // REQ-MTG-02（2.2 唯一性细化，D2.1 A1）：per-(plan,target) 前置——无快照
        // attempt 不参与唯一性判定（不阻塞 target-attempt 的 journal 准备）。
        if let Some(existing) = self.get_attempt_for_work_item_group(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
            input_target,
        )? {
            return Err(incomplete_group_attempt(
                &existing.id,
                "group attempt exists without an initialization journal",
            ));
        }
        let existing_attempts: Vec<CodingExecutionAttempt> = super::list_json_records(
            &self.coding_attempts_root(&input.project_id, &input.issue_id),
        )?;
        // REQ-MTG-02（2.2 单 active 细化，D2.1 A4）：per-(issue,target)——无快照
        // active attempt 不计入任何桶。
        if let Some(active) = existing_attempts.into_iter().find(|attempt| {
            attempt.status.is_active()
                && super::CodingAttemptStore::attempt_target_bucket(attempt) == input_target
        }) {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

        let journal = self.build_group_initialization_journal(
            input,
            bound_plan_revision_id,
            &ordered_unit_bindings,
            admission_kind,
            journal_target,
        )?;
        write_json(&path, &journal)?;
        Ok(journal)
    }

    pub fn get_group_initialization(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let path = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_group_initialization_journal",
                id: plan_id.to_string(),
            });
        }
        let journal: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&journal)?;
        if journal.project_id != project_id
            || journal.issue_id != issue_id
            || journal.plan_id != plan_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_group_initialization_journal",
                id: plan_id.to_string(),
            });
        }
        Ok(journal)
    }

    /// REQ-MTG-02（OQ2 读取规则，WP2）：plan 的全部初始化 journal——先试原路径
    /// （存量单 target journal 零迁移），原路径不存在再列 `{plan_id}/` 子目录
    /// （按文件名=target UUID 升序，确定性）。
    pub fn list_group_initialization_journals_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Vec<CodingGroupInitializationJournal>, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let original = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if super::path_is_regular_file(&original)? {
            return Ok(vec![
                self.get_group_initialization(project_id, issue_id, plan_id)?,
            ]);
        }
        let directory = self
            .coding_attempts_root(project_id, issue_id)
            .join("group-initializations")
            .join(plan_id);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    directory.display()
                )));
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read {} entry: {error}", directory.display()))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort();
        let mut journals = Vec::with_capacity(paths.len());
        for path in paths {
            let journal: CodingGroupInitializationJournal = read_json(&path)?;
            validate_group_initialization_journal(&journal)?;
            if journal.project_id != project_id
                || journal.issue_id != issue_id
                || journal.plan_id != plan_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_journal",
                    id: plan_id.to_string(),
                });
            }
            journals.push(journal);
        }
        Ok(journals)
    }

    pub fn ensure_group_initialization_attempt(
        &self,
        journal: &CodingGroupInitializationJournal,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        guard.validate_identity(
            self,
            &journal.project_id,
            &journal.issue_id,
            &journal.lock_work_item_id,
        )?;
        // D2.1c：attempt 持久化消费 guard 已持的 owning Plan 共享准入
        //（scope 在 creation 锁外层取得，不嵌套重取）；guard 无归属
        //（legacy lock WI）或归属与 journal plan 不一致时，以 journal.plan_id
        // 自身的准入兜底——purging/deleted 墓碑后的 journal 恢复创建稳定
        // 拒绝（REQ-DEL-05：不能检查后另一入口再创建）。
        let guard_admits_journal_plan = guard.admitted_plan_scope().is_some_and(|scope| {
            scope.kind == crate::product::entity_deletion_store::DeletionEntityKind::Plan
                && scope.project_id == journal.project_id
                && scope.issue_id == journal.issue_id
                && scope.entity_id == journal.plan_id
        });
        if guard_admits_journal_plan {
            self.ensure_group_initialization_attempt_inner(journal)
        } else {
            self.with_plan_live_write(
                &journal.project_id,
                &journal.issue_id,
                &journal.plan_id,
                || self.ensure_group_initialization_attempt_inner(journal),
            )
        }
    }

    fn ensure_group_initialization_attempt_inner(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        // REQ-MTG-02（2.2 异 target 并行解禁）：active 检查按 target 桶细化——
        // 本 journal 的桶内必须恰为自身；异 target active（D2.1 A4 无快照亦同）
        // 不阻塞。
        let journal_target = Self::attempt_target_bucket(&journal.attempt);
        let active_attempts = super::list_json_records::<CodingExecutionAttempt>(
            &self.coding_attempts_root(&journal.project_id, &journal.issue_id),
        )?
        .into_iter()
        .filter(|attempt| {
            attempt.status.is_active() && Self::attempt_target_bucket(attempt) == journal_target
        })
        .collect::<Vec<_>>();
        let attempt_path =
            self.attempt_path(&journal.project_id, &journal.issue_id, &journal.attempt.id);
        if super::path_is_regular_file(&attempt_path)? {
            let existing: CodingExecutionAttempt = read_json(&attempt_path)?;
            if existing != journal.attempt {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "persisted attempt differs from initialization journal",
                ));
            }
            if active_attempts.len() != 1 || active_attempts[0].id != journal.attempt.id {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "another active attempt exists during initialization replay",
                ));
            }
        } else {
            // H2b（设计 A 节「journal 在而 attempts 亡」）：已越
            // AttemptPersisted 的 journal，其 attempt 曾被耐久物化过——
            // 此时缺失＝已被删除，不得用旧 snapshot 重建（复活旧 attempt
            // ID）；未达 AttemptPersisted 的缺失属既有初始化中断前缀，
            // 保持首物化续做，原协议零变化。
            if journal
                .phase
                .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
            {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "persisted attempt is missing beyond AttemptPersisted; the old attempt id must not be revived",
                ));
            }
            if let Some(active) = active_attempts.first() {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    &format!(
                        "active attempt {} differs from initialization journal",
                        active.id
                    ),
                ));
            }
            write_json(&attempt_path, &journal.attempt)?;
        }

        let provider_path = self.role_provider_config_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt.id,
        );
        if super::path_is_regular_file(&provider_path)? {
            let existing: CodingRoleProviderConfigSnapshot = read_json(&provider_path)?;
            if existing != journal.provider_config {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "provider config differs from initialization journal",
                ));
            }
        } else {
            write_json(&provider_path, &journal.provider_config)?;
        }
        Ok(journal.attempt.clone())
    }

    pub fn validate_materialized_group_initialization_attempt(
        &self,
        journal: &CodingGroupInitializationJournal,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        guard.validate_identity(
            self,
            &journal.project_id,
            &journal.issue_id,
            &journal.lock_work_item_id,
        )?;
        let attempt_path =
            self.attempt_path(&journal.project_id, &journal.issue_id, &journal.attempt.id);
        if !super::path_is_regular_file(&attempt_path)? {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "persisted attempt is missing during bound replay",
            ));
        }
        let persisted_attempt: CodingExecutionAttempt = read_json(&attempt_path)?;
        if persisted_attempt != journal.attempt {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "persisted attempt differs from initialization journal",
            ));
        }
        // REQ-MTG-02（2.2）：同 ensure——active 检查按 target 桶细化。
        let journal_target = Self::attempt_target_bucket(&journal.attempt);
        let active_attempts = super::list_json_records::<CodingExecutionAttempt>(
            &self.coding_attempts_root(&journal.project_id, &journal.issue_id),
        )?
        .into_iter()
        .filter(|attempt| {
            attempt.status.is_active() && Self::attempt_target_bucket(attempt) == journal_target
        })
        .collect::<Vec<_>>();
        if active_attempts.len() != 1 || active_attempts[0].id != journal.attempt.id {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "another active attempt exists during bound replay",
            ));
        }
        let provider_path = self.role_provider_config_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt.id,
        );
        if !super::path_is_regular_file(&provider_path)? {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "provider config is missing during bound replay",
            ));
        }
        let provider_config: CodingRoleProviderConfigSnapshot = read_json(&provider_path)?;
        if provider_config != journal.provider_config {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "provider config differs from initialization journal",
            ));
        }
        Ok(persisted_attempt)
    }

    pub fn ensure_group_initialization_plan_binding(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<(), ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        self.save_plan_binding(&journal.attempt, &journal.plan_binding)
    }

    pub fn ensure_group_initialization_unit(
        &self,
        journal: &CodingGroupInitializationJournal,
        index: usize,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        let expected =
            journal
                .units
                .get(index)
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_unit",
                    id: index.to_string(),
                })?;
        let materialized =
            self.list_coding_units(&journal.project_id, &journal.issue_id, &journal.attempt.id)?;
        if materialized.len() > journal.units.len()
            || materialized
                .iter()
                .zip(journal.units.iter())
                .any(|(existing, expected)| existing != expected)
        {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "materialized units differ from initialization journal",
            ));
        }
        if let Some(existing) = materialized.get(index) {
            return Ok(existing.clone());
        }
        if materialized.len() != index {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "materialized units are not a contiguous journal prefix",
            ));
        }
        write_json(
            &self.coding_unit_path(
                &expected.project_id,
                &expected.issue_id,
                &expected.attempt_id,
                &expected.id,
            ),
            expected,
        )?;
        Ok(expected.clone())
    }

    pub fn mark_group_initialization_error(
        &self,
        journal: &CodingGroupInitializationJournal,
        error: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        // D2.1c：失败标记是 journal 写——删除窗口内不得复活已 purge 的
        // journal（purging/deleted 墓碑稳定拒绝）。
        self.with_plan_live_write(
            &journal.project_id,
            &journal.issue_id,
            &journal.plan_id,
            || self.mark_group_initialization_error_inner(journal, error),
        )
    }
    fn mark_group_initialization_error_inner(
        &self,
        journal: &CodingGroupInitializationJournal,
        error: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        let path = self.locate_group_initialization_journal_path(journal)?;
        let mut current: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&current)?;
        if !same_group_initialization_identity(&current, journal) {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "initialization journal changed while recording failure",
            ));
        }
        current.error = Some(error.to_string());
        current.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &current)?;
        Ok(current)
    }
    pub fn advance_group_initialization_phase(
        &self,
        expected: &CodingGroupInitializationJournal,
        next: CodingGroupInitializationPhase,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        // D2.1c：相位推进是 journal 写——删除窗口内不得复活已 purge 的
        // journal（purging/deleted 墓碑稳定拒绝）。
        self.with_plan_live_write(
            &expected.project_id,
            &expected.issue_id,
            &expected.plan_id,
            || self.advance_group_initialization_phase_inner(expected, next),
        )
    }
    fn advance_group_initialization_phase_inner(
        &self,
        expected: &CodingGroupInitializationJournal,
        next: CodingGroupInitializationPhase,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        validate_group_initialization_journal(expected)?;
        let path = self.locate_group_initialization_journal_path(expected)?;
        let mut current: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&current)?;
        if !same_group_initialization_identity(&current, expected) {
            return Err(incomplete_group_attempt(
                &expected.attempt.id,
                "initialization journal changed during replay",
            ));
        }
        if current.phase.order() >= next.order() {
            return Ok(current);
        }
        if next.order() != current.phase.order() + 1 {
            return Err(incomplete_group_attempt(
                &expected.attempt.id,
                "initialization phase advance is not contiguous",
            ));
        }
        current.phase = next;
        current.error = None;
        current.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &current)?;
        Ok(current)
    }
}
