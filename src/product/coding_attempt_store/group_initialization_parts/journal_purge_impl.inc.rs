impl super::CodingAttemptStore {
    pub(crate) fn delete_group_initialization_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(_plan_id) = attempt.work_item_group_id.as_deref() else {
            return Ok(());
        };
        // 先查原路径（单 target journal——mismatch 保持原 fail-closed 报错），
        // 再查 per-target 子目录（异 target 的 sibling journal 跳过不误删）。
        let original = self.group_initialization_journal_path(
            &attempt.project_id,
            &attempt.issue_id,
            _plan_id,
        );
        if super::path_is_regular_file(&original)? {
            let journal: CodingGroupInitializationJournal = read_json(&original)?;
            validate_group_initialization_journal(&journal)?;
            if journal.attempt.id != attempt.id {
                return Err(incomplete_group_attempt(
                    &attempt.id,
                    "delete target differs from initialization journal",
                ));
            }
            return super::remove_file_if_exists(&original);
        }
        if let Some(target) = Self::attempt_target_bucket(attempt) {
            let target_path = self.group_initialization_journal_path_for_target(
                &attempt.project_id,
                &attempt.issue_id,
                _plan_id,
                &target,
            );
            if super::path_is_regular_file(&target_path)? {
                let journal: CodingGroupInitializationJournal = read_json(&target_path)?;
                validate_group_initialization_journal(&journal)?;
                if journal.attempt.id == attempt.id {
                    return super::remove_file_if_exists(&target_path);
                }
            }
        }
        Ok(())
    }

    // ─── H2b（设计 A 节，整组删除与重启）：journal 引用分类与闭包清理 ───

    /// 按调用方枚举的真实 attempt 存活集分类该 plan 全部初始化 journal
    ///（原路径 + per-target 子目录；坏 JSON/身份错配 fail-closed 同
    /// `list_group_initialization_journals_for_plan`）。判定只看 journal
    /// 相位与存活集成员关系，不读 attempt 文件（权威存活集归闭包/调用方，
    /// 不复制第二套扫描器）：未达 AttemptPersisted 的 journal 缺 attempt
    /// 属既有初始化中断前缀（`InitializationPrefix`，续原协议，不当残留
    /// 清）；已越 AttemptPersisted 按 attempt 存活分 `BoundToAlive` /
    /// `BoundToDead`（后者为执行引用断裂，零成员闭包清理对象）。
    pub fn classify_group_initialization_references(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        existing_attempt_ids: &[String],
    ) -> Result<Vec<CodingGroupJournalReference>, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let existing: std::collections::BTreeSet<String> =
            existing_attempt_ids.iter().cloned().collect();
        Ok(self
            .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)?
            .into_iter()
            .map(|journal| {
                let journal_id = journal.id.clone();
                let attempt_id = journal.attempt.id.clone();
                if journal
                    .phase
                    .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
                {
                    if existing.contains(&attempt_id) {
                        CodingGroupJournalReference::BoundToAliveAttempt {
                            journal_id,
                            attempt_id,
                        }
                    } else {
                        CodingGroupJournalReference::BoundToDeadAttempt {
                            journal_id,
                            attempt_id,
                        }
                    }
                } else {
                    CodingGroupJournalReference::InitializationPrefix {
                        journal_id,
                        attempt_id,
                    }
                }
            })
            .collect())
    }

    /// H2b（设计 A 节完整重置闭包第 4 步）：清该 plan 的 group 初始化
    /// journal 残留（legacy 原路径 `{plan_id}.json` + per-target 子目录
    /// `{plan_id}/{target}.json`，含 attempts 已亡的 journal）。持精确
    /// Plan 独占 lease 消费（同 `coding_plan_run::purge_plan_with_lease`
    /// 模式）——租约即安全清理窗口，不嵌套重取 scope；missing 幂等；
    /// journal 身份/归属错配 fail-closed 不误删他 Plan 对象；不删
    /// `group-initializations` 根与 arbitration 公共锁。
    ///
    /// **内容不可解析 ≠ 删除失败**（REQ-WPS-03/04）：journal 的归属由路径
    /// 界定（文件名即 plan_id、目录即 plan_id），解析失败只说明该文件已不是
    /// 有效 journal 记录——按孤儿残留直接删除并继续（[`Self::read_journal_for_purge`]）；
    /// 真实 IO 错误（打开/删除失败）仍 fail-closed 上抛。
    pub fn purge_group_initialization_journals_with_lease(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_group_initialization_purge_scope",
                id: scope.entity_id.clone(),
            });
        }
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        // ① legacy 原路径 `{plan_id}.json`。
        let original = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if super::path_is_regular_file(&original)? {
            // 损坏残留（解析失败）已按孤儿处理：直接删掉，继续流程。
            if let Some(journal) = Self::read_journal_for_purge(&original)? {
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
            }
            super::remove_file_if_exists(&original)?;
        }
        // ② per-target 子目录 `{plan_id}/{target}.json`（逐一身份核对；非
        //    json 条目跳过，归属错配 fail-closed；损坏内容按孤儿残留删除）。
        let directory = self
            .coding_attempts_root(project_id, issue_id)
            .join("group-initializations")
            .join(plan_id);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    directory.display()
                )));
            }
        };
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read {} entry: {error}", directory.display()))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Some(journal) = Self::read_journal_for_purge(&path)? {
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
            }
            super::remove_file_if_exists(&path)?;
        }
        Ok(())
    }

    /// 读取「路径界定归属」的 journal 残留（purge 专用）。
    ///
    /// - 文件缺失 → `None`（缺失即已清，幂等）；
    /// - 内容解析失败 → `None`：该文件已不携带任何 journal 记录，路径
    ///   （`{plan_id}.json`／`{plan_id}/{target}.json`）本身已界定归属，故按孤儿
    ///   残留删除，不得让整个删除因残留物不可解析而失败（REQ-WPS-03/04）；
    /// - 真实 IO 错误（打开失败，权限拒绝等）→ 上抛 fail-closed。
    fn read_journal_for_purge(
        path: &std::path::Path,
    ) -> Result<Option<CodingGroupInitializationJournal>, ProductStoreError> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "open {}: {error}",
                    path.display()
                )));
            }
        };
        match serde_json::from_reader(file) {
            Ok(journal) => Ok(Some(journal)),
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "purge unparseable group initialization journal residue (path-scoped ownership)"
                );
                Ok(None)
            }
        }
    }

    /// OQ2 定位规则（WP2）：mark/advance 相位函数据此找到 journal 的实际存储
    /// 位置——先试原路径（存量单 target journal 零迁移），再试 attempt 快照
    /// 对应的 per-target 子目录；两处 id 均不匹配时回落原路径（由调用方的
    /// 既有校验产出 fail-closed 错误）。
    fn locate_group_initialization_journal_path(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        let original = self.group_initialization_journal_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.plan_id,
        );
        if super::path_is_regular_file(&original)? {
            let existing: CodingGroupInitializationJournal = read_json(&original)?;
            if existing.id == journal.id {
                return Ok(original);
            }
        }
        if let Some(target) = Self::attempt_target_bucket(&journal.attempt) {
            let target_path = self.group_initialization_journal_path_for_target(
                &journal.project_id,
                &journal.issue_id,
                &journal.plan_id,
                &target,
            );
            if super::path_is_regular_file(&target_path)? {
                let existing: CodingGroupInitializationJournal = read_json(&target_path)?;
                if existing.id == journal.id {
                    return Ok(target_path);
                }
            }
        }
        Ok(original)
    }

    fn build_group_initialization_journal(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        let id = self.allocate_coding_attempt_id();
        let attempt_no = self
            .list_attempts_for_work_item(
                &input.project_id,
                &input.issue_id,
                &input.current_work_item_id,
            )?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let first_unit_id = next_sequential_id("coding_unit", 0);
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            work_item_id: input.current_work_item_id.clone(),
            attempt_no,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Created,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch.clone(),
            branch_name: input.branch_name.clone(),
            worktree_path: input.worktree_path.clone(),
            provider_config_snapshot: input.provider_config_snapshot.clone(),
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: Some(input.plan_id.clone()),
            current_work_item_id: Some(input.current_work_item_id.clone()),
            active_unit_id: Some(first_unit_id),
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: now.clone(),
            updated_at: now.clone(),
            target_snapshot: input.target_snapshot.clone(),
            start_run_policy: input.start_run_policy.clone(),
            start_claim: None,
            completed_at: None,
        };
        let units = unit_bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| CodingExecutionUnit {
                id: next_sequential_id("coding_unit", index),
                attempt_id: id.clone(),
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                plan_id: input.plan_id.clone(),
                logical_work_item_id: binding.logical_work_item_id.clone(),
                work_item_revision_id: binding.work_item_revision_id.clone(),
                dependency_logical_work_item_ids: binding.dependency_logical_work_item_ids.clone(),
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
                started_at: (index == 0).then(|| now.clone()),
                completed_at: None,
                latest_handoff_revision_id: None,
                completion_commit: None,
                summary: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .collect::<Vec<_>>();
        let journal = CodingGroupInitializationJournal {
            id: match journal_target {
                // OQ2：单 target journal id 保持原形态（存量零迁移）；多 target
                // per-target journal 以 target UUID 限定（同 plan 各 journal 身份
                // 可区分，定位规则按 id 匹配）。
                None => format!("coding_group_initialization_{}", input.plan_id),
                Some(target) => {
                    format!("coding_group_initialization_{}_{}", input.plan_id, target.0)
                }
            },
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            lock_work_item_id: input.current_work_item_id.clone(),
            // 缺陷 #13 层2（组入口路由分流配套）：仓维锁 bind 只接受
            // `repo_worktree_lease_` 前缀租约；Logical 路由（journal_target
            // 存在）必须派生仓维前缀，Legacy 路由维持 issue 维前缀不变。
            worktree_lease_id: format!(
                "{}{}",
                if journal_target.is_some() {
                    "repo_worktree_lease_"
                } else {
                    "issue_worktree_lease_"
                },
                uuid::Uuid::new_v4().simple()
            ),
            provider_config: CodingRoleProviderConfigSnapshot::from(
                &input.provider_config_snapshot,
            ),
            plan_binding: CodingAttemptPlanBinding {
                attempt_id: id,
                plan_id: input.plan_id.clone(),
                bound_plan_revision_id: bound_plan_revision_id.to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: now.clone(),
            },
            attempt,
            units,
            phase: CodingGroupInitializationPhase::Prepared,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        validate_group_initialization_journal(&journal)?;
        Ok(journal)
    }
}
