//! 巡检上下文与判定辅助（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;
/// 巡检上下文：读路径共用的只读事实（session 集合 / attempt 探针 / 窗口簇结论）。
pub(super) struct PatrolContext<'a> {
    pub(super) paths: &'a ProductAppPaths,
    /// 路二：全量 workspace session id 集合（由列举构建）。
    pub(super) session_ids: BTreeSet<String>,
    /// 路二枚举失败的组合（非空 → 「未命中」结论不可用，REQ-DRG-23）。
    pub(super) session_enumeration_errors: Vec<String>,
    /// 路一探针（生产走内置 `CodingAttemptStore`；测试注入）。
    pub(super) attempt_probe: Option<AttemptProbe<'a>>,
    /// attempt id → 该 attempt 名下全部窗口（含归档）的 L2 聚合结论。
    pub(super) attempt_clusters: BTreeMap<String, AttemptClusterVerdict>,
}

impl PatrolContext<'_> {
    /// 构造存在性核对器（沿用 `resolve` 的九类分派，不另造一套）。
    pub(super) fn resolver(&self) -> DanglingResolver<'_> {
        match self.attempt_probe {
            Some(probe) => {
                DanglingResolver::with_attempt_probe(self.paths, &self.session_ids, probe)
            }
            None => DanglingResolver::new(self.paths, &self.session_ids),
        }
    }

    /// 路二是否可用（任一组合枚举失败即不可用）。
    pub(super) fn sessions_complete(&self) -> bool {
        self.session_enumeration_errors.is_empty()
    }

    /// 会话枚举失败的可读原因（供「不可判定」原因列）。
    pub(super) fn session_enumeration_reason(&self) -> String {
        format!(
            "存在性数据源 / 作用域不可用：workspace session 枚举失败，路二无法完成（{}）",
            self.session_enumeration_errors.join("；")
        )
    }

    /// 单条引用的三态判定（`Exists` / `Missing` + 证据与动作 / `Undetermined`）。
    pub(super) fn judge(
        &self,
        reference: &PatrolReference<'_>,
        lc: Option<&LcScopeContext>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        match reference.target_kind {
            DanglingTargetKind::StorySpec
            | DanglingTargetKind::DesignSpec
            | DanglingTargetKind::Plan => self.judge_issue_target(reference),
            DanglingTargetKind::Session => Ok(self.judge_session(reference)),
            DanglingTargetKind::Attempt => self.judge_attempt(reference),
            DanglingTargetKind::Enrollment => self.judge_enrollment(reference),
            DanglingTargetKind::Checkout
            | DanglingTargetKind::AggregateIndex
            | DanglingTargetKind::LogicalMember => Ok(self.judge_lc_target(reference, lc)),
        }
    }

    /// story / design / plan：同 issue 作用域内的真实读（L1 档位）。
    pub(super) fn judge_issue_target(
        &self,
        reference: &PatrolReference<'_>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        if reference.project_id.is_empty() || reference.issue_id.is_empty() {
            return Ok(ReferenceJudgement::Undetermined {
                reason: REASON_NO_ISSUE_SCOPE.to_string(),
            });
        }
        let scope = EntityScope::new(
            reference.project_id.clone(),
            reference.issue_id.clone(),
            deletion_kind_of(reference.target_kind),
            reference.target_id.clone(),
        );
        let resolver = self.resolver();
        match resolver.target_exists(
            reference.target_kind,
            reference.target_id.as_str(),
            Some(&scope),
        )? {
            TargetExistence::Exists => Ok(ReferenceJudgement::Alive),
            TargetExistence::Unknown => Ok(ReferenceJudgement::Undetermined {
                reason: REASON_TARGET_UNKNOWN.to_string(),
            }),
            TargetExistence::Missing => {
                let level = resolver.evidence_level(
                    reference.target_kind,
                    reference.target_id.as_str(),
                    Some(&scope),
                )?;
                Ok(l1_evidence_judgement(level))
            }
        }
    }

    /// session：路二集合成员关系（session 档位恒「只报不删」）。
    pub(super) fn judge_session(&self, reference: &PatrolReference<'_>) -> ReferenceJudgement {
        if self.session_ids.contains(reference.target_id.as_str()) {
            return ReferenceJudgement::Alive;
        }
        if self.sessions_complete() {
            return report_only_judgement();
        }
        // 路二未能完成：MUST NOT 当成「目标不存在」（REQ-DRG-21）。
        ReferenceJudgement::Undetermined {
            reason: self.session_enumeration_reason(),
        }
    }

    /// attempt：两路核对（路一 coding attempt + 路二 session 集合）。
    pub(super) fn judge_attempt(
        &self,
        reference: &PatrolReference<'_>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        match self.attempt_existence(reference.target_id.as_str())? {
            TargetExistence::Exists => Ok(ReferenceJudgement::Alive),
            TargetExistence::Unknown => Ok(ReferenceJudgement::Undetermined {
                reason: if self.sessions_complete() {
                    REASON_TARGET_UNKNOWN.to_string()
                } else {
                    self.session_enumeration_reason()
                },
            }),
            TargetExistence::Missing => Ok(match self.attempt_clusters.get(&reference.target_id) {
                Some(cluster) if cluster.l2_ready => ReferenceJudgement::Missing {
                    evidence_level: format!("{EVIDENCE_LABEL_L2}（{}）", cluster.summary),
                    proposed_action: PATROL_ACTION_DELETE_GROUP,
                },
                Some(cluster) => ReferenceJudgement::Missing {
                    evidence_level: format!("{EVIDENCE_LABEL_INSUFFICIENT}（{}）", cluster.summary),
                    proposed_action: PATROL_ACTION_REPORT_ONLY,
                },
                None => report_only_judgement(),
            }),
        }
    }

    /// attempt 存在性（路二不可用时，「未命中」降级为 `Unknown`）。
    pub(super) fn attempt_existence(
        &self,
        attempt_id: &str,
    ) -> Result<TargetExistence, ProductStoreError> {
        let existence =
            self.resolver()
                .target_exists(DanglingTargetKind::Attempt, attempt_id, None)?;
        match existence {
            TargetExistence::Missing if !self.sessions_complete() => Ok(TargetExistence::Unknown),
            other => Ok(other),
        }
    }

    /// enrollment：按记录所属 project/issue 读当处文件（REQ-DRG-01 点名数据源）。
    pub(super) fn judge_enrollment(
        &self,
        reference: &PatrolReference<'_>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        if reference.project_id.is_empty() || reference.issue_id.is_empty() {
            return Ok(ReferenceJudgement::Undetermined {
                reason: REASON_NO_ISSUE_SCOPE.to_string(),
            });
        }
        let store = IssueAutomationStore::new(self.paths.clone());
        let existing = store.get(&reference.project_id, &reference.issue_id)?;
        Ok(match existing {
            Some(record) if record.enrollment_id == reference.target_id => {
                ReferenceJudgement::Alive
            }
            // 文件缺失或当处 enrollment 是另一个 id：明确未命中。
            _ => report_only_judgement(),
        })
    }

    /// checkout / aggregate_index / logical_member：同 LC scope 内的真实读。
    pub(super) fn judge_lc_target(
        &self,
        reference: &PatrolReference<'_>,
        lc: Option<&LcScopeContext>,
    ) -> ReferenceJudgement {
        let Some(lc) = lc else {
            return ReferenceJudgement::Undetermined {
                reason: REASON_NO_LC_SCOPE.to_string(),
            };
        };
        let existence = match reference.target_kind {
            DanglingTargetKind::LogicalMember => lc.member_exists(reference.target_id.as_str()),
            DanglingTargetKind::Checkout => lc.checkout_exists(reference.target_id.as_str()),
            _ => lc.aggregate_index_exists(self.paths, reference.target_id.as_str()),
        };
        match existence {
            Ok(true) => ReferenceJudgement::Alive,
            // 三类恒「只报不删」（§3.1.2 降级档位）。
            Ok(false) => report_only_judgement(),
            Err(reason) => ReferenceJudgement::Undetermined { reason },
        }
    }
}

/// 一个 attempt 名下全部窗口（含归档）的 L2 聚合结论（r3 N-9：删除按 attempt 聚合）。
#[derive(Debug, Clone)]
pub(super) struct AttemptClusterVerdict {
    /// 四条是否在该 attempt 名下的**每一个**窗口上都成立。
    pub(super) l2_ready: bool,
    /// 四条件可读摘要（取该簇路径最小的窗口）。
    pub(super) summary: String,
}

/// 逻辑代码库 scope 的只读上下文（成员 / checkout 存在性集合 + 作用域标识）。
pub(super) struct LcScopeContext {
    /// scope 所属 project。
    pub(super) project_id: String,
    /// LC id（`None` = 旧版别名仓，按 unscoped store 解析 legacy 根）。
    pub(super) lc_id: Option<String>,
    /// scope 根目录。
    pub(super) root: PathBuf,
    /// 逻辑成员 id 集合（读失败带原因：该 scope 的成员引用一律不可判定）。
    pub(super) members: Result<BTreeSet<String>, String>,
    /// checkout id 集合（读失败带原因：该 scope 的 checkout 引用一律不可判定）。
    pub(super) checkouts: Result<BTreeSet<String>, String>,
}

impl LcScopeContext {
    /// 读取该 scope 的成员 / checkout 存在性集合（只读；读失败逐条进不可判定）。
    pub(super) fn load(
        paths: &ProductAppPaths,
        project_id: &str,
        lc_id: Option<String>,
        root: PathBuf,
    ) -> Self {
        let store = logical_codebase_store(paths, &lc_id);
        let members = match store.list_members(project_id) {
            Ok(records) => Ok(records
                .into_iter()
                .map(|record| record.logical_repository_id.0.to_string())
                .collect()),
            Err(error) => Err(format!("逻辑成员记录读取失败：{error}")),
        };
        let checkouts = match store.list_checkouts(project_id) {
            Ok(records) => Ok(records
                .into_iter()
                .map(|record| record.checkout_id.0.to_string())
                .collect()),
            Err(error) => Err(format!("checkout 记录读取失败：{error}")),
        };
        Self {
            project_id: project_id.to_string(),
            lc_id,
            root,
            members,
            checkouts,
        }
    }

    /// 逻辑成员是否存在（同 LC scope 内比对）。
    pub(super) fn member_exists(&self, id: &str) -> Result<bool, String> {
        match &self.members {
            Ok(ids) => Ok(ids.contains(id)),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// checkout 是否存在（同 LC scope 内比对）。
    pub(super) fn checkout_exists(&self, id: &str) -> Result<bool, String> {
        match &self.checkouts {
            Ok(ids) => Ok(ids.contains(id)),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// 聚合索引是否存在（同 LC scope 内按 id 真实读）。
    pub(super) fn aggregate_index_exists(
        &self,
        paths: &ProductAppPaths,
        id: &str,
    ) -> Result<bool, String> {
        let store = aggregate_index_store(paths, &self.lc_id);
        match store.get(&self.project_id, id) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(error) => Err(format!("聚合索引记录读取失败：{error}")),
        }
    }
}

/// 单条引用的判定结果（三态在巡检面的映射）。
pub(super) enum ReferenceJudgement {
    /// 目标仍在：不产发现、不产不可判定。
    Alive,
    /// 目标已删：产一条发现（证据等级 + 建议动作）。
    Missing {
        evidence_level: String,
        proposed_action: &'static str,
    },
    /// 无法判定：产一条不可判定条目（原因必填）。
    Undetermined { reason: String },
}

/// 一条待核对的引用。
pub(super) struct PatrolReference<'a> {
    /// 引用字段路径（判据表登记值，或 LC 三类的实际记录形状路径）。
    pub(super) field: &'a str,
    /// 引用目标类型。
    pub(super) target_kind: DanglingTargetKind,
    /// 引用目标 id。
    pub(super) target_id: String,
    /// 目标所属 project（同 issue 作用域用；无作用域需求时为空串）。
    pub(super) project_id: String,
    /// 目标所属 issue（同 issue 作用域用；无作用域需求时为空串）。
    pub(super) issue_id: String,
}

impl<'a> PatrolReference<'a> {
    pub(super) fn new(
        field: &'a str,
        target_kind: DanglingTargetKind,
        target_id: impl Into<String>,
        project_id: &str,
        issue_id: &str,
    ) -> Self {
        Self {
            field,
            target_kind,
            target_id: target_id.into(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        }
    }

    /// attempt 引用（全局目标，无 project/issue 作用域需求）。
    pub(super) fn attempt(attempt_id: impl Into<String>) -> Self {
        Self {
            field: FIELD_ATTEMPT,
            target_kind: DanglingTargetKind::Attempt,
            target_id: attempt_id.into(),
            project_id: String::new(),
            issue_id: String::new(),
        }
    }
}

/// 「已判定悬空 + 有 L1 墓碑」→ 建议删除；其余（含证据不足）→ 只报不删。
pub(super) fn l1_evidence_judgement(level: EvidenceLevel) -> ReferenceJudgement {
    match level {
        EvidenceLevel::L1 => ReferenceJudgement::Missing {
            evidence_level: EVIDENCE_LABEL_L1.to_string(),
            proposed_action: PATROL_ACTION_DELETE,
        },
        EvidenceLevel::L2 | EvidenceLevel::Insufficient => report_only_judgement(),
    }
}

/// 「已判定悬空 + 规则降级 / 证据不足」→ 只报不删。
pub(super) fn report_only_judgement() -> ReferenceJudgement {
    ReferenceJudgement::Missing {
        evidence_level: EVIDENCE_LABEL_INSUFFICIENT.to_string(),
        proposed_action: PATROL_ACTION_REPORT_ONLY,
    }
}

/// 目标类型 → 墓碑实体种类（仅 story / design / plan 有 L1 档位）。
pub(super) fn deletion_kind_of(target_kind: DanglingTargetKind) -> DeletionEntityKind {
    match target_kind {
        DanglingTargetKind::StorySpec => DeletionEntityKind::Story,
        DanglingTargetKind::DesignSpec => DeletionEntityKind::Design,
        _ => DeletionEntityKind::Plan,
    }
}

/// 判据表中某记录类型的某个目标类型的字段路径（`entity_kind` 分派用）。
pub(super) fn field_path_for(
    record_type: DanglingRecordType,
    target_kind: DanglingTargetKind,
) -> Option<&'static str> {
    reference_rule(record_type)?
        .fields
        .iter()
        .find(|field| field.target_kind == target_kind)
        .map(|field| field.path)
}

/// 核对一条引用并把结论落到三栏之一（读失败 → 不可判定，不得折叠）。
pub(super) fn check_reference(
    ctx: &PatrolContext<'_>,
    record_path: &str,
    record_type: DanglingRecordType,
    reference: &PatrolReference<'_>,
    lc: Option<&LcScopeContext>,
    builder: &mut ReportBuilder,
) {
    match ctx.judge(reference, lc) {
        Ok(ReferenceJudgement::Alive) => {}
        Ok(ReferenceJudgement::Undetermined { reason }) => {
            builder.push_undetermined(record_path.to_string(), reference.field, reason);
        }
        Ok(ReferenceJudgement::Missing {
            evidence_level,
            proposed_action,
        }) => builder.push_finding(PatrolFinding {
            record_path: record_path.to_string(),
            record_type: record_type.as_str().to_string(),
            reference_field: reference.field.to_string(),
            target_kind: reference.target_kind.as_str().to_string(),
            target_id: reference.target_id.clone(),
            evidence_level,
            proposed_action: proposed_action.to_string(),
        }),
        Err(error) => builder.push_undetermined(
            record_path.to_string(),
            reference.field,
            format!("存在性读取失败：{error}"),
        ),
    }
}
