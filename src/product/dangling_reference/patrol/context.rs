//! 巡检上下文与判定辅助（由 patrol.rs 按职责原样拆出）。
//!
//! **判定口径的唯一来源是证据等级表**（`reference_table`：REQ-DRG-02 附表
//! #1–#10 逐行照抄）：存在性分派走 `resolve::dispatch_existence`（与①层**共用
//! 同一套分派**），可删档位走附表的「可删证据等级」列——本模块 MUST NOT 另写
//! 一套 per-target 分支。巡检面与①层的差别只在于**判定面能力**（作用域 /
//! LC scope 读 / enrollment 读 / 路二可用性），见 [`PatrolContext::env`]。

use super::*;
/// 巡检上下文：读路径共用的只读事实（session 集合 / attempt 探针 / 窗口簇结论）。
pub(super) struct PatrolContext<'a> {
    pub(super) paths: &'a ProductAppPaths,
    /// 路二：全量 workspace session id 集合（由列举构建）。
    pub(super) session_ids: BTreeSet<String>,
    /// 路二枚举失败原因（`None` = 路二完整可用；`Some` = 枚举失败，「未命中」
    /// 结论不可用，REQ-DRG-23）。
    pub(super) session_unavailable_reason: Option<String>,
    /// 路一探针（生产走内置 `CodingAttemptStore`；测试注入）。
    pub(super) attempt_probe: Option<AttemptProbe<'a>>,
    /// attempt id → 该 attempt 名下全部窗口（含归档）的 L2 聚合结论。
    pub(super) attempt_clusters: BTreeMap<String, AttemptClusterVerdict>,
}

impl PatrolContext<'_> {
    /// 构造存在性核对器（证据等级判定沿用 `resolve` 的实现，不另造一套）。
    pub(super) fn resolver(&self) -> DanglingResolver<'_> {
        match self.attempt_probe {
            Some(probe) => {
                DanglingResolver::with_attempt_probe(self.paths, &self.session_ids, probe)
            }
            None => DanglingResolver::new(self.paths, &self.session_ids),
        }
    }

    /// 巡检面的判定面能力（作用域 / LC scope 读 / enrollment 读 / 路二可用性）。
    ///
    /// 统一分派（`resolve::dispatch_existence`）据此读存在性；①层与巡检面的
    /// 差别**只在这里**（巡检面能真读 LC scope 与 enrollment 文件）。
    fn env<'a>(
        &'a self,
        scope: Option<&'a EntityScope>,
        lc: Option<&'a dyn LcScopeReader>,
        enrollment: Option<&'a dyn EnrollmentReader>,
    ) -> ExistenceEnv<'a> {
        ExistenceEnv {
            paths: self.paths,
            session_ids: &self.session_ids,
            attempt_probe: self.attempt_probe,
            scope,
            lc,
            enrollment,
            session_unavailable_reason: self.session_unavailable_reason.as_deref(),
        }
    }

    /// attempt 存在性（统一分派；路二不可用时「未命中」降级为 `Unknown`）。
    pub(super) fn attempt_existence(
        &self,
        attempt_id: &str,
    ) -> Result<TargetExistence, ProductStoreError> {
        let verdict = dispatch_existence(
            &self.env(None, None, None),
            window_record_target_kind(),
            attempt_id,
        )?;
        Ok(verdict.existence)
    }

    /// 单条引用的三态判定（`Exists` / `Missing` + 证据与动作 / `Undetermined`）。
    ///
    /// 分派口径一律取自**证据等级表**（`reference_table::evidence_policy`），
    /// 实现与①层**共用** `resolve::dispatch_existence`——本方法只负责把巡检面
    /// **能提供的判定面能力**（作用域 / LC scope 读 / enrollment 读 / 路二可用性）
    /// 传进去，并把三态落到巡检面的三栏结论上；MUST NOT 另写一套 per-target
    /// 分支（REQ-DRG-01 / 23）。
    pub(super) fn judge(
        &self,
        reference: &PatrolReference<'_>,
        lc: Option<&LcScopeContext>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        let policy = evidence_policy(reference.target_kind);
        // 作用域需求（附表列）：story / design / plan / enrollment 必须持有记录
        // 所属的 project/issue；缺作用域即「不可判定」（REQ-DRG-23），不得猜。
        if policy.scope_requirement == ScopeRequirement::IssueScope
            && (reference.project_id.is_empty() || reference.issue_id.is_empty())
        {
            return Ok(ReferenceJudgement::Undetermined {
                reason: REASON_NO_ISSUE_SCOPE.to_string(),
            });
        }
        // 判定面能力：墓碑作用域（附表登记的墓碑实体种类）/ LC scope 读 /
        // enrollment 读 / 路二可用性。
        let scope = policy.deletion_kind.map(|deletion_kind| {
            EntityScope::new(
                reference.project_id.clone(),
                reference.issue_id.clone(),
                deletion_kind,
                reference.target_id.clone(),
            )
        });
        let enrollment = EnrollmentFileReader {
            paths: self.paths,
            project_id: &reference.project_id,
            issue_id: &reference.issue_id,
        };
        let env = self.env(
            scope.as_ref(),
            lc.map(|scope| scope as &dyn LcScopeReader),
            Some(&enrollment),
        );
        let verdict = dispatch_existence(&env, reference.target_kind, &reference.target_id)?;
        match verdict.existence {
            TargetExistence::Exists => Ok(ReferenceJudgement::Alive),
            TargetExistence::Unknown => Ok(ReferenceJudgement::Undetermined {
                reason: undetermined_reason(verdict.cause.as_ref()),
            }),
            TargetExistence::Missing => self.missing_judgement(reference, policy, scope.as_ref()),
        }
    }

    /// 「已判定悬空」后的证据与动作（按附表登记的可删等级分派）。
    fn missing_judgement(
        &self,
        reference: &PatrolReference<'_>,
        policy: &TargetEvidencePolicy,
        scope: Option<&EntityScope>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        match policy.deletable_level {
            // L1 档：墓碑四条（REQ-DRG-02）达标才建议删除，否则只报不删。
            DeletableLevel::L1 => {
                let level = self.resolver().evidence_level(
                    reference.target_kind,
                    reference.target_id.as_str(),
                    scope,
                )?;
                Ok(l1_evidence_judgement(level))
            }
            // L2 档：该类型的专属可算条件（窗口簇四条件）按 attempt 聚合。
            DeletableLevel::L2 => Ok(match self.attempt_clusters.get(&reference.target_id) {
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
            // 只报不删档位（含 enrollment 的「永不删」）：一律只报不删。
            DeletableLevel::ReportOnly | DeletableLevel::NeverDelete => Ok(report_only_judgement()),
        }
    }
}

/// 不可判定原因类别 → 巡检面「不可判定」原因文案。
fn undetermined_reason(cause: Option<&UndeterminedCause>) -> String {
    match cause {
        Some(UndeterminedCause::MissingIssueScope) => REASON_NO_ISSUE_SCOPE.to_string(),
        Some(UndeterminedCause::MissingLcScope) => REASON_NO_LC_SCOPE.to_string(),
        Some(UndeterminedCause::TargetUnknown) => REASON_TARGET_UNKNOWN.to_string(),
        Some(UndeterminedCause::SourceUnavailable(reason)) => reason.clone(),
        None => REASON_TARGET_UNKNOWN.to_string(),
    }
}

/// enrollment 文件读能力（按记录所属 issue 绑定；巡检面持有，①层不持有）。
struct EnrollmentFileReader<'a> {
    paths: &'a ProductAppPaths,
    project_id: &'a str,
    issue_id: &'a str,
}

impl EnrollmentReader for EnrollmentFileReader<'_> {
    /// 当处 enrollment 记录存在且 enrollment id 命中 → 目标仍在。
    ///
    /// 文件缺失 / 当处 enrollment 是另一个 id → 明确未命中（REQ-DRG-01）。
    fn exists(&self, enrollment_id: &str) -> Result<bool, ProductStoreError> {
        let existing =
            IssueAutomationStore::new(self.paths.clone()).get(self.project_id, self.issue_id)?;
        Ok(existing.is_some_and(|record| record.enrollment_id == enrollment_id))
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

/// 巡检面的 LC scope 读能力（统一分派按附表的 `ExistenceDispatch` 列调用）。
impl LcScopeReader for LcScopeContext {
    /// 同 LC scope 内逻辑成员是否存在。
    fn member_exists(&self, target_id: &str) -> Result<bool, String> {
        LcScopeContext::member_exists(self, target_id)
    }

    /// 同 LC scope 内 checkout 是否存在。
    fn checkout_exists(&self, target_id: &str) -> Result<bool, String> {
        LcScopeContext::checkout_exists(self, target_id)
    }

    /// 同 LC scope 内聚合索引是否存在。
    fn aggregate_index_exists(
        &self,
        paths: &ProductAppPaths,
        target_id: &str,
    ) -> Result<bool, String> {
        LcScopeContext::aggregate_index_exists(self, paths, target_id)
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
    ///
    /// 目标类型取自**判据表**（reviewer 窗口 / 快照的引用字段指向 attempt），
    /// MUST NOT 在此另写 per-target 常量。
    pub(super) fn attempt(attempt_id: impl Into<String>) -> Self {
        Self {
            field: FIELD_ATTEMPT,
            target_kind: window_record_target_kind(),
            target_id: attempt_id.into(),
            project_id: String::new(),
            issue_id: String::new(),
        }
    }
}

/// 窗口 / 快照记录引用的目标类型（取自判据表；窗口簇按该目标聚合）。
pub(super) fn window_record_target_kind() -> DanglingTargetKind {
    primary_target_kind(DanglingRecordType::ReviewerWindow)
        .expect("判据表必须登记 reviewer 窗口的引用字段")
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
