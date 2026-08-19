use chrono::Utc;
use std::path::PathBuf;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    DesignSpecRecord, LifecycleConfirmationStatus, ProjectProviderDefaultsRecord,
    SpecVersionRecord, StorySpecRecord, WorkspaceType,
};

use super::{
    AggregateDesignSpecScope, AggregateStorySpecScope, AppendSpecVersionInput,
    CreateDesignSpecInput, CreateProjectProviderDefaultsInput, CreateStorySpecInput,
    LifecycleStore, count_json_files, delete_required_file, ensure_target_absent,
    list_json_records, path_is_regular_file, remove_dir_all_if_exists, validate_relative_ids,
};

pub(crate) enum ExistingSpecRecord {
    Story {
        path: PathBuf,
        record: StorySpecRecord,
    },
    Design {
        path: PathBuf,
        record: DesignSpecRecord,
    },
}

/// confirm 聚合 spec 门禁的业务违规类型（4xx 阻断，稳定错误码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmGateViolation {
    /// 多仓 Story/Design involved 为空（REQ-PLN-04）。
    InvolvedUndetermined,
    /// 多仓 Design（involved>1）缺 change_order（REQ-PLN-05 收紧）。
    ChangeOrderRequired,
}

impl ConfirmGateViolation {
    /// 稳定错误码字符串（HTTP `code` 字段，消费者依赖，不得漂移）。
    pub fn stable_code(&self) -> &'static str {
        match self {
            ConfirmGateViolation::InvolvedUndetermined => "involved_repositories_undetermined",
            ConfirmGateViolation::ChangeOrderRequired => {
                "change_order_required_for_logical_codebase"
            }
        }
    }
}

/// validate_confirm_aggregate_spec 的失败分类，供调用方区分 HTTP 状态码（#5 收尾）。
///
/// - [`Self::Violation`]：业务违规 → 4xx + [`ConfirmGateViolation::stable_code`]；
/// - [`Self::SpecNotFound`]：spec 文件缺失 → 404 `spec_not_found`；
/// - [`Self::SpecLoad`]：spec 读盘/解析失败 → 500 `confirm_gate_spec_load_failed`。
#[derive(Debug, thiserror::Error)]
pub enum ConfirmAggregateGateError {
    /// 业务违规（involved 空 / 多仓 Design 缺 change_order）。
    #[error("{message}")]
    Violation {
        violation: ConfirmGateViolation,
        message: String,
    },
    /// spec 文件不存在（load_existing_spec 返 NotFound）。
    #[error("spec not found: {0}")]
    SpecNotFound(String),
    /// spec 读盘/解析失败（IO/JSON 等基础设施错误）。
    #[error("confirm_gate_spec_load_failed: {0}")]
    SpecLoad(ProductStoreError),
}

impl ConfirmAggregateGateError {
    /// 构造业务违规错误。
    pub fn gate_violation(violation: ConfirmGateViolation, message: &str) -> Self {
        Self::Violation {
            violation,
            message: message.to_string(),
        }
    }

    /// 业务违规的稳定错误码（仅 [`Self::Violation`] 有意义，其余变体返回各自读盘语义码）。
    pub fn stable_code(&self) -> &'static str {
        match self {
            Self::Violation { violation, .. } => violation.stable_code(),
            Self::SpecNotFound(_) => "spec_not_found",
            Self::SpecLoad(_) => "confirm_gate_spec_load_failed",
        }
    }

    /// 业务违规的可读消息（WS/HTTP message 字段统一用此，避免调用方重复 match）。
    ///
    /// `SpecLoad` 返回脱敏的固定文案（不拼接底层 `ProductStoreError` 的 Display，
    /// 避免绝对路径/OS 诊断/JSON 解析细节经公开 API message 泄露——reviewer Important）。
    /// 底层错误仍保留在 enum 变体里，供服务端日志/调试使用。
    pub fn message(&self) -> String {
        match self {
            Self::Violation { message, .. } => message.clone(),
            Self::SpecNotFound(_) => "spec not found".to_string(),
            Self::SpecLoad(_) => "spec load failed".to_string(),
        }
    }
}

impl From<ProductStoreError> for ConfirmAggregateGateError {
    /// 把 load_existing_spec 的 ProductStoreError 分类：NotFound → SpecNotFound，其余 → SpecLoad。
    fn from(error: ProductStoreError) -> Self {
        match error {
            ProductStoreError::NotFound { kind: "spec", id } => Self::SpecNotFound(id),
            other => Self::SpecLoad(other),
        }
    }
}

impl LifecycleStore {
    pub fn create_story_spec(
        &self,
        input: CreateStorySpecInput,
    ) -> Result<StorySpecRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        // 聚合代码库（aggregate_codebase=Some）无 repo_id（Logical 分支传空串），跳过
        // validate_relative_id（空串必败）；单仓（None）保持原校验，向后兼容。
        if input.aggregate_codebase.is_none() {
            validate_relative_id(&input.repository_id)?;
        }

        let root = self.story_specs_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("story_spec", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();

        // 逻辑代码库分支：以聚合视野字段为权威。AI 未明确涉及仓库或涉及不在有效集合的
        // 仓库 → blocker，不塞 primary（REQ-PLN-07）。传统单仓 issue（scope = None）走原
        // repository_id 单值路径，聚合字段保持空/None。
        let (logical_codebase_ref, involved_repository_ids, focus_repository_id) =
            match &input.aggregate_codebase {
                Some(scope) => {
                    validate_aggregate_story_scope(scope, LifecycleConfirmationStatus::Draft)?;
                    (
                        Some(scope.logical_codebase_ref),
                        scope.involved_repository_ids.clone(),
                        scope.focus_repository_id,
                    )
                }
                None => (None, Vec::new(), None),
            };

        let story = StorySpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            title: input.title,
            logical_codebase_ref,
            involved_repository_ids,
            focus_repository_id,
            current_version: None,
            confirmation_status: LifecycleConfirmationStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &story)?;
        Ok(story)
    }

    pub fn list_story_specs(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<StorySpecRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.story_specs_root(project_id, issue_id))
    }

    pub fn delete_story_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        story_spec_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(story_spec_id)?;

        delete_required_file(
            &self
                .story_specs_root(project_id, issue_id)
                .join(format!("{story_spec_id}.json")),
            "story_spec",
            story_spec_id,
        )?;
        remove_dir_all_if_exists(&self.versions_root(project_id, issue_id, story_spec_id))?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            story_spec_id,
            WorkspaceType::Story,
        )
    }

    pub fn create_design_spec(
        &self,
        input: CreateDesignSpecInput,
    ) -> Result<DesignSpecRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_ids(&input.story_spec_ids)?;

        let root = self.design_specs_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("design_spec", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();

        // 逻辑代码库分支：以聚合视野字段为权威。AI 未明确涉及仓库或涉及不在有效集合的
        // 仓库 → blocker，不回落 issue.repo_id（REQ-PLN-08）。Draft 时 change_order 可为空；
        // Confirmed 时 involved > 1 的 Design 必须提供。若给出则全部 id 必须 ∈ involved 且
        // 不重复。传统单仓 issue（scope = None）聚合字段保持空/None。
        let (logical_codebase_ref, involved_repository_ids, change_order) =
            match &input.aggregate_codebase {
                Some(scope) => {
                    validate_aggregate_design_scope(scope, LifecycleConfirmationStatus::Draft)?;
                    (
                        Some(scope.logical_codebase_ref),
                        scope.involved_repository_ids.clone(),
                        scope.change_order.clone(),
                    )
                }
                None => (None, Vec::new(), Vec::new()),
            };

        let design = DesignSpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            story_spec_ids: input.story_spec_ids,
            title: input.title,
            logical_codebase_ref,
            involved_repository_ids,
            change_order,
            current_version: None,
            confirmation_status: LifecycleConfirmationStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &design)?;
        Ok(design)
    }

    pub fn list_design_specs(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<DesignSpecRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.design_specs_root(project_id, issue_id))
    }

    pub fn delete_design_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        design_spec_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(design_spec_id)?;

        delete_required_file(
            &self
                .design_specs_root(project_id, issue_id)
                .join(format!("{design_spec_id}.json")),
            "design_spec",
            design_spec_id,
        )?;
        remove_dir_all_if_exists(&self.versions_root(project_id, issue_id, design_spec_id))?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            design_spec_id,
            WorkspaceType::Design,
        )
    }

    pub fn append_version(
        &self,
        input: AppendSpecVersionInput,
    ) -> Result<SpecVersionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.entity_id)?;

        let spec = self.load_existing_spec(&input.project_id, &input.issue_id, &input.entity_id)?;
        let root = self.versions_root(&input.project_id, &input.issue_id, &input.entity_id);
        let versions: Vec<SpecVersionRecord> = list_json_records(&root)?;
        let version = super::next_version_number(&versions)?;
        let id = next_sequential_id(
            "version",
            usize::try_from(version - 1).map_err(|_| {
                ProductStoreError::Io(format!("version sequence overflow: {version}"))
            })?,
        );
        let now = Utc::now().to_rfc3339();
        let record = SpecVersionRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            entity_id: input.entity_id,
            version,
            markdown: input.markdown,
            provider_run_refs: input.provider_run_refs,
            review_refs: input.review_refs,
            confirmed_by: input.confirmed_by,
            created_at: now.clone(),
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &record)?;
        self.update_spec_current_version(spec, version, now)?;
        Ok(record)
    }

    pub(crate) fn ensure_version(
        &self,
        input: AppendSpecVersionInput,
    ) -> Result<SpecVersionRecord, ProductStoreError> {
        let versions = self.list_versions(&input.project_id, &input.issue_id, &input.entity_id)?;
        if let Some(latest) = versions
            .last()
            .filter(|record| record.markdown.trim() == input.markdown.trim())
        {
            let spec =
                self.load_existing_spec(&input.project_id, &input.issue_id, &input.entity_id)?;
            self.update_spec_current_version(spec, latest.version, Utc::now().to_rfc3339())?;
            return Ok(latest.clone());
        }
        self.append_version(input)
    }

    pub fn list_versions(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> Result<Vec<SpecVersionRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;
        list_json_records(&self.versions_root(project_id, issue_id, entity_id))
    }

    pub fn update_spec_confirmation_status(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        status: LifecycleConfirmationStatus,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;

        let spec = self.load_existing_spec(project_id, issue_id, entity_id)?;
        let updated_at = Utc::now().to_rfc3339();
        match spec {
            ExistingSpecRecord::Story {
                path,
                record: mut story,
            } => {
                story.confirmation_status = status;
                story.updated_at = updated_at;
                write_json(&path, &story)
            }
            ExistingSpecRecord::Design {
                path,
                record: mut design,
            } => {
                design.confirmation_status = status;
                design.updated_at = updated_at;
                write_json(&path, &design)
            }
        }
    }

    pub fn update_story_spec_aggregate(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        scope: &AggregateStorySpecScope,
    ) -> Result<StorySpecRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;
        // 校验 involved ⊆ effective_member_ids（Draft 态也校验，防止脏数据）；顺带校验
        // focus_repository_id ∈ involved。Draft 态允许空 involved（AI 尚未产出）。
        validate_aggregate_story_scope(scope, LifecycleConfirmationStatus::Draft)?;

        let ExistingSpecRecord::Story {
            path,
            record: mut story,
        } = self.load_existing_spec(project_id, issue_id, entity_id)?
        else {
            return Err(ProductStoreError::Conflict {
                kind: "story_aggregate_kind_mismatch",
                id: entity_id.to_string(),
            });
        };
        // 仅 Draft 态可更新（Confirmed 后锁定）。
        if story.confirmation_status != LifecycleConfirmationStatus::Draft {
            return Err(ProductStoreError::Conflict {
                kind: "story_aggregate_locked",
                id: entity_id.to_string(),
            });
        }

        story.involved_repository_ids = scope.involved_repository_ids.clone();
        story.focus_repository_id = scope.focus_repository_id;
        story.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &story)?;
        Ok(story)
    }

    pub fn update_design_spec_aggregate(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        scope: &AggregateDesignSpecScope,
    ) -> Result<DesignSpecRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;
        // 校验 involved ⊆ effective_member_ids（Draft 态也校验，防止脏数据）；顺带校验
        // change_order ⊆ involved 且不重复。Draft 态允许空 involved（AI 尚未产出）。
        validate_aggregate_design_scope(scope, LifecycleConfirmationStatus::Draft)?;

        let ExistingSpecRecord::Design {
            path,
            record: mut design,
        } = self.load_existing_spec(project_id, issue_id, entity_id)?
        else {
            return Err(ProductStoreError::Conflict {
                kind: "design_aggregate_kind_mismatch",
                id: entity_id.to_string(),
            });
        };
        // 仅 Draft 态可更新（Confirmed 后锁定）。
        if design.confirmation_status != LifecycleConfirmationStatus::Draft {
            return Err(ProductStoreError::Conflict {
                kind: "design_aggregate_locked",
                id: entity_id.to_string(),
            });
        }

        design.involved_repository_ids = scope.involved_repository_ids.clone();
        design.change_order = scope.change_order.clone();
        design.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &design)?;
        Ok(design)
    }

    pub fn upsert_project_provider_defaults(
        &self,
        input: CreateProjectProviderDefaultsInput,
    ) -> Result<ProjectProviderDefaultsRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;

        let defaults = ProjectProviderDefaultsRecord {
            project_id: input.project_id,
            author_provider: input.author_provider,
            reviewer_provider: input.reviewer_provider,
            review_rounds: input.review_rounds,
            superpowers_enabled: input.superpowers_enabled,
            openspec_enabled: input.openspec_enabled,
            updated_at: Utc::now().to_rfc3339(),
        };

        write_json(
            &self
                .paths
                .project_provider_defaults_path(&defaults.project_id),
            &defaults,
        )?;
        Ok(defaults)
    }

    /// 确认 gate 校验（Task 6 下沉，Blocker 2 修复）：多仓（logical_codebase_ref 非 None）
    /// Story/Design 在 Confirmed 前校验 ① involved 非空（REQ-PLN-04「AI 不确定即 blocker」）；
    /// ② Design involved>1 必须 change_order（决策 3b，REQ-PLN-05 收紧）。
    ///
    /// 单仓（logical_codebase_ref None）或非 Story/Design 不校验，保持 Legacy 行为不变（红线）。
    ///
    /// 返回 [`ConfirmAggregateGateError`]，让调用方（WebSocket/HTTP）按变体决定 HTTP 状态码与
    /// 稳定错误码：① 业务违规（involved 空 / 多仓 Design 缺 change_order）→ 4xx + 稳定码；
    /// ② spec 文件缺失（NotFound）→ 404；③ spec 读盘/解析失败 → 500。修复前三者被字符串化
    /// 后统一映射 500，丢失了 NotFound 的 404 语义（#5 收尾）。
    pub fn validate_confirm_aggregate_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        workspace_type: &WorkspaceType,
    ) -> Result<(), ConfirmAggregateGateError> {
        if !matches!(workspace_type, WorkspaceType::Story | WorkspaceType::Design) {
            return Ok(());
        }
        let spec = self.load_existing_spec(project_id, issue_id, entity_id)?;
        let (is_logical, involved_len, change_order_empty) = match &spec {
            ExistingSpecRecord::Story { record, .. } => (
                record.logical_codebase_ref.is_some(),
                record.involved_repository_ids.len(),
                true,
            ),
            ExistingSpecRecord::Design { record, .. } => (
                record.logical_codebase_ref.is_some(),
                record.involved_repository_ids.len(),
                record.change_order.is_empty(),
            ),
        };
        if is_logical && involved_len == 0 {
            return Err(ConfirmAggregateGateError::gate_violation(
                ConfirmGateViolation::InvolvedUndetermined,
                "AI 未明确涉及仓库，不能确认多仓 Spec（REQ-PLN-04）",
            ));
        }
        if matches!(workspace_type, WorkspaceType::Design)
            && is_logical
            && involved_len > 1
            && change_order_empty
        {
            return Err(ConfirmAggregateGateError::gate_violation(
                ConfirmGateViolation::ChangeOrderRequired,
                "多仓 Design 必须提供 change_order（REQ-PLN-05）",
            ));
        }
        Ok(())
    }

    pub(crate) fn load_existing_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> Result<ExistingSpecRecord, ProductStoreError> {
        let story_path = self
            .story_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_is_regular_file(&story_path)? {
            let record = read_json(&story_path)?;
            return Ok(ExistingSpecRecord::Story {
                path: story_path,
                record,
            });
        }

        let design_path = self
            .design_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_is_regular_file(&design_path)? {
            let record = read_json(&design_path)?;
            return Ok(ExistingSpecRecord::Design {
                path: design_path,
                record,
            });
        }

        Err(ProductStoreError::NotFound {
            kind: "spec",
            id: entity_id.to_string(),
        })
    }

    pub(crate) fn update_spec_current_version(
        &self,
        spec: ExistingSpecRecord,
        version: u32,
        updated_at: String,
    ) -> Result<(), ProductStoreError> {
        match spec {
            ExistingSpecRecord::Story {
                path,
                record: mut story,
            } => {
                story.current_version = Some(version);
                story.updated_at = updated_at;
                write_json(&path, &story)
            }
            ExistingSpecRecord::Design {
                path,
                record: mut design,
            } => {
                design.current_version = Some(version);
                design.updated_at = updated_at;
                write_json(&path, &design)
            }
        }
    }
}

/// 校验聚合代码库 Story 视野（REQ-PLN-07）。fail-closed，不回落单仓 primary：
/// 1. `involved_repository_ids` 空 → Draft 态允许（AI 尚未产出，方案 X 阶段 1）；其余状态
///    （Confirmed 等）→ blocker `involved_repositories_undetermined`。
/// 2. 任一 involved id ∉ `effective_member_ids` → 越界成员 → blocker
///    `involved_repository_not_effective`。
/// 3. `focus_repository_id` 若给出但 ∉ involved → blocker `focus_repository_not_involved`。
fn validate_aggregate_story_scope(
    scope: &AggregateStorySpecScope,
    status: LifecycleConfirmationStatus,
) -> Result<(), ProductStoreError> {
    // Draft 态允许空 involved（AI 尚未产出，方案 X 阶段 1）；其余状态强制非空。
    if status != LifecycleConfirmationStatus::Draft && scope.involved_repository_ids.is_empty() {
        return Err(ProductStoreError::InvalidRecord {
            kind: "story_aggregate_scope",
            reason: "involved_repositories_undetermined: AI 未明确涉及仓库，不回落 primary"
                .to_string(),
        });
    }

    for involved in &scope.involved_repository_ids {
        if !scope.effective_member_ids.contains(involved) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "story_aggregate_scope",
                reason: format!(
                    "involved_repository_not_effective: {involved:?} 不在 effective_member_ids"
                ),
            });
        }
    }

    if let Some(focus) = scope.focus_repository_id
        && !scope.involved_repository_ids.contains(&focus)
    {
        return Err(ProductStoreError::InvalidRecord {
            kind: "story_aggregate_scope",
            reason: format!(
                "focus_repository_not_involved: {focus:?} 不在 involved_repository_ids"
            ),
        });
    }

    Ok(())
}

/// 校验聚合代码库 Design 视野（REQ-PLN-08）。fail-closed，不回落 issue.repo_id：
/// 1. `involved_repository_ids` 空 → Draft 态允许（AI 尚未产出，方案 X 阶段 1）；其余状态
///    （Confirmed 等）→ blocker `involved_repositories_undetermined`。
/// 2. 任一 involved id ∉ `effective_member_ids` → 越界成员 → blocker
///    `involved_repository_not_effective`。
/// 3. `change_order` 若给出，任一 id ∉ involved_repository_ids → blocker
///    `change_order_repository_not_involved`（执行顺序图必须覆盖全部涉及仓库）。
/// 4. `change_order` 若给出，出现重复顶点 → blocker `change_order_duplicate_repository`
///    （执行顺序图不得重复顶点）。
///
/// `change_order` 缺失（空）不强制 blocker：AI 可不给改动顺序；WorkItem 编译时若 Design 有
/// change_order 才作 depends_on 依据（Task 9 消费）。Design 不再读取 issue.repo_id 填充任何字段。
fn validate_aggregate_design_scope(
    scope: &AggregateDesignSpecScope,
    status: LifecycleConfirmationStatus,
) -> Result<(), ProductStoreError> {
    // Draft 态允许空 involved（AI 尚未产出，方案 X 阶段 1）；其余状态强制非空。
    if status != LifecycleConfirmationStatus::Draft && scope.involved_repository_ids.is_empty() {
        return Err(ProductStoreError::InvalidRecord {
            kind: "design_aggregate_scope",
            reason: "involved_repositories_undetermined: AI 未明确涉及仓库，不回落 issue.repo_id"
                .to_string(),
        });
    }

    for involved in &scope.involved_repository_ids {
        if !scope.effective_member_ids.contains(involved) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "design_aggregate_scope",
                reason: format!(
                    "involved_repository_not_effective: {involved:?} 不在 effective_member_ids"
                ),
            });
        }
    }

    for order_member in &scope.change_order {
        if !scope.involved_repository_ids.contains(order_member) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "design_aggregate_scope",
                reason: format!(
                    "change_order_repository_not_involved: {order_member:?} 不在 involved_repository_ids"
                ),
            });
        }
    }

    let mut seen = std::collections::HashSet::new();
    for order_member in &scope.change_order {
        if !seen.insert(*order_member) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "design_aggregate_scope",
                reason: format!(
                    "change_order_duplicate_repository: {order_member:?} 在 change_order 中重复"
                ),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::LogicalRepositoryId;
    use tempfile::TempDir;
    use uuid::Uuid;

    const TEST_PROJECT_ID: &str = "project_0001";
    const TEST_ISSUE_ID: &str = "issue_0001";
    const TEST_REPOSITORY_ID: &str = "repository_0001";
    const TEST_STORY_SPEC_ID: &str = "story_spec_0001";

    fn setup() -> (TempDir, LifecycleStore) {
        let tmp = TempDir::new().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        (tmp, store)
    }

    /// 构造一个聚合视野 Story（involved 空、Draft 态），作为 update 回写测试的前置。
    fn create_aggregate_story(store: &LifecycleStore) -> StorySpecRecord {
        store
            .create_story_spec(CreateStorySpecInput {
                project_id: TEST_PROJECT_ID.to_string(),
                issue_id: TEST_ISSUE_ID.to_string(),
                repository_id: TEST_REPOSITORY_ID.to_string(),
                title: "aggregate story".to_string(),
                aggregate_codebase: Some(AggregateStorySpecScope {
                    logical_codebase_ref: Uuid::from_u128(0x0100),
                    effective_member_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
                    involved_repository_ids: Vec::new(),
                    focus_repository_id: None,
                }),
            })
            .unwrap()
    }

    /// 构造一个聚合视野 Design（involved 空、Draft 态），作为 update 回写测试的前置。
    fn create_aggregate_design(store: &LifecycleStore) -> DesignSpecRecord {
        store
            .create_design_spec(CreateDesignSpecInput {
                project_id: TEST_PROJECT_ID.to_string(),
                issue_id: TEST_ISSUE_ID.to_string(),
                story_spec_ids: vec![TEST_STORY_SPEC_ID.to_string()],
                title: "aggregate design".to_string(),
                aggregate_codebase: Some(AggregateDesignSpecScope {
                    logical_codebase_ref: Uuid::from_u128(0x0100),
                    effective_member_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
                    involved_repository_ids: Vec::new(),
                    change_order: Vec::new(),
                }),
            })
            .unwrap()
    }

    #[test]
    fn update_story_spec_aggregate_writes_involved_in_draft() {
        let (_tmp, store) = setup();
        let story = create_aggregate_story(&store);
        let involved = LogicalRepositoryId(Uuid::from_u128(1));
        let scope = AggregateStorySpecScope {
            logical_codebase_ref: story.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![involved],
            involved_repository_ids: vec![involved], // AI 产出
            focus_repository_id: Some(involved),
        };

        let updated = store
            .update_story_spec_aggregate(&story.project_id, &story.issue_id, &story.id, &scope)
            .unwrap();
        assert_eq!(updated.involved_repository_ids, vec![involved]);
        assert_eq!(updated.focus_repository_id, Some(involved));
        assert_eq!(
            updated.confirmation_status,
            LifecycleConfirmationStatus::Draft
        );

        // 磁盘持久化一致（reload 校验回写落盘）。
        let reloaded = store
            .load_existing_spec(&story.project_id, &story.issue_id, &story.id)
            .unwrap();
        match reloaded {
            ExistingSpecRecord::Story { record, .. } => {
                assert_eq!(record.involved_repository_ids, vec![involved]);
                assert_eq!(record.focus_repository_id, Some(involved));
            }
            ExistingSpecRecord::Design { .. } => panic!("expected story spec"),
        }
    }

    #[test]
    fn update_story_spec_aggregate_rejects_involved_outside_effective() {
        let (_tmp, store) = setup();
        let story = create_aggregate_story(&store);
        let outside = LogicalRepositoryId(Uuid::from_u128(99));
        let scope = AggregateStorySpecScope {
            logical_codebase_ref: story.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
            involved_repository_ids: vec![outside],
            focus_repository_id: None,
        };

        let error = store
            .update_story_spec_aggregate(&story.project_id, &story.issue_id, &story.id, &scope)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
                if reason.contains("involved_repository_not_effective")),
            "越界 involved 应拒绝，错误: {error:?}"
        );

        // 校验失败不得落盘：reload 仍为空 involved。
        let reloaded = store
            .load_existing_spec(&story.project_id, &story.issue_id, &story.id)
            .unwrap();
        match reloaded {
            ExistingSpecRecord::Story { record, .. } => {
                assert!(record.involved_repository_ids.is_empty());
            }
            ExistingSpecRecord::Design { .. } => panic!("expected story spec"),
        }
    }

    #[test]
    fn update_story_spec_aggregate_rejects_focus_outside_involved() {
        let (_tmp, store) = setup();
        let story = create_aggregate_story(&store);
        let scope = AggregateStorySpecScope {
            logical_codebase_ref: story.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![
                LogicalRepositoryId(Uuid::from_u128(1)),
                LogicalRepositoryId(Uuid::from_u128(2)),
            ],
            involved_repository_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
            focus_repository_id: Some(LogicalRepositoryId(Uuid::from_u128(2))),
        };

        let error = store
            .update_story_spec_aggregate(&story.project_id, &story.issue_id, &story.id, &scope)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
                if reason.contains("focus_repository_not_involved")),
            "focus 必须在 involved 集合内，错误: {error:?}"
        );
    }

    #[test]
    fn update_story_spec_aggregate_rejects_when_confirmed() {
        let (_tmp, store) = setup();
        let story = create_aggregate_story(&store);
        store
            .update_spec_confirmation_status(
                &story.project_id,
                &story.issue_id,
                &story.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let involved = LogicalRepositoryId(Uuid::from_u128(1));
        let scope = AggregateStorySpecScope {
            logical_codebase_ref: story.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![involved],
            involved_repository_ids: vec![involved],
            focus_repository_id: Some(involved),
        };

        let error = store
            .update_story_spec_aggregate(&story.project_id, &story.issue_id, &story.id, &scope)
            .unwrap_err();
        assert!(
            matches!(
                error,
                ProductStoreError::Conflict {
                    kind: "story_aggregate_locked",
                    ..
                }
            ),
            "Confirmed 态应锁定，错误: {error:?}"
        );

        // 锁定后原值不变。
        let reloaded = store
            .load_existing_spec(&story.project_id, &story.issue_id, &story.id)
            .unwrap();
        match reloaded {
            ExistingSpecRecord::Story { record, .. } => {
                assert!(record.involved_repository_ids.is_empty());
                assert_eq!(
                    record.confirmation_status,
                    LifecycleConfirmationStatus::Confirmed
                );
            }
            ExistingSpecRecord::Design { .. } => panic!("expected story spec"),
        }
    }

    #[test]
    fn update_design_spec_aggregate_writes_involved_and_change_order_in_draft() {
        let (_tmp, store) = setup();
        let design = create_aggregate_design(&store);
        let involved = LogicalRepositoryId(Uuid::from_u128(1));
        let scope = AggregateDesignSpecScope {
            logical_codebase_ref: design.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![involved],
            involved_repository_ids: vec![involved], // AI 产出
            change_order: vec![involved],
        };

        let updated = store
            .update_design_spec_aggregate(&design.project_id, &design.issue_id, &design.id, &scope)
            .unwrap();
        assert_eq!(updated.involved_repository_ids, vec![involved]);
        assert_eq!(updated.change_order, vec![involved]);

        // 磁盘持久化一致（reload 校验回写落盘）。
        let reloaded = store
            .load_existing_spec(&design.project_id, &design.issue_id, &design.id)
            .unwrap();
        match reloaded {
            ExistingSpecRecord::Design { record, .. } => {
                assert_eq!(record.involved_repository_ids, vec![involved]);
                assert_eq!(record.change_order, vec![involved]);
            }
            ExistingSpecRecord::Story { .. } => panic!("expected design spec"),
        }
    }

    #[test]
    fn update_design_spec_aggregate_rejects_involved_outside_effective() {
        let (_tmp, store) = setup();
        let design = create_aggregate_design(&store);
        let outside = LogicalRepositoryId(Uuid::from_u128(99));
        let scope = AggregateDesignSpecScope {
            logical_codebase_ref: design.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
            involved_repository_ids: vec![outside],
            change_order: Vec::new(),
        };

        let error = store
            .update_design_spec_aggregate(&design.project_id, &design.issue_id, &design.id, &scope)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
                if reason.contains("involved_repository_not_effective")),
            "越界 involved 应拒绝，错误: {error:?}"
        );
    }

    #[test]
    fn update_design_spec_aggregate_rejects_change_order_outside_involved() {
        let (_tmp, store) = setup();
        let design = create_aggregate_design(&store);
        let scope = AggregateDesignSpecScope {
            logical_codebase_ref: design.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![
                LogicalRepositoryId(Uuid::from_u128(1)),
                LogicalRepositoryId(Uuid::from_u128(2)),
            ],
            involved_repository_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
            change_order: vec![LogicalRepositoryId(Uuid::from_u128(2))],
        };

        let error = store
            .update_design_spec_aggregate(&design.project_id, &design.issue_id, &design.id, &scope)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
                if reason.contains("change_order_repository_not_involved")),
            "change_order 必须在 involved 集合内，错误: {error:?}"
        );
    }

    #[test]
    fn update_design_spec_aggregate_rejects_when_confirmed() {
        let (_tmp, store) = setup();
        let design = create_aggregate_design(&store);
        store
            .update_spec_confirmation_status(
                &design.project_id,
                &design.issue_id,
                &design.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let involved = LogicalRepositoryId(Uuid::from_u128(1));
        let scope = AggregateDesignSpecScope {
            logical_codebase_ref: design.logical_codebase_ref.unwrap(),
            effective_member_ids: vec![involved],
            involved_repository_ids: vec![involved],
            change_order: vec![involved],
        };

        let error = store
            .update_design_spec_aggregate(&design.project_id, &design.issue_id, &design.id, &scope)
            .unwrap_err();
        assert!(
            matches!(
                error,
                ProductStoreError::Conflict {
                    kind: "design_aggregate_locked",
                    ..
                }
            ),
            "Confirmed 态应锁定，错误: {error:?}"
        );
    }

    #[test]
    fn validate_story_scope_draft_allows_empty_involved() {
        let scope = AggregateStorySpecScope {
            logical_codebase_ref: Uuid::nil(),
            effective_member_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
            involved_repository_ids: vec![], // 空
            focus_repository_id: None,
        };
        // Draft 态允许空 involved（AI 尚未产出）
        assert!(validate_aggregate_story_scope(&scope, LifecycleConfirmationStatus::Draft).is_ok());
        // Confirmed 态强制非空
        assert!(
            validate_aggregate_story_scope(&scope, LifecycleConfirmationStatus::Confirmed).is_err()
        );
    }

    #[test]
    fn validate_design_scope_draft_allows_empty_involved() {
        let scope = AggregateDesignSpecScope {
            logical_codebase_ref: Uuid::nil(),
            effective_member_ids: vec![LogicalRepositoryId(Uuid::from_u128(1))],
            involved_repository_ids: vec![],
            change_order: vec![],
        };
        assert!(
            validate_aggregate_design_scope(&scope, LifecycleConfirmationStatus::Draft).is_ok()
        );
        assert!(
            validate_aggregate_design_scope(&scope, LifecycleConfirmationStatus::Confirmed)
                .is_err()
        );
    }
}
