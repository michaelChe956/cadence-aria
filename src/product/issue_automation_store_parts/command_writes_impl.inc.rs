impl IssueAutomationStore {
    /// 批 2 T1（REQ-WIGA-09）：不可恢复终止 CAS。enrollment 文件锁内：
    /// 命令账本先判（同 command 同 payload 重放首次 durable 结果、异
    /// payload Conflict），enrollment/revision 身份互证，随后
    /// `terminated_at`＋`enabled=false`＋`command_ledger` 同一原子写落盘
    /// （revision+1）。terminated 是不可恢复终态：此后 Enable/Disable/
    /// rebind/bind 一律拒绝（守卫见 compare_and_set/rebind），仅 N0
    /// attention 事实收尾对同一 terminated 绑定放行。锁内禁 await。
    pub fn terminate_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        request: EnrollmentTerminationRequest,
    ) -> Result<EnrollmentTerminationResult, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let Some(mut saved) = read_optional_enrollment(&path)? else {
                return Ok(TerminationResolution::Missing);
            };
            // 账本先判（身份边界见 `termination_command_ledger`）：同命令（含仅前置条件刷新）已终止即重放。
            if let Some(ledger) = saved
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == request.command_id)
            {
                if termination_ledger_verdict(ledger, &request)
                    == TerminationLedgerVerdict::Divergent
                {
                    return Ok(TerminationResolution::Conflict {
                        current_revision: Some(saved.policy_revision),
                        kind: ConflictKind::CommandLedgerPayloadMismatch,
                    });
                }
                if let Some(terminated_at) = saved.terminated_at.clone() {
                    return Ok(TerminationResolution::Replayed {
                        command_id: request.command_id.clone(),
                        terminated_at,
                        enrollment: saved,
                    });
                }
                // 曾被同命令终止、其后又被人工重新开启：不重放，落到精确 CAS。
            }
            if saved.enrollment_id != request.expected_enrollment_id {
                return Ok(TerminationResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::BindingIdentityMismatch,
                });
            }
            if request.expected_policy_revision != saved.policy_revision {
                return Ok(TerminationResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::RevisionStale,
                });
            }
            // 已 terminated 的异命令终止：终态不再接受新终止命令；未完成
            // 清理的重试以同 command 重放（上方 Replayed 分支）。
            if saved.terminated_at.is_some() {
                return Ok(TerminationResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::DefensiveStateMismatch,
                });
            }
            let binding_version = saved
                .binding_history
                .as_ref()
                .map(|history| history.current.binding_version)
                .unwrap_or(0);
            let now = now_rfc3339();
            saved.enabled = false;
            saved.terminated_at = Some(now.clone());
            saved.policy_revision += 1;
            saved.updated_at = now.clone();
            record_termination_ledger_accept(&mut saved.command_ledger, &request, binding_version);
            write_json(&path, &saved)?;
            Ok(TerminationResolution::Accepted {
                command_id: request.command_id.clone(),
                terminated_at: now,
                enrollment: saved,
            })
        })?;
        resolve_termination(resolution)
    }

    /// C1 Task 1（REQ-WIGA-01、REQ-C1-TARGET-01）：显式重绑/换代。在同一
    /// enrollment 文件锁内：先查命令账本（同 command 同 payload 幂等重放
    /// 首次 durable 结果、异 payload Conflict），再校验 enabled、expected
    /// policy/binding 版本与 target 授权域（跨载体/身份漂移 fail-closed，
    /// 旧 enrollment 无声明 target 不猜），最后追加 previous、写入新
    /// current（binding_version+1）并把 enrollment 投影同步到新代——
    /// 旧代身份只读保留，迟到旧回执按版本拒绝。
    pub fn rebind(
        &self,
        project_id: &str,
        issue_id: &str,
        request: EnrollmentRebindRequest,
    ) -> Result<EnrollmentRebindResult, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        validate_relative_id(&request.binding.plan_id)?;
        validate_relative_id(&request.binding.session_id)?;
        let digest = request.payload_digest();
        let resolution = with_exclusive_lock(&path, || {
            let Some(mut saved) = read_optional_enrollment(&path)? else {
                return Ok(RebindResolution::Missing);
            };
            // 批 2 T1（REQ-WIGA-09）：terminated 是不可恢复终态——任何
            // rebind（含旧命令账本重放面）一律 Conflict，不换代、不恢复。
            if saved.terminated_at.is_some() {
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::DefensiveStateMismatch,
                });
            }
            // 命令账本先判：同 command 同 payload 返回首次 durable 结果，
            // 异 payload 一律 Conflict（fail-closed，不猜哪个是"真的"）。
            if let Some(ledger) = saved
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == request.command_id)
            {
                if ledger.payload_digest == digest {
                    return Ok(RebindResolution::Replayed {
                        command_id: request.command_id.clone(),
                        enrollment: saved,
                    });
                }
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::CommandLedgerPayloadMismatch,
                });
            }
            if !saved.enabled {
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::BindingIdentityMismatch,
                });
            }
            if request.expected_policy_revision != saved.policy_revision {
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::RevisionStale,
                });
            }
            // target 授权域：无声明（旧 enrollment）不猜；跨载体/身份漂移拒绝。
            let Some(declared) = saved.target.clone() else {
                return Ok(RebindResolution::InvalidScope(
                    "rebind requires an enrollment with an explicitly declared target; \
                     re-enable with an explicit target first"
                        .to_string(),
                ));
            };
            if request.binding.target != declared {
                return Ok(RebindResolution::InvalidScope(format!(
                    "rebind target must match the enrollment's declared target \
                     carrier and identity: expected {declared:?}, got {:?}",
                    request.binding.target
                )));
            }
            let Some(history) = saved.binding_history.clone() else {
                return Ok(RebindResolution::InvalidScope(
                    "rebind requires durable binding history; re-enable with an \
                     explicit target first"
                        .to_string(),
                ));
            };
            if request.expected_binding_version != history.current.binding_version {
                return Ok(RebindResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                    kind: ConflictKind::BindingIdentityMismatch,
                });
            }
            // 追加换代：previous 只读保留旧代完整身份；current 写新代；
            // enrollment 投影（plan/session/source/providers）同步到新代。
            let mut previous = history.previous;
            let binding_version = history.current.binding_version + 1;
            previous.push(history.current);
            saved.binding_history = Some(EnrollmentBindingHistory {
                current: EnrollmentBindingIdentity {
                    binding_version,
                    enrollment_id: saved.enrollment_id.clone(),
                    plan_id: request.binding.plan_id.clone(),
                    session_id: request.binding.session_id.clone(),
                    source: request.binding.source.clone(),
                    target: request.binding.target.clone(),
                    author_provider: request.binding.author_provider.clone(),
                    reviewer_provider: request.binding.reviewer_provider.clone(),
                },
                previous,
            });
            saved.plan_id = Some(request.binding.plan_id.clone());
            saved.session_id = Some(request.binding.session_id.clone());
            saved.source = request.binding.source.clone();
            saved.options.author_provider = request.binding.author_provider;
            saved.options.reviewer_provider = request.binding.reviewer_provider;
            saved.policy_revision += 1;
            saved.updated_at = now_rfc3339();
            saved.command_ledger.push(EnrollmentCommandResult {
                command_id: request.command_id.clone(),
                payload_digest: digest,
                state: OperationState::Accepted,
                binding_version,
            });
            write_json(&path, &saved)?;
            Ok(RebindResolution::Accepted {
                command_id: request.command_id.clone(),
                enrollment: saved,
            })
        })?;
        resolve_rebind(resolution)
    }

    /// C1 Task 6（REQ-WIGA-03）：命令账本读取——同 `command_id` 同 payload
    /// 返回首次 durable 结果（调用方按 Replayed 投影），异 payload
    /// Conflict（fail-closed，不猜哪个是"真的"）。只读，不改变 enrollment
    /// 事实；enrollment 缺失返回 None。
    pub fn find_command_result(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
        payload_digest: &str,
    ) -> Result<Option<EnrollmentCommandResult>, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        with_exclusive_lock(&path, || {
            let Some(saved) = read_optional_enrollment(&path)? else {
                return Ok(None);
            };
            let Some(entry) = saved
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == command_id)
            else {
                return Ok(None);
            };
            if entry.payload_digest != payload_digest {
                return Err(ProductStoreError::Conflict {
                    kind: "enrollment_command_ledger",
                    id: command_id.to_string(),
                });
            }
            Ok(Some(entry.clone()))
        })
        .map_err(command_ledger_error)
    }

    /// C1 Task 6（REQ-WIGA-03）：追加命令账本（lease takeover 等非换代
    /// 动作的结果记录）。不改变 binding/policy；`binding_version` 以锁内
    /// 当前 binding 为准（无 binding 历史的旧 enrollment 记 0）。同
    /// command 同 payload 幂等返回既有条目，异 payload Conflict。
    pub fn append_command_result(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
        payload_digest: &str,
        state: OperationState,
    ) -> Result<EnrollmentCommandResult, EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        with_exclusive_lock(&path, || {
            let Some(mut saved) = read_optional_enrollment(&path)? else {
                return Err(ProductStoreError::NotFound {
                    kind: "automation_enrollment",
                    id: format!("{project_id}/{issue_id}"),
                });
            };
            if let Some(entry) = saved
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == command_id)
            {
                if entry.payload_digest == payload_digest {
                    return Ok(entry.clone());
                }
                return Err(ProductStoreError::Conflict {
                    kind: "enrollment_command_ledger",
                    id: command_id.to_string(),
                });
            }
            let binding_version = saved
                .binding_history
                .as_ref()
                .map(|history| history.current.binding_version)
                .unwrap_or(0);
            let entry = EnrollmentCommandResult {
                command_id: command_id.to_string(),
                payload_digest: payload_digest.to_string(),
                state,
                binding_version,
            };
            saved.command_ledger.push(entry.clone());
            saved.updated_at = now_rfc3339();
            write_json(&path, &saved)?;
            Ok(entry)
        })
        .map_err(command_ledger_error)
    }

    /// 批 2 N0（REQ-WIGA-09/12）：upsert 自动化待处理事实。enrollment
    /// 文件锁内做精确绑定互证（仅当前代 binding 全等可写，旧/别的绑定
    /// 拒绝；disabled 未终止拒绝；同一已 terminated 绑定放行终止收尾），
    /// 事实的 plan/session 身份与绑定当前代互证；同 key 不增项且
    /// `occurred_at` 首次冻结（重入保留）；不递增 `policy_revision`（不
    /// 破坏其他 CAS 持有者的 expected_revision）。写失败原样上抛。
    pub fn upsert_automation_attention_fact(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_binding: &EnrollmentBindingIdentity,
        fact: AutomationAttentionFact,
    ) -> Result<(), EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let Some(mut saved) = read_optional_enrollment(&path)? else {
                return Ok(AttentionWriteResolution::Missing);
            };
            if let Err(resolution) = attest_attention_binding_locked(&saved, expected_binding) {
                return Ok(resolution);
            }
            // 绑定互证：事实携带的 plan/session 身份必须与绑定当前代一致。
            if let Some(plan_id) = fact.plan_id.as_deref()
                && plan_id != expected_binding.plan_id
            {
                return Ok(AttentionWriteResolution::InvalidScope(format!(
                    "attention fact plan_id must match the bound plan: expected {}, got {plan_id}",
                    expected_binding.plan_id
                )));
            }
            if let Some(session_id) = fact.session_id.as_deref()
                && session_id != expected_binding.session_id
            {
                return Ok(AttentionWriteResolution::InvalidScope(format!(
                    "attention fact session_id must match the bound session: expected {}, \
                     got {session_id}",
                    expected_binding.session_id
                )));
            }
            // 稳定 key/time：同 key 重入替换内容但保留首次 occurred_at。
            match saved
                .attention_facts
                .iter_mut()
                .find(|existing| existing.key == fact.key)
            {
                Some(existing) => {
                    let frozen = existing.occurred_at.clone();
                    *existing = fact;
                    existing.occurred_at = frozen;
                }
                None => saved.attention_facts.push(fact),
            }
            saved.updated_at = now_rfc3339();
            write_json(&path, &saved)?;
            Ok(AttentionWriteResolution::Applied)
        })?;
        resolve_attention_write(resolution)
    }

    /// 批 2 N0（REQ-WIGA-09/12）：按稳定 key 清除待处理事实（绑定互证
    /// 规则同 upsert；T3 终止批量清/continue-chain 成功清均走此口）。
    /// key 不存在幂等成功；有清除才落盘；不递增 `policy_revision`。
    pub fn resolve_automation_attention_fact(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_binding: &EnrollmentBindingIdentity,
        key: &str,
    ) -> Result<(), EnrollmentError> {
        let path = self.enrollment_path(project_id, issue_id)?;
        let resolution = with_exclusive_lock(&path, || {
            let Some(mut saved) = read_optional_enrollment(&path)? else {
                return Ok(AttentionWriteResolution::Missing);
            };
            if let Err(resolution) = attest_attention_binding_locked(&saved, expected_binding) {
                return Ok(resolution);
            }
            let before = saved.attention_facts.len();
            saved.attention_facts.retain(|fact| fact.key != key);
            if saved.attention_facts.len() != before {
                saved.updated_at = now_rfc3339();
                write_json(&path, &saved)?;
            }
            Ok(AttentionWriteResolution::Applied)
        })?;
        resolve_attention_write(resolution)
    }
}
