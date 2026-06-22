use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignContextCapabilities, IssueRecord, IssueWorkItemDependencyEdge, IssueWorkItemPlan,
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleWorkItemRecord,
    OutlineContextBlockerResolution, ProviderName, RepositoryProfile, RepositoryProfileConfidence,
    RepositoryRecord, VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationManualCheck, VerificationPlan, VerificationScope,
    WorkItemContextBudget, WorkItemDraftCandidate, WorkItemDraftRecord,
    WorkItemExecutionPlanStatus, WorkItemGenerationMode, WorkItemKind, WorkItemPlanOutline,
    WorkItemPlanStatus, WorkItemStatus,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

const WORK_ITEM_SPLIT_OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "repository_profile": {
      "type": "object",
      "properties": {
        "confidence": { "type": "string" },
        "detected_layers": { "type": "array", "items": { "type": "string" } },
        "split_recommendation": { "type": "string" },
        "languages": { "type": "array", "items": { "type": "string" } },
        "frameworks": { "type": "array", "items": { "type": "string" } },
        "package_managers": { "type": "array", "items": { "type": "string" } },
        "test_frameworks": { "type": "array", "items": { "type": "string" } },
        "build_systems": { "type": "array", "items": { "type": "string" } },
        "verification_capabilities": { "type": "array", "items": { "type": "string" } },
        "uncertainties": { "type": "array", "items": { "type": "string" } }
      },
      "required": ["confidence", "detected_layers", "split_recommendation"]
    },
    "plan": {
      "type": "object",
      "properties": {
        "work_item_ids": { "type": "array", "items": { "type": "string" } },
        "dependency_graph": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "from_work_item_id": { "type": "string" },
              "to_work_item_id": { "type": "string" }
            },
            "required": ["from_work_item_id", "to_work_item_id"]
          }
        }
      }
    },
    "work_items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "kind": { "type": "string" },
          "sequence_hint": { "type": "integer" },
          "depends_on": { "type": "array", "items": { "type": "integer" } },
          "exclusive_write_scopes": { "type": "array", "items": { "type": "string" } },
          "forbidden_write_scopes": { "type": "array", "items": { "type": "string" } },
          "context_budget": {
            "type": "object",
            "properties": {
              "target_context_k": { "type": "string" },
              "max_summary_chars": { "type": "integer" },
              "max_handoff_chars": { "type": "integer" },
              "max_code_context_chars": { "type": "integer" },
              "max_context_file_refs": { "type": "integer" },
              "max_traceability_refs": { "type": "integer" },
              "max_dependency_handoffs": { "type": "integer" }
            }
          },
          "required_handoff_from": { "type": "array", "items": { "type": "string" } },
          "require_execution_plan_confirm": { "type": "boolean" }
        },
        "required": ["title", "kind"]
      }
    },
    "verification_plans": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "scope": { "type": "string" },
          "commands": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "id": { "type": "string" },
                "label": { "type": "string" },
                "command": { "type": "string" },
                "cwd": { "type": "string" },
                "purpose": { "type": "string" },
                "required": { "type": "boolean" },
                "timeout_seconds": { "type": "integer" },
                "safety": { "type": "string" }
              },
              "required": ["label", "command", "purpose"]
            }
          },
          "manual_checks": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "id": { "type": "string" },
                "label": { "type": "string" },
                "instructions": { "type": "string" },
                "required": { "type": "boolean" }
              },
              "required": ["label", "instructions"]
            }
          },
          "required_gates": { "type": "array", "items": { "type": "string" } },
          "risk_notes": { "type": "array", "items": { "type": "string" } },
          "confidence": { "type": "string" },
          "fallback_policy": { "type": "string" }
        }
      }
    }
  },
  "required": ["repository_profile", "work_items", "verification_plans"]
}"#;

const WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "outline": {
      "type": "object",
      "required": [
        "id",
        "project_id",
        "issue_id",
        "source_story_spec_ids",
        "source_design_spec_ids",
        "strategy_summary",
        "work_item_outlines",
        "dependency_graph",
        "risks",
        "handoff_strategy",
        "status"
      ]
    },
    "context_blockers": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["code", "message", "needed_context"]
      }
    }
  },
  "additionalProperties": false
}"#;

#[derive(Debug, Clone)]
pub struct WorkItemSplitProviderOutput {
    pub repository_profile: RepositoryProfile,
    pub plan: IssueWorkItemPlan,
    pub work_items: Vec<LifecycleWorkItemRecord>,
    pub verification_plans: Vec<VerificationPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanContextBlocker {
    pub code: String,
    pub message: String,
    pub needed_context: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineAuthorOutput {
    pub outline: Option<WorkItemPlanOutline>,
    pub context_blockers: Vec<WorkItemPlanContextBlocker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemDraftInvocation {
    pub prompt: String,
    pub sentinel_nonce: String,
}

/// 被重做的 WorkItem 规格：旧 id + 用户反馈。
#[derive(Debug, Clone)]
pub struct RedoSpec {
    pub old_id: String,
    pub feedback: String,
}

/// DAG 重连：把 graph 中对旧 id 的引用改为新 id。
///
/// `id_mapping`: old_id → new_id。只重写映射中存在的 id，未映射的边原样保留。
pub fn repatch_dependencies(
    graph: &[IssueWorkItemDependencyEdge],
    id_mapping: &HashMap<String, String>,
) -> Vec<IssueWorkItemDependencyEdge> {
    graph
        .iter()
        .map(|edge| IssueWorkItemDependencyEdge {
            from_work_item_id: id_mapping
                .get(&edge.from_work_item_id)
                .cloned()
                .unwrap_or_else(|| edge.from_work_item_id.clone()),
            to_work_item_id: id_mapping
                .get(&edge.to_work_item_id)
                .cloned()
                .unwrap_or_else(|| edge.to_work_item_id.clone()),
        })
        .collect()
}

#[derive(Clone)]
pub struct WorkItemSplitEngine {
    provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
}

#[derive(Debug)]
struct ProviderInvocationResult {
    structured_output: serde_json::Value,
    run_ref: String,
}

#[derive(Debug, Clone)]
pub struct WorkItemSplitInvocation {
    pub prompt: String,
    pub provider_type: ProviderType,
    pub worktree_path: String,
    pub author_provider: ProviderName,
    pub sentinel_nonce: String,
}

impl WorkItemSplitEngine {
    pub fn new(provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>) -> Self {
        Self { provider_adapter }
    }

    pub fn build_generate_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
    ) -> ApiResult<WorkItemSplitInvocation> {
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;

        let repository_structure = summarize_repository_structure(&repository.path);
        let prompt = build_split_prompt(
            request,
            issue,
            repository,
            &story_context,
            &design_context,
            &repository_structure,
        );

        Ok(WorkItemSplitInvocation {
            sentinel_nonce: prompt_nonce(&prompt),
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
        })
    }

    pub fn build_outline_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        context_resolutions: &[OutlineContextBlockerResolution],
    ) -> ApiResult<WorkItemSplitInvocation> {
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;
        let repository_structure = summarize_repository_structure(&repository.path);
        let capabilities = merge_design_context_capabilities(&design_context);
        let gaps = design_context_gaps(&capabilities);
        let (prompt, sentinel_nonce) = build_outline_prompt_with_nonce(
            request,
            issue,
            repository,
            &story_context,
            &design_context,
            &repository_structure,
            &gaps,
            context_resolutions,
        );

        Ok(WorkItemSplitInvocation {
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
            sentinel_nonce,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_revision_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[RedoSpec],
    ) -> ApiResult<WorkItemSplitInvocation> {
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;

        let repository_structure = summarize_repository_structure(&repository.path);
        let prompt = build_revision_prompt(
            request,
            issue,
            repository,
            retained,
            redo_specs,
            &story_context,
            &design_context,
            &repository_structure,
        );

        Ok(WorkItemSplitInvocation {
            sentinel_nonce: prompt_nonce(&prompt),
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
        })
    }

    pub fn complete_generate_from_structured_output(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: &ProviderName,
        prompt: &str,
        structured_output: serde_json::Value,
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        parse_provider_output(
            lifecycle,
            request,
            issue,
            repository,
            run_ref,
            &structured_output,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_revision_from_structured_output(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: &ProviderName,
        prompt: &str,
        structured_output: serde_json::Value,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[RedoSpec],
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        if retained.is_empty() && redo_specs.is_empty() {
            return parse_provider_output(
                lifecycle,
                request,
                issue,
                repository,
                run_ref,
                &structured_output,
            );
        }

        materialize_revision_output(
            lifecycle,
            request,
            issue,
            repository,
            run_ref,
            &structured_output,
            retained,
            redo_specs,
        )
    }

    pub async fn generate(
        &self,
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let invocation = Self::build_generate_invocation(
            request,
            lifecycle,
            issue,
            repository,
            author_provider,
        )?;

        let provider_output = self
            .invoke_provider(
                &invocation.prompt,
                repository,
                invocation.author_provider.clone(),
                lifecycle,
                issue,
            )
            .await?;

        parse_provider_output(
            lifecycle,
            request,
            issue,
            repository,
            provider_output.run_ref,
            &provider_output.structured_output,
        )
    }

    async fn invoke_provider(
        &self,
        prompt: &str,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
    ) -> ApiResult<ProviderInvocationResult> {
        let provider_type = provider_name_to_type(&author_provider);
        let worktree_path = repository.path.to_string_lossy().to_string();
        let adapter_input = AdapterInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree_path),
            prompt: prompt.to_string(),
            context_files: Vec::new(),
            output_schema: WORK_ITEM_SPLIT_OUTPUT_SCHEMA.to_string(),
            timeout: 3 * 60 * 60,
            max_retries: 1,
        };

        let adapter = self.provider_adapter.clone();
        let output = tokio::task::spawn_blocking(move || adapter.run(&adapter_input))
            .await
            .map_err(|error| {
                ApiError::runtime(
                    "work_item_split_provider_panic",
                    "provider adapter panicked",
                    json!({"details": error.to_string()}),
                )
            })?
            .map_err(map_provider_adapter_error)?;

        let structured_output = output.structured_output.ok_or_else(|| {
            ApiError::runtime(
                "work_item_split_provider_output_invalid",
                "provider did not return structured output",
                json!({}),
            )
        })?;

        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                &author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        Ok(ProviderInvocationResult {
            structured_output,
            run_ref,
        })
    }

    /// Revision：保留项 + redo-only 重做项 + DAG repatch。
    ///
    /// 局部重做时，prompt 注入"保留项清单（只作上下文，不允许重写）+ 重做项及反馈"，
    /// provider 只输出 redo 项。后端负责：
    /// 1. retained 原记录直接合并；
    /// 2. 为 redo 输出分配新 id / verification_plan id；
    /// 3. 用 redo_specs 顺序建立 old_id -> new_id 映射；
    /// 4. `repatch_dependencies` 把 dependency_graph 与 retained/redo 的 depends_on 中旧 id 改成新 id。
    ///
    /// retained/redo_specs 均空时表示整组 review/AutoRevision，退化为完整 split 输出解析。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_revision(
        &self,
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[RedoSpec],
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let invocation = Self::build_revision_invocation(
            request,
            lifecycle,
            issue,
            repository,
            author_provider,
            retained,
            redo_specs,
        )?;

        let provider_output = self
            .invoke_provider(
                &invocation.prompt,
                repository,
                invocation.author_provider,
                lifecycle,
                issue,
            )
            .await?;
        let structured = &provider_output.structured_output;

        if retained.is_empty() && redo_specs.is_empty() {
            return parse_provider_output(
                lifecycle,
                request,
                issue,
                repository,
                provider_output.run_ref,
                structured,
            );
        }

        materialize_revision_output(
            lifecycle,
            request,
            issue,
            repository,
            provider_output.run_ref,
            structured,
            retained,
            redo_specs,
        )
    }
}

fn provider_name_to_type(name: &ProviderName) -> ProviderType {
    match name {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}

fn collect_story_context(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<Vec<String>> {
    let project_id = &issue.project_id;
    let issue_id = &issue.id;
    let story_specs = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;

    request
        .story_spec_ids
        .iter()
        .map(|id| {
            let spec = story_specs.iter().find(|s| &s.id == id).ok_or_else(|| {
                ApiError::runtime("story_spec_not_found", "story spec not found", json!({}))
            })?;
            let markdown = latest_markdown(lifecycle, project_id, issue_id, id)?;
            Ok(format!(
                "Story Spec: {} ({})\n{}",
                spec.title, spec.id, markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
}

fn collect_design_context(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<Vec<String>> {
    let project_id = &issue.project_id;
    let issue_id = &issue.id;
    let design_specs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;

    request
        .design_spec_ids
        .iter()
        .map(|id| {
            let spec = design_specs.iter().find(|s| &s.id == id).ok_or_else(|| {
                ApiError::runtime("design_spec_not_found", "design spec not found", json!({}))
            })?;
            let markdown = latest_markdown(lifecycle, project_id, issue_id, id)?;
            Ok(format!(
                "Design Spec: {} ({})\n{}",
                spec.title, spec.id, markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
}

pub fn design_context_capabilities_for_request(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<DesignContextCapabilities> {
    let design_context = collect_design_context(lifecycle, request, issue)?;
    Ok(merge_design_context_capabilities(&design_context))
}

fn latest_markdown(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> ApiResult<String> {
    let versions = lifecycle
        .list_versions(project_id, issue_id, entity_id)
        .map_err(product_store_api_error)?;
    Ok(versions
        .into_iter()
        .max_by_key(|v| v.version)
        .map(|v| v.markdown)
        .unwrap_or_else(|| "(no version)".to_string()))
}

fn summarize_repository_structure(path: &Path) -> String {
    let mut entries = Vec::new();
    if let Ok(reader) = std::fs::read_dir(path) {
        for entry in reader.flatten() {
            if let Ok(metadata) = entry.metadata() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" {
                    continue;
                }
                let kind = if metadata.is_dir() { "dir" } else { "file" };
                entries.push(format!("{kind}: {name}"));
            }
        }
    }
    entries.sort();
    entries.truncate(30);
    if entries.is_empty() {
        "(empty repository)".to_string()
    } else {
        entries.join("\n")
    }
}

pub fn extract_design_context_capabilities(markdown: &str) -> DesignContextCapabilities {
    let normalized = markdown_headings(markdown).join("\n").to_lowercase();
    DesignContextCapabilities {
        has_architecture: contains_any(&normalized, &["架构概览", "系统架构", "architecture"]),
        has_module_breakdown: contains_any(
            &normalized,
            &["模块划分", "模块拆分", "modules", "module breakdown"],
        ),
        has_tech_stack: contains_any(
            &normalized,
            &["技术选型", "技术栈", "tech stack", "technology"],
        ),
        has_test_strategy: contains_any(
            &normalized,
            &["测试框架", "测试策略", "test strategy", "testing strategy"],
        ),
        has_key_paths: contains_any(
            &normalized,
            &[
                "关键目录结构",
                "关键路径",
                "key paths",
                "directory structure",
            ],
        ),
    }
}

pub fn design_context_gaps(capabilities: &DesignContextCapabilities) -> Vec<String> {
    let mut gaps = Vec::new();
    if !capabilities.has_architecture {
        gaps.push("missing_architecture".to_string());
    }
    if !capabilities.has_module_breakdown {
        gaps.push("missing_module_breakdown".to_string());
    }
    if !capabilities.has_tech_stack {
        gaps.push("missing_tech_stack".to_string());
    }
    if !capabilities.has_test_strategy {
        gaps.push("missing_test_strategy".to_string());
    }
    if !capabilities.has_key_paths {
        gaps.push("missing_key_paths".to_string());
    }
    gaps
}

fn merge_design_context_capabilities(design_context: &[String]) -> DesignContextCapabilities {
    design_context.iter().fold(
        DesignContextCapabilities {
            has_architecture: false,
            has_module_breakdown: false,
            has_tech_stack: false,
            has_test_strategy: false,
            has_key_paths: false,
        },
        |mut merged, markdown| {
            let current = extract_design_context_capabilities(markdown);
            merged.has_architecture |= current.has_architecture;
            merged.has_module_breakdown |= current.has_module_breakdown;
            merged.has_tech_stack |= current.has_tech_stack;
            merged.has_test_strategy |= current.has_test_strategy;
            merged.has_key_paths |= current.has_key_paths;
            merged
        },
    )
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn markdown_headings(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                Some(trimmed.trim_matches('#').trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn build_outline_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
    design_context_gaps: &[String],
    context_resolutions: &[OutlineContextBlockerResolution],
) -> String {
    build_outline_prompt_with_nonce(
        request,
        issue,
        repository,
        story_context,
        design_context,
        repository_structure,
        design_context_gaps,
        context_resolutions,
    )
    .0
}

#[allow(clippy::too_many_arguments)]
fn build_outline_prompt_with_nonce(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
    design_context_gaps: &[String],
    context_resolutions: &[OutlineContextBlockerResolution],
) -> (String, String) {
    let nonce = structured_output_nonce();
    let prompt = format!(
        "你是 Aria 的 WorkItemPlan Outline Planner。请基于以下输入生成第一阶段 WorkItemPlan Outline。\n\n\
         [issue]\n\
         title: {title}\n\
         description: {description}\n\n\
         [repository]\n\
         id: {repo_id}\n\
         path: {repo_path}\n\n\
         [confirmed_story_specs]\n{story_context}\n\n\
         [confirmed_design_specs]\n{design_context}\n\n\
         [repository_structure_summary]\n{repository_structure}\n\n\
         [design_context_gaps]\n{design_context_gaps}\n\n\
         [context_blocker_resolutions]\n{context_resolutions}\n\n\
         [user_options]\n\
         include_integration_tests: {include_integration_tests}\n\
         include_e2e_tests: {include_e2e_tests}\n\
         force_frontend_backend_split: {force_frontend_backend_split}\n\
         require_execution_plan_confirm: {require_execution_plan_confirm}\n\n\
         [strict_output_contract]\n\
         只能输出 WorkItemPlan Outline，不得输出完整 Work Item。\n\
         不得输出 VerificationPlan、verification_plan、verification_plans、work_item_id、work_item_ids。\n\
         不得输出 repository_profile，不得输出 parallel_groups。\n\
         不得修改仓库文件，不得创建计划文档。\n\
         如果无法补齐模块边界、关键路径或测试策略，请不要猜测完整拆分；请在 context_blockers 数组中写明需要用户补充的上下文。\n\
         可以在最终结构化 JSON 前输出简短、可读的规划过程，供 Workbench 流式展示。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出。\n\n\
         {schema}",
        title = issue.title,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        design_context_gaps = format_string_list(design_context_gaps),
        context_resolutions = format_context_resolutions(context_resolutions),
        include_integration_tests = request.include_integration_tests.unwrap_or(false),
        include_e2e_tests = request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split = request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm = request.require_execution_plan_confirm.unwrap_or(false),
        nonce = nonce,
        schema = WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
    );
    (prompt, nonce)
}

fn format_string_list(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values
            .iter()
            .map(|value| format!("- {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_context_resolutions(resolutions: &[OutlineContextBlockerResolution]) -> String {
    if resolutions.is_empty() {
        return "(none)".to_string();
    }

    resolutions
        .iter()
        .map(|resolution| {
            let summary = resolution.summary.as_deref().unwrap_or("(none)");
            format!(
                "- blocker_node_id: {blocker}\n  resolution_node_id: {resolution_node}\n  artifact_ref: {artifact_ref}\n  estimated_tokens: {tokens}\n  summary: {summary}",
                blocker = resolution.blocker_node_id,
                resolution_node = resolution.resolution_node_id,
                artifact_ref = resolution.resolution_artifact_ref,
                tokens = resolution.estimated_tokens,
                summary = summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_split_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
) -> String {
    let nonce = structured_output_nonce();
    let revision_feedback_section = request
        .revision_feedback
        .as_deref()
        .map(|feedback| {
            format!(
                "[revision_feedback]\n\
                 Previous validation found the following issues; please fix them in the regenerated plan:\n{feedback}\n\n"
            )
        })
        .unwrap_or_default();

    format!(
        "你是 Aria 的 Work Item Splitter。请基于以下输入生成 IssueWorkItemPlan 候选拆分。\n\n\
         [issue]\n\
         title: {title}\n\
         description: {description}\n\n\
         [repository]\n\
         id: {repo_id}\n\
         path: {repo_path}\n\n\
         [confirmed_story_specs]\n{story_context}\n\n\
         [confirmed_design_specs]\n{design_context}\n\n\
         [repository_structure_summary]\n{repository_structure}\n\n\
         {revision_feedback_section}\n\
         [openspec_constraint_summary]\n\
         story_spec_ids: {story_ids}\n\
         design_spec_ids: {design_ids}\n\n\
         [user_options]\n\
         include_integration_tests: {include_integration_tests}\n\
         include_e2e_tests: {include_e2e_tests}\n\
         force_frontend_backend_split: {force_frontend_backend_split}\n\
         require_execution_plan_confirm: {require_execution_plan_confirm}\n\n\
         [output_schema]\n\
         可以在最终结构化 JSON 前输出简短、可读的拆分过程，供 Workbench 流式展示。\n\
         长时间分析、探索代码库或自动修正前，先输出一行简短可读状态，供 Workbench 流式展示；不要等待所有工具调用结束后才给第一段说明。\n\
         如果需要执行多步代码库探索，每完成一组探索后输出一句当前发现摘要。\n\
         这些可读状态必须位于最终 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> 之前；最终结构化 JSON 仍只放在最后一个 sentinel block 中。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出。\n\
         work_items 数组顺序即执行顺序；depends_on 使用同数组中的 0-based 索引。verification_plans 数组与 work_items 一一对应。\n\
         每个 work_item 必须包含 `kind` 字段（不要写成 `type`），合法取值为以下之一：backend、frontend、integration、e2e、docs、infra、other。\n\n\
         {schema}",
        title = issue.title,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        revision_feedback_section = revision_feedback_section,
        story_ids = request.story_spec_ids.join(", "),
        design_ids = request.design_spec_ids.join(", "),
        include_integration_tests = request.include_integration_tests.unwrap_or(false),
        include_e2e_tests = request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split = request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm = request.require_execution_plan_confirm.unwrap_or(false),
        nonce = nonce,
        schema = WORK_ITEM_SPLIT_OUTPUT_SCHEMA,
    )
}

fn structured_output_nonce() -> String {
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

fn prompt_nonce(prompt: &str) -> String {
    prompt
        .split_once("<ARIA_STRUCTURED_OUTPUT nonce=\"")
        .and_then(|(_, tail)| tail.split_once('"'))
        .map(|(nonce, _)| nonce.to_string())
        .unwrap_or_default()
}

fn work_item_kind_text(kind: &WorkItemKind) -> &'static str {
    match kind {
        WorkItemKind::Backend => "backend",
        WorkItemKind::Frontend => "frontend",
        WorkItemKind::Integration => "integration",
        WorkItemKind::E2e => "e2e",
        WorkItemKind::Docs => "docs",
        WorkItemKind::Infra => "infra",
        WorkItemKind::Other => "other",
    }
}

#[allow(clippy::too_many_arguments)]
fn build_revision_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    retained: &[LifecycleWorkItemRecord],
    redo_specs: &[RedoSpec],
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
) -> String {
    if retained.is_empty() && redo_specs.is_empty() {
        return build_split_prompt(
            request,
            issue,
            repository,
            story_context,
            design_context,
            repository_structure,
        );
    }

    let nonce = structured_output_nonce();
    let retained_section = if retained.is_empty() {
        "(无)".to_string()
    } else {
        retained
            .iter()
            .map(|wi| {
                format!(
                    "- {} [{}] {}",
                    wi.id,
                    work_item_kind_text(&wi.kind),
                    wi.title
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let redo_section = redo_specs
        .iter()
        .map(|r| format!("- {}: {}", r.old_id, r.feedback))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "你是 Aria 的 Work Item Splitter。当前请求是局部重做（revision）。请基于以下输入，仅输出需要重做的 work_items 与 verification_plans。\n\n\
         [issue]\n\
         title: {title}\n\
         description: {description}\n\n\
         [repository]\n\
         id: {repo_id}\n\
         path: {repo_path}\n\n\
         [confirmed_story_specs]\n{story_context}\n\n\
         [confirmed_design_specs]\n{design_context}\n\n\
         [repository_structure_summary]\n{repository_structure}\n\n\
         [retained_work_items]\n\
         以下 WorkItem 必须保留，不得在输出中重写：\n{retained_section}\n\n\
         [redo_work_items]\n\
         以下 WorkItem 需要按用户反馈重做，请只输出这些项：\n{redo_section}\n\n\
         [output_schema]\n\
         可以在最终结构化 JSON 前输出简短、可读的拆分过程，供 Workbench 流式展示。\n\
         长时间分析、探索代码库或自动修正前，先输出一行简短可读状态，供 Workbench 流式展示；不要等待所有工具调用结束后才给第一段说明。\n\
         如果需要执行多步代码库探索，每完成一组探索后输出一句当前发现摘要。\n\
         这些可读状态必须位于最终 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> 之前；最终结构化 JSON 仍只放在最后一个 sentinel block 中。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出 redo-only 结果。\n\
         work_items 数组必须且仅包含重做项，顺序对应 redo_work_items 列表；verification_plans 与 work_items 一一对应；depends_on 使用 0-based 索引。\n\
         每个 work_item 必须包含 `kind` 字段（不要写成 `type`），合法取值为以下之一：backend、frontend、integration、e2e、docs、infra、other。\n\n\
         {schema}",
        title = issue.title,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        retained_section = retained_section,
        redo_section = redo_section,
        nonce = nonce,
        schema = WORK_ITEM_SPLIT_OUTPUT_SCHEMA,
    )
}

fn map_provider_adapter_error(error: ProviderAdapterError) -> ApiError {
    ApiError::runtime(
        "work_item_split_provider_error",
        &error.details,
        json!({
            "provider_error_code": error.code,
            "stdout": error.stdout,
            "stderr": error.stderr,
            "exit_code": error.exit_code,
        }),
    )
}

fn product_store_api_error(error: crate::product::json_store::ProductStoreError) -> ApiError {
    ApiError::runtime("product_store_error", error.to_string(), json!({}))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderOutlineAuthorOutput {
    #[serde(default)]
    outline: Option<WorkItemPlanOutline>,
    #[serde(default)]
    context_blockers: Vec<WorkItemPlanContextBlocker>,
}

pub fn parse_work_item_plan_outline_output(
    value: serde_json::Value,
) -> ApiResult<OutlineAuthorOutput> {
    if let Some(field) = forbidden_outline_field(&value) {
        return Err(ApiError::validation_with_details(
            "outline_forbidden_field",
            format!("WorkItemPlan Outline output must not contain `{field}`"),
            json!({ "field": field }),
        ));
    }

    let output: ProviderOutlineAuthorOutput = serde_json::from_value(value).map_err(|error| {
        ApiError::runtime(
            "outline_parse_error",
            format!("failed to parse WorkItemPlan Outline output: {error}"),
            json!({}),
        )
    })?;

    if output.outline.is_none() && output.context_blockers.is_empty() {
        return Err(ApiError::validation(
            "outline_empty_output",
            "WorkItemPlan Outline output must include outline or context_blockers",
        ));
    }

    Ok(OutlineAuthorOutput {
        outline: output.outline,
        context_blockers: output.context_blockers,
    })
}

pub fn build_work_item_draft_invocation(
    outline: &WorkItemPlanOutline,
    current_outline_id: &str,
    generation_mode: WorkItemGenerationMode,
    accepted_drafts: &[WorkItemDraftRecord],
    feedback: Option<&str>,
) -> ApiResult<WorkItemDraftInvocation> {
    let current_outline = outline
        .work_item_outlines
        .iter()
        .find(|item| item.outline_id == current_outline_id)
        .ok_or_else(|| {
            ApiError::validation_with_details(
                "work_item_draft_outline_missing",
                format!("current outline `{current_outline_id}` not found"),
                json!({ "outline_id": current_outline_id }),
            )
        })?;
    let dependency_ids: std::collections::HashSet<&str> = current_outline
        .depends_on
        .iter()
        .map(String::as_str)
        .collect();
    let direct_dependencies: Vec<&WorkItemDraftRecord> = accepted_drafts
        .iter()
        .filter(|draft| dependency_ids.contains(draft.outline_id.as_str()))
        .collect();
    let other_previous: Vec<&WorkItemDraftRecord> = accepted_drafts
        .iter()
        .filter(|draft| !dependency_ids.contains(draft.outline_id.as_str()))
        .collect();
    let nonce = structured_output_nonce();
    let prompt = build_work_item_draft_prompt(
        outline,
        current_outline,
        generation_mode,
        &direct_dependencies,
        &other_previous,
        feedback,
        &nonce,
    );

    Ok(WorkItemDraftInvocation {
        prompt,
        sentinel_nonce: nonce,
    })
}

pub fn parse_work_item_draft_output(value: serde_json::Value) -> ApiResult<WorkItemDraftCandidate> {
    if value.get("drafts").is_some() || value.get("work_items").is_some() {
        return Err(ApiError::validation(
            "work_item_draft_multiple_items",
            "single item draft output must contain exactly one draft",
        ));
    }
    if let Some(field) = forbidden_work_item_draft_field(&value) {
        return Err(ApiError::validation_with_details(
            "work_item_draft_forbidden_field",
            format!("WorkItemDraftCandidate output must not contain `{field}`"),
            json!({ "field": field }),
        ));
    }

    let draft_value = value.get("draft").cloned().unwrap_or(value);
    serde_json::from_value(draft_value).map_err(|error| {
        ApiError::runtime(
            "work_item_draft_parse_error",
            format!("failed to parse WorkItemDraftCandidate output: {error}"),
            json!({}),
        )
    })
}

fn build_work_item_draft_prompt(
    outline: &WorkItemPlanOutline,
    current_outline: &crate::product::models::WorkItemOutline,
    generation_mode: WorkItemGenerationMode,
    direct_dependencies: &[&WorkItemDraftRecord],
    other_previous: &[&WorkItemDraftRecord],
    feedback: Option<&str>,
    nonce: &str,
) -> String {
    let outline_json = serde_json::to_string_pretty(outline).unwrap_or_else(|_| "{}".to_string());
    let current_outline_json =
        serde_json::to_string_pretty(current_outline).unwrap_or_else(|_| "{}".to_string());
    let direct_dependency_json =
        serde_json::to_string_pretty(direct_dependencies).unwrap_or_else(|_| "[]".to_string());
    let previous_summaries = other_previous
        .iter()
        .map(|draft| {
            format!(
                "- {} / {}: {}",
                draft.outline_id, draft.draft_id, draft.candidate.handoff_summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let feedback_section = feedback
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("\n[user_or_reviewer_feedback]\n{value}\n"))
        .unwrap_or_default();
    let mode = match generation_mode {
        WorkItemGenerationMode::Serial => "serial",
        WorkItemGenerationMode::Batch => "batch",
    };

    format!(
        "你是 Aria 的 Work Item Draft author。请只为当前 WorkItemPlan Outline 中的一个 item 生成 WorkItemDraftCandidate。\n\n\
         [generation_mode]\n{mode}\n\n\
         [confirmed_outline]\n{outline_json}\n\n\
         [current_work_item_outline]\n{current_outline_json}\n\n\
         [直接依赖 draft 完整内容]\n{direct_dependency_json}\n\n\
         [其他已 accepted draft 摘要]\n{previous_summaries}\n\
         {feedback_section}\n\
         [hard_rules]\n\
         - 只能输出一个 WorkItemDraftCandidate，字段必须对应当前 outline_id `{outline_id}`。\n\
         - 不得修改 Outline，不得新增、删除或重命名 outline。\n\
         - 不得输出 work_item_id、draft_id、status、generated_from_node_id、accepted_at、batch_id 等后端状态字段。\n\
         - verification_plan 可以是对象，但 required_gates 只能引用同一 verification_plan 内的 command/manual_check id。\n\
         - 可以先输出简短可读状态；最终 JSON 必须放在最后一个 nonce sentinel block 中，不要输出 Markdown code fence。\n\n\
         [output]\n\
         <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{{\"draft\":{{\"outline_id\":\"{outline_id}\",\"title\":\"...\",\"kind\":\"backend|frontend|integration|e2e|docs|infra|other\",\"goal\":\"...\",\"implementation_context\":\"...\",\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on_outline_ids\":[],\"required_handoff_from_outline_ids\":[],\"handoff_summary\":\"...\",\"verification_plan\":{{}}}}}}</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">",
        outline_id = current_outline.outline_id,
    )
}

fn forbidden_work_item_draft_field(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_forbidden_work_item_draft_key(key) {
                    return Some(key.clone());
                }
                if let Some(field) = forbidden_work_item_draft_field(value) {
                    return Some(field);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(forbidden_work_item_draft_field),
        _ => None,
    }
}

fn is_forbidden_work_item_draft_key(key: &str) -> bool {
    matches!(
        key,
        "work_item_id"
            | "draft_id"
            | "status"
            | "generated_from_node_id"
            | "accepted_at"
            | "batch_id"
            | "superseded_by_draft_id"
            | "supersede_reason"
            | "copied_from_draft_id"
            | "review_node_id"
            | "review_verdict_ref"
    )
}

fn forbidden_outline_field(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_forbidden_outline_key(key) {
                    return Some(key.clone());
                }
                if let Some(field) = forbidden_outline_field(value) {
                    return Some(field);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(forbidden_outline_field),
        _ => None,
    }
}

fn is_forbidden_outline_key(key: &str) -> bool {
    matches!(
        key,
        "work_items"
            | "work_item_id"
            | "work_item_ids"
            | "verification_plan"
            | "verification_plans"
            | "repository_profile"
            | "parallel_groups"
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderRepositoryProfile {
    confidence: String,
    detected_layers: Vec<String>,
    split_recommendation: String,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    frameworks: Vec<String>,
    #[serde(default)]
    package_managers: Vec<String>,
    #[serde(default)]
    test_frameworks: Vec<String>,
    #[serde(default)]
    build_systems: Vec<String>,
    #[serde(default)]
    verification_capabilities: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderWorkItem {
    title: String,
    /// Provider 习惯输出 `type` 而非 `kind`,接受别名以兼容真实 claude 输出。
    /// 合法取值见 `parse_work_item_kind`: backend/frontend/integration/e2e/docs/infra/other。
    #[serde(alias = "type")]
    kind: String,
    #[serde(default)]
    sequence_hint: Option<u32>,
    #[serde(default)]
    depends_on: Vec<usize>,
    #[serde(default)]
    exclusive_write_scopes: Vec<String>,
    #[serde(default)]
    forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    context_budget: Option<WorkItemContextBudget>,
    #[serde(default)]
    required_handoff_from: Vec<String>,
    #[serde(default)]
    require_execution_plan_confirm: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderVerificationCommand {
    id: Option<String>,
    label: String,
    command: String,
    #[serde(default)]
    cwd: String,
    purpose: String,
    #[serde(default)]
    required: bool,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    safety: String,
}

fn default_timeout() -> u64 {
    300
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderVerificationManualCheck {
    id: Option<String>,
    label: String,
    instructions: String,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderVerificationPlan {
    #[serde(default)]
    scope: String,
    #[serde(default)]
    commands: Vec<ProviderVerificationCommand>,
    #[serde(default)]
    manual_checks: Vec<ProviderVerificationManualCheck>,
    #[serde(default)]
    required_gates: Vec<String>,
    #[serde(default)]
    risk_notes: Vec<String>,
    #[serde(default)]
    confidence: String,
    #[serde(default)]
    fallback_policy: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ProviderOutput {
    repository_profile: ProviderRepositoryProfile,
    work_items: Vec<ProviderWorkItem>,
    verification_plans: Vec<ProviderVerificationPlan>,
}

fn parse_provider_output(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: String,
    structured: &serde_json::Value,
) -> ApiResult<WorkItemSplitProviderOutput> {
    let parsed: ProviderOutput = serde_json::from_value(structured.clone()).map_err(|error| {
        ApiError::runtime(
            "work_item_split_provider_output_invalid",
            format!("failed to parse provider output: {error}"),
            json!({}),
        )
    })?;

    if parsed.work_items.is_empty() {
        return Err(ApiError::runtime(
            "work_item_split_provider_output_invalid",
            "provider returned no work items",
            json!({}),
        ));
    }

    if parsed.work_items.len() != parsed.verification_plans.len() {
        return Err(ApiError::runtime(
            "work_item_split_provider_output_invalid",
            "verification_plans count must match work_items count",
            json!({}),
        ));
    }

    let count = lifecycle
        .count_work_items(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let work_item_ids: Vec<String> = (0..parsed.work_items.len())
        .map(|index| crate::product::id::next_sequential_id("work_item", count + index))
        .collect();

    let profile_id = crate::product::id::next_sequential_id(
        "repository_profile",
        lifecycle
            .list_repository_profiles(&issue.project_id, &issue.id)
            .map_err(product_store_api_error)?
            .len(),
    );

    let mut verification_plan_ids = Vec::with_capacity(parsed.verification_plans.len());
    let existing_verification_plans = lifecycle
        .list_verification_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    for index in 0..parsed.verification_plans.len() {
        verification_plan_ids.push(crate::product::id::next_sequential_id(
            "verification_plan",
            existing_verification_plans.len() + index,
        ));
    }

    let mut work_items = Vec::with_capacity(parsed.work_items.len());
    for (index, item) in parsed.work_items.iter().enumerate() {
        let id = work_item_ids[index].clone();
        let depends_on: Vec<String> = item
            .depends_on
            .iter()
            .filter_map(|dep_index| work_item_ids.get(*dep_index).cloned())
            .collect();
        work_items.push(LifecycleWorkItemRecord {
            id,
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            story_spec_ids: request.story_spec_ids.clone(),
            design_spec_ids: request.design_spec_ids.clone(),
            title: item.title.clone(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            kind: parse_work_item_kind(&item.kind),
            sequence_hint: item.sequence_hint,
            depends_on,
            exclusive_write_scopes: item.exclusive_write_scopes.clone(),
            forbidden_write_scopes: item.forbidden_write_scopes.clone(),
            context_budget: item.context_budget.clone().unwrap_or_default(),
            required_handoff_from: item.required_handoff_from.clone(),
            verification_plan_ref: Some(verification_plan_ids[index].clone()),
            require_execution_plan_confirm: item.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: String::new(),
            updated_at: String::new(),
        });
    }

    let mut dependency_graph: Vec<IssueWorkItemDependencyEdge> = Vec::new();
    for item in &work_items {
        for dep in &item.depends_on {
            dependency_graph.push(IssueWorkItemDependencyEdge {
                from_work_item_id: dep.clone(),
                to_work_item_id: item.id.clone(),
            });
        }
    }

    let repository_profile = RepositoryProfile {
        id: profile_id.clone(),
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        repository_id: repository.id.clone(),
        provider_run_ref: Some(provider_run_ref.clone()),
        languages: parsed.repository_profile.languages,
        frameworks: parsed.repository_profile.frameworks,
        package_managers: parsed.repository_profile.package_managers,
        test_frameworks: parsed.repository_profile.test_frameworks,
        build_systems: parsed.repository_profile.build_systems,
        verification_capabilities: parsed.repository_profile.verification_capabilities,
        detected_layers: parsed.repository_profile.detected_layers,
        split_recommendation: parsed.repository_profile.split_recommendation,
        confidence: parse_confidence(&parsed.repository_profile.confidence),
        uncertainties: parsed.repository_profile.uncertainties,
        created_at: String::new(),
        updated_at: String::new(),
    };

    let verification_plans: Vec<VerificationPlan> = parsed
        .verification_plans
        .iter()
        .enumerate()
        .map(|(index, plan)| VerificationPlan {
            id: verification_plan_ids[index].clone(),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            work_item_id: work_item_ids[index].clone(),
            repository_profile_ref: Some(profile_id.clone()),
            provider_run_ref: Some(provider_run_ref.clone()),
            scope: parse_verification_scope(&plan.scope),
            commands: plan
                .commands
                .iter()
                .enumerate()
                .map(|(cmd_index, cmd)| VerificationCommand {
                    id: cmd
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("cmd_{:03}", cmd_index + 1)),
                    label: cmd.label.clone(),
                    command: cmd.command.clone(),
                    cwd: cmd.cwd.clone(),
                    purpose: cmd.purpose.clone(),
                    required: cmd.required,
                    timeout_seconds: cmd.timeout_seconds,
                    source: VerificationCommandSource::Provider,
                    safety: parse_safety(&cmd.safety),
                })
                .collect(),
            manual_checks: plan
                .manual_checks
                .iter()
                .enumerate()
                .map(|(check_index, check)| VerificationManualCheck {
                    id: check
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("manual_{:03}", check_index + 1)),
                    label: check.label.clone(),
                    instructions: check.instructions.clone(),
                    required: check.required,
                })
                .collect(),
            required_gates: plan.required_gates.clone(),
            risk_notes: plan.risk_notes.clone(),
            confidence: parse_confidence(&plan.confidence),
            fallback_policy: parse_fallback_policy(&plan.fallback_policy),
            created_at: String::new(),
            updated_at: String::new(),
        })
        .collect();

    let existing_plans = lifecycle
        .list_issue_work_item_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let plan_id =
        crate::product::id::next_sequential_id("issue_work_item_plan", existing_plans.len());

    let plan = IssueWorkItemPlan {
        id: plan_id,
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        source_story_spec_ids: request.story_spec_ids.clone(),
        source_design_spec_ids: request.design_spec_ids.clone(),
        options: IssueWorkItemPlanOptions {
            include_integration_tests: request.include_integration_tests.unwrap_or(false),
            include_e2e_tests: request.include_e2e_tests.unwrap_or(false),
            force_frontend_backend_split: request.force_frontend_backend_split.unwrap_or(false),
            require_execution_plan_confirm: request.require_execution_plan_confirm.unwrap_or(false),
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: work_item_ids.clone(),
        repository_profile_ref: Some(profile_id),
        verification_plan_ids: verification_plan_ids.clone(),
        dependency_graph,
        created_from_provider_run: Some(provider_run_ref),
        validator_findings: Vec::new(),
        review_summary: None,
        created_at: String::new(),
        updated_at: String::new(),
    };

    Ok(WorkItemSplitProviderOutput {
        repository_profile,
        plan,
        work_items,
        verification_plans,
    })
}

fn parse_revision_redo_output(structured: &serde_json::Value) -> ApiResult<ProviderOutput> {
    serde_json::from_value(structured.clone()).map_err(|error| {
        ApiError::runtime(
            "work_item_split_provider_output_invalid",
            format!("failed to parse revision redo output: {error}"),
            json!({}),
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn materialize_revision_output(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: String,
    structured: &serde_json::Value,
    retained: &[LifecycleWorkItemRecord],
    redo_specs: &[RedoSpec],
) -> ApiResult<WorkItemSplitProviderOutput> {
    let redo = parse_revision_redo_output(structured)?;
    if redo.work_items.len() != redo_specs.len()
        || redo.verification_plans.len() != redo_specs.len()
    {
        return Err(ApiError::validation(
            "revision_redo_count_mismatch",
            format!(
                "redo_specs={} but provider returned work_items={} verification_plans={}",
                redo_specs.len(),
                redo.work_items.len(),
                redo.verification_plans.len()
            ),
        ));
    }

    let mut id_mapping = HashMap::new();
    let mut merged_work_items = retained.to_vec();

    let profile_id = crate::product::id::next_sequential_id(
        "repository_profile",
        lifecycle
            .list_repository_profiles(&issue.project_id, &issue.id)
            .map_err(product_store_api_error)?
            .len(),
    );

    let parsed_profile = redo.repository_profile;
    let (mut redo_work_items, redo_verification_plans) = materialize_redo_items(
        lifecycle,
        request,
        issue,
        repository,
        &provider_run_ref,
        redo.work_items,
        redo.verification_plans,
        redo_specs,
        &profile_id,
        &mut id_mapping,
    )?;
    merged_work_items.append(&mut redo_work_items);

    for wi in &mut merged_work_items {
        wi.depends_on = wi
            .depends_on
            .iter()
            .map(|dep| id_mapping.get(dep).cloned().unwrap_or_else(|| dep.clone()))
            .collect();
    }

    let old_graph = build_graph_from_work_items(&merged_work_items);
    let dependency_graph = repatch_dependencies(&old_graph, &id_mapping);

    build_revision_provider_output(
        lifecycle,
        request,
        issue,
        repository,
        provider_run_ref,
        profile_id,
        merged_work_items,
        parsed_profile,
        redo_verification_plans,
        dependency_graph,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_redo_items(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: &str,
    redo_items: Vec<ProviderWorkItem>,
    redo_plans: Vec<ProviderVerificationPlan>,
    redo_specs: &[RedoSpec],
    profile_id: &str,
    id_mapping: &mut HashMap<String, String>,
) -> ApiResult<(Vec<LifecycleWorkItemRecord>, Vec<VerificationPlan>)> {
    let count = lifecycle
        .count_work_items(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let work_item_ids: Vec<String> = (0..redo_items.len())
        .map(|index| crate::product::id::next_sequential_id("work_item", count + index))
        .collect();

    let existing_verification_plans = lifecycle
        .list_verification_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let verification_plan_ids: Vec<String> = (0..redo_plans.len())
        .map(|index| {
            crate::product::id::next_sequential_id(
                "verification_plan",
                existing_verification_plans.len() + index,
            )
        })
        .collect();

    let mut work_items = Vec::with_capacity(redo_items.len());
    for (index, item) in redo_items.iter().enumerate() {
        let id = work_item_ids[index].clone();
        let depends_on: Vec<String> = item
            .depends_on
            .iter()
            .filter_map(|dep_index| {
                work_item_ids.get(*dep_index).cloned().or_else(|| {
                    tracing::warn!(
                        "revision redo work item {} depends_on index {} is out of range or references a retained item; ignoring",
                        id,
                        dep_index
                    );
                    None
                })
            })
            .collect();
        work_items.push(LifecycleWorkItemRecord {
            id: id.clone(),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            story_spec_ids: request.story_spec_ids.clone(),
            design_spec_ids: request.design_spec_ids.clone(),
            title: item.title.clone(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            kind: parse_work_item_kind(&item.kind),
            sequence_hint: item.sequence_hint,
            depends_on,
            exclusive_write_scopes: item.exclusive_write_scopes.clone(),
            forbidden_write_scopes: item.forbidden_write_scopes.clone(),
            context_budget: item.context_budget.clone().unwrap_or_default(),
            required_handoff_from: item.required_handoff_from.clone(),
            verification_plan_ref: Some(verification_plan_ids[index].clone()),
            require_execution_plan_confirm: item.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: String::new(),
            updated_at: String::new(),
        });

        id_mapping.insert(redo_specs[index].old_id.clone(), id);
    }

    let verification_plans: Vec<VerificationPlan> = redo_plans
        .iter()
        .enumerate()
        .map(|(index, plan)| VerificationPlan {
            id: verification_plan_ids[index].clone(),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            work_item_id: work_item_ids[index].clone(),
            repository_profile_ref: Some(profile_id.to_string()),
            provider_run_ref: Some(provider_run_ref.to_string()),
            scope: parse_verification_scope(&plan.scope),
            commands: plan
                .commands
                .iter()
                .enumerate()
                .map(|(cmd_index, cmd)| VerificationCommand {
                    id: cmd
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("cmd_{:03}", cmd_index + 1)),
                    label: cmd.label.clone(),
                    command: cmd.command.clone(),
                    cwd: cmd.cwd.clone(),
                    purpose: cmd.purpose.clone(),
                    required: cmd.required,
                    timeout_seconds: cmd.timeout_seconds,
                    source: VerificationCommandSource::Provider,
                    safety: parse_safety(&cmd.safety),
                })
                .collect(),
            manual_checks: plan
                .manual_checks
                .iter()
                .enumerate()
                .map(|(check_index, check)| VerificationManualCheck {
                    id: check
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("manual_{:03}", check_index + 1)),
                    label: check.label.clone(),
                    instructions: check.instructions.clone(),
                    required: check.required,
                })
                .collect(),
            required_gates: plan.required_gates.clone(),
            risk_notes: plan.risk_notes.clone(),
            confidence: parse_confidence(&plan.confidence),
            fallback_policy: parse_fallback_policy(&plan.fallback_policy),
            created_at: String::new(),
            updated_at: String::new(),
        })
        .collect();

    Ok((work_items, verification_plans))
}

fn build_graph_from_work_items(
    items: &[LifecycleWorkItemRecord],
) -> Vec<IssueWorkItemDependencyEdge> {
    let mut graph = Vec::new();
    for item in items {
        for dep in &item.depends_on {
            graph.push(IssueWorkItemDependencyEdge {
                from_work_item_id: dep.clone(),
                to_work_item_id: item.id.clone(),
            });
        }
    }
    graph
}

#[allow(clippy::too_many_arguments)]
fn build_revision_provider_output(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: String,
    profile_id: String,
    mut work_items: Vec<LifecycleWorkItemRecord>,
    parsed_profile: ProviderRepositoryProfile,
    redo_verification_plans: Vec<VerificationPlan>,
    dependency_graph: Vec<IssueWorkItemDependencyEdge>,
) -> ApiResult<WorkItemSplitProviderOutput> {
    let redo_count = redo_verification_plans.len();
    let retained_count = work_items.len().saturating_sub(redo_count);

    let existing_verification_plans = lifecycle
        .list_verification_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let mut verification_plans = Vec::with_capacity(work_items.len());
    let mut verification_plan_ids = Vec::with_capacity(work_items.len());

    // retained 的 verification_plan id 必须跳过 redo_verification_plans 已占用的
    // redo_count 个 id，避免与 redo 项冲突。
    for (index, wi) in work_items.iter_mut().enumerate() {
        let plan_id = if index < retained_count {
            crate::product::id::next_sequential_id(
                "verification_plan",
                existing_verification_plans.len() + redo_count + index,
            )
        } else {
            redo_verification_plans[index - retained_count].id.clone()
        };
        wi.verification_plan_ref = Some(plan_id.clone());
        verification_plan_ids.push(plan_id.clone());

        if index < retained_count {
            verification_plans.push(VerificationPlan {
                id: plan_id,
                project_id: issue.project_id.clone(),
                issue_id: issue.id.clone(),
                work_item_id: wi.id.clone(),
                repository_profile_ref: Some(profile_id.clone()),
                provider_run_ref: Some(provider_run_ref.clone()),
                scope: VerificationScope::Custom,
                commands: Vec::new(),
                manual_checks: Vec::new(),
                required_gates: Vec::new(),
                risk_notes: Vec::new(),
                confidence: RepositoryProfileConfidence::Medium,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
                created_at: String::new(),
                updated_at: String::new(),
            });
        } else {
            verification_plans.push(redo_verification_plans[index - retained_count].clone());
        }
    }

    let repository_profile = RepositoryProfile {
        id: profile_id.clone(),
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        repository_id: repository.id.clone(),
        provider_run_ref: Some(provider_run_ref.clone()),
        languages: parsed_profile.languages,
        frameworks: parsed_profile.frameworks,
        package_managers: parsed_profile.package_managers,
        test_frameworks: parsed_profile.test_frameworks,
        build_systems: parsed_profile.build_systems,
        verification_capabilities: parsed_profile.verification_capabilities,
        detected_layers: parsed_profile.detected_layers,
        split_recommendation: parsed_profile.split_recommendation,
        confidence: parse_confidence(&parsed_profile.confidence),
        uncertainties: parsed_profile.uncertainties,
        created_at: String::new(),
        updated_at: String::new(),
    };

    let work_item_ids: Vec<String> = work_items.iter().map(|wi| wi.id.clone()).collect();

    let existing_plans = lifecycle
        .list_issue_work_item_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let plan_id =
        crate::product::id::next_sequential_id("issue_work_item_plan", existing_plans.len());

    let plan = IssueWorkItemPlan {
        id: plan_id,
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        source_story_spec_ids: request.story_spec_ids.clone(),
        source_design_spec_ids: request.design_spec_ids.clone(),
        options: IssueWorkItemPlanOptions {
            include_integration_tests: request.include_integration_tests.unwrap_or(false),
            include_e2e_tests: request.include_e2e_tests.unwrap_or(false),
            force_frontend_backend_split: request.force_frontend_backend_split.unwrap_or(false),
            require_execution_plan_confirm: request.require_execution_plan_confirm.unwrap_or(false),
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids,
        repository_profile_ref: Some(profile_id),
        verification_plan_ids,
        dependency_graph,
        created_from_provider_run: Some(provider_run_ref),
        validator_findings: Vec::new(),
        review_summary: None,
        created_at: String::new(),
        updated_at: String::new(),
    };

    Ok(WorkItemSplitProviderOutput {
        repository_profile,
        plan,
        work_items,
        verification_plans,
    })
}

fn parse_work_item_kind(value: &str) -> WorkItemKind {
    match value {
        "backend" => WorkItemKind::Backend,
        "frontend" => WorkItemKind::Frontend,
        "integration" => WorkItemKind::Integration,
        "e2e" => WorkItemKind::E2e,
        "docs" => WorkItemKind::Docs,
        "infra" => WorkItemKind::Infra,
        _ => WorkItemKind::Other,
    }
}

fn parse_confidence(value: &str) -> RepositoryProfileConfidence {
    match value {
        "high" => RepositoryProfileConfidence::High,
        "low" => RepositoryProfileConfidence::Low,
        _ => RepositoryProfileConfidence::Medium,
    }
}

fn parse_verification_scope(value: &str) -> VerificationScope {
    match value {
        "unit" => VerificationScope::Unit,
        "integration" => VerificationScope::Integration,
        "e2e" => VerificationScope::E2e,
        "build" => VerificationScope::Build,
        "lint" => VerificationScope::Lint,
        "manual" => VerificationScope::Manual,
        _ => VerificationScope::Custom,
    }
}

fn parse_safety(value: &str) -> VerificationCommandSafety {
    match value {
        "approved" => VerificationCommandSafety::Approved,
        _ => VerificationCommandSafety::NeedsManualReview,
    }
}

fn parse_fallback_policy(value: &str) -> VerificationFallbackPolicy {
    match value {
        "repair_provider_output" => VerificationFallbackPolicy::RepairProviderOutput,
        _ => VerificationFallbackPolicy::ManualGate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::models::{
        IssuePhase, IssueStatus, WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
        WorkItemGenerationMode,
    };
    use std::path::PathBuf;

    #[test]
    fn build_split_prompt_includes_revision_feedback() {
        let request = GenerateWorkItemsRequest {
            title: "test plan".to_string(),
            story_spec_ids: vec![],
            design_spec_ids: vec![],
            include_integration_tests: None,
            include_e2e_tests: None,
            force_frontend_backend_split: None,
            require_execution_plan_confirm: None,
            author_provider: None,
            reviewer_provider: None,
            review_rounds: None,
            superpowers_enabled: None,
            openspec_enabled: None,
            revision_feedback: Some("- [error] missing write scope\n".to_string()),
        };
        let issue = IssueRecord {
            id: "issue_0001".to_string(),
            project_id: "project_0001".to_string(),
            repo_id: None,
            title: "Test Issue".to_string(),
            description: None,
            change_id: "change_0001".to_string(),
            phase: IssuePhase::Clarification,
            status: IssueStatus::Draft,
            active_binding_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let repository = RepositoryRecord {
            id: "repo_0001".to_string(),
            project_id: "project_0001".to_string(),
            name: "test-repo".to_string(),
            path: PathBuf::from("/tmp/repo"),
            repo_hash: "abc".to_string(),
            runtime_root: PathBuf::from("/tmp/repo"),
            default_policy_preset: "default".to_string(),
            default_provider_mode: "default".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        };

        let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

        assert!(
            prompt.contains("[revision_feedback]"),
            "prompt should contain revision feedback section: {prompt}"
        );
        assert!(
            prompt.contains("missing write scope"),
            "prompt should contain feedback content: {prompt}"
        );
    }

    fn split_prompt_fixture() -> (GenerateWorkItemsRequest, IssueRecord, RepositoryRecord) {
        let request = GenerateWorkItemsRequest {
            title: "test plan".to_string(),
            story_spec_ids: vec![],
            design_spec_ids: vec![],
            include_integration_tests: None,
            include_e2e_tests: None,
            force_frontend_backend_split: None,
            require_execution_plan_confirm: None,
            author_provider: None,
            reviewer_provider: None,
            review_rounds: None,
            superpowers_enabled: None,
            openspec_enabled: None,
            revision_feedback: None,
        };
        let issue = IssueRecord {
            id: "issue_0001".to_string(),
            project_id: "project_0001".to_string(),
            repo_id: None,
            title: "Test Issue".to_string(),
            description: None,
            change_id: "change_0001".to_string(),
            phase: IssuePhase::Clarification,
            status: IssueStatus::Draft,
            active_binding_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let repository = RepositoryRecord {
            id: "repo_0001".to_string(),
            project_id: "project_0001".to_string(),
            name: "test-repo".to_string(),
            path: PathBuf::from("/tmp/repo"),
            repo_hash: "abc".to_string(),
            runtime_root: PathBuf::from("/tmp/repo"),
            default_policy_preset: "default".to_string(),
            default_provider_mode: "default".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        (request, issue, repository)
    }

    #[test]
    fn design_context_capabilities_detects_required_sections() {
        let markdown = r#"
# 技术方案

## 架构概览
系统分层说明。

## Modules
模块拆分说明。

## Tech Stack
Rust + React。

## Test Strategy
cargo test 与 vitest。

## Key Paths
- src/product
- web/src

## Dependencies / Verification
外部依赖和验证约束。
"#;

        let capabilities = extract_design_context_capabilities(markdown);

        assert!(capabilities.has_architecture);
        assert!(capabilities.has_module_breakdown);
        assert!(capabilities.has_tech_stack);
        assert!(capabilities.has_test_strategy);
        assert!(capabilities.has_key_paths);
        assert!(design_context_gaps(&capabilities).is_empty());
    }

    #[test]
    fn legacy_design_spec_gaps_are_injected_without_blocking() {
        let markdown = r#"
# 旧版设计

## Architecture
只有架构描述。

## 模块划分
有模块拆分，但没有测试策略和关键目录。
"#;

        let capabilities = extract_design_context_capabilities(markdown);
        let gaps = design_context_gaps(&capabilities);

        assert!(capabilities.has_architecture);
        assert!(capabilities.has_module_breakdown);
        assert_eq!(
            gaps,
            vec![
                "missing_tech_stack".to_string(),
                "missing_test_strategy".to_string(),
                "missing_key_paths".to_string()
            ]
        );
    }

    #[test]
    fn outline_author_prompt_forbids_full_work_items_and_repository_profile() {
        let (request, issue, repository) = split_prompt_fixture();
        let prompt = build_outline_prompt(
            &request,
            &issue,
            &repository,
            &["story context".to_string()],
            &["design context".to_string()],
            "(empty)",
            &["missing_test_strategy".to_string()],
            &[],
        );

        assert!(prompt.contains("只能输出 WorkItemPlan Outline"));
        assert!(prompt.contains("不得输出完整 Work Item"));
        assert!(prompt.contains("不得输出 VerificationPlan"));
        assert!(prompt.contains("不得输出 repository_profile"));
        assert!(prompt.contains("不得输出 parallel_groups"));
        assert!(prompt.contains("context_blockers"));
        assert!(prompt.contains("missing_test_strategy"));
        assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    }

    #[test]
    fn outline_parser_accepts_valid_sentinel_json() {
        let parsed =
            parse_work_item_plan_outline_output(valid_outline_author_output()).expect("outline");

        assert!(parsed.context_blockers.is_empty());
        let outline = parsed.outline.expect("outline payload");
        assert_eq!(outline.work_item_outlines[0].outline_id, "outline_backend");
        assert_eq!(
            outline.dependency_graph[0].from_outline_id,
            "outline_backend"
        );
    }

    #[test]
    fn outline_parser_rejects_verification_plan_or_work_item_id() {
        let mut output = valid_outline_author_output();
        output["outline"]["work_item_outlines"][0]["verification_plan"] =
            serde_json::json!({"commands": []});

        let error = parse_work_item_plan_outline_output(output).expect_err("forbidden field");
        assert_eq!(error.code, "outline_forbidden_field");

        let mut output = valid_outline_author_output();
        output["outline"]["work_item_outlines"][0]["work_item_id"] =
            serde_json::json!("work_item_0001");

        let error = parse_work_item_plan_outline_output(output).expect_err("forbidden field");
        assert_eq!(error.code, "outline_forbidden_field");
    }

    #[test]
    fn single_item_prompt_contains_accepted_previous_context() {
        let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
            .expect("outline output")
            .outline
            .expect("outline");
        let accepted_backend = sample_draft_record(
            "draft_backend",
            "outline_backend",
            WorkItemDraftCandidate {
                outline_id: "outline_backend".to_string(),
                title: "后端 API".to_string(),
                kind: WorkItemKind::Backend,
                goal: "实现 API".to_string(),
                implementation_context: "定义 GET /api/session/status".to_string(),
                exclusive_write_scopes: vec!["src/product/**".to_string()],
                forbidden_write_scopes: vec!["web/**".to_string()],
                depends_on_outline_ids: vec![],
                required_handoff_from_outline_ids: vec![],
                handoff_summary: "后端输出 SessionStatusDto".to_string(),
                verification_plan: serde_json::json!({"commands": []}),
            },
        );

        let invocation = build_work_item_draft_invocation(
            &outline,
            "outline_frontend",
            WorkItemGenerationMode::Serial,
            &[accepted_backend],
            Some("补充错误态"),
        )
        .expect("draft invocation");

        assert!(invocation.prompt.contains("outline_frontend"));
        assert!(invocation.prompt.contains("serial"));
        assert!(invocation.prompt.contains("SessionStatusDto"));
        assert!(invocation.prompt.contains("直接依赖 draft 完整内容"));
        assert!(invocation.prompt.contains("补充错误态"));
    }

    #[test]
    fn single_item_prompt_forbids_work_item_id_and_outline_changes() {
        let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
            .expect("outline output")
            .outline
            .expect("outline");

        let invocation = build_work_item_draft_invocation(
            &outline,
            "outline_backend",
            WorkItemGenerationMode::Serial,
            &[],
            None,
        )
        .expect("draft invocation");

        assert!(invocation.prompt.contains("不得输出 work_item_id"));
        assert!(invocation.prompt.contains("不得修改 Outline"));
        assert!(
            invocation
                .prompt
                .contains("只能输出一个 WorkItemDraftCandidate")
        );
    }

    #[test]
    fn single_item_parser_rejects_multiple_work_items() {
        let error = parse_work_item_draft_output(serde_json::json!({
            "drafts": [
                valid_work_item_draft_candidate_json("outline_backend"),
                valid_work_item_draft_candidate_json("outline_frontend")
            ]
        }))
        .expect_err("multiple drafts must be rejected");

        assert_eq!(error.code, "work_item_draft_multiple_items");
    }

    #[test]
    fn single_item_parser_rejects_backend_status_fields() {
        let mut output = serde_json::json!({
            "draft": valid_work_item_draft_candidate_json("outline_backend")
        });
        output["draft"]["status"] = serde_json::json!("accepted");

        let error = parse_work_item_draft_output(output).expect_err("status must be rejected");
        assert_eq!(error.code, "work_item_draft_forbidden_field");
    }

    #[test]
    fn build_split_prompt_inlines_schema_and_kind_guidance() {
        // 回归 Bug: prompt 曾引用不存在的 `src/product/work_item_split_output_schema.json`,
        // 而 WORK_ITEM_SPLIT_OUTPUT_SCHEMA 常量未注入 prompt,导致 provider 不知道
        // `kind` 是必填字段,按习惯输出 `type` 触发 `missing field kind`。
        // 修复后 prompt 必须内联 schema 正文并给出 kind 合法取值。
        let (request, issue, repository) = split_prompt_fixture();
        let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

        assert!(
            !prompt.contains("work_item_split_output_schema.json"),
            "prompt must not reference a non-existent schema file path: {prompt}"
        );
        // schema 正文必须内联进 prompt(取 schema 常量里的标志性片段)。
        assert!(
            prompt.contains("\"kind\""),
            "prompt must inline the schema's `kind` property: {prompt}"
        );
        assert!(
            prompt.contains("\"required\""),
            "prompt must inline the schema's `required` clause: {prompt}"
        );
        // kind 合法取值引导(provider 必须知道有哪些枚举值可选)。
        for kind_value in [
            "backend",
            "frontend",
            "integration",
            "e2e",
            "docs",
            "infra",
            "other",
        ] {
            assert!(
                prompt.contains(kind_value),
                "prompt must list kind value `{kind_value}`: {prompt}"
            );
        }
    }

    #[test]
    fn build_split_prompt_allows_readable_stream_before_final_sentinel() {
        let (request, issue, repository) = split_prompt_fixture();
        let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

        assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
        assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
        assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
        assert!(prompt.contains("最后必须输出一个 nonce sentinel JSON block"));
        assert!(prompt.contains("后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT"));
        assert!(prompt.contains("不要输出 Markdown code fence"));
    }

    #[test]
    fn split_prompt_requests_progress_before_long_operations() {
        let (request, issue, repository) = split_prompt_fixture();
        let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

        assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
        assert!(prompt.contains("先输出一行简短可读状态"));
        assert!(prompt.contains("每完成一组探索后输出一句当前发现摘要"));
    }

    #[test]
    fn build_revision_prompt_inlines_schema_and_kind_guidance() {
        let (request, issue, repository) = split_prompt_fixture();
        let redo_specs = vec![RedoSpec {
            old_id: "work_item_0001".to_string(),
            feedback: "拆得太粗".to_string(),
        }];
        let prompt = build_revision_prompt(
            &request,
            &issue,
            &repository,
            &[],
            &redo_specs,
            &[],
            &[],
            "(empty)",
        );

        assert!(
            !prompt.contains("work_item_split_output_schema.json"),
            "revision prompt must not reference a non-existent schema file path: {prompt}"
        );
        assert!(
            prompt.contains("\"kind\""),
            "revision prompt must inline the schema's `kind` property: {prompt}"
        );
        assert!(
            prompt.contains("\"required\""),
            "revision prompt must inline the schema's `required` clause: {prompt}"
        );
        for kind_value in [
            "backend",
            "frontend",
            "integration",
            "e2e",
            "docs",
            "infra",
            "other",
        ] {
            assert!(
                prompt.contains(kind_value),
                "revision prompt must list kind value `{kind_value}`: {prompt}"
            );
        }
    }

    #[test]
    fn build_revision_prompt_allows_readable_stream_before_final_sentinel() {
        let (request, issue, repository) = split_prompt_fixture();
        let redo_specs = vec![RedoSpec {
            old_id: "work_item_0001".to_string(),
            feedback: "拆得太粗".to_string(),
        }];
        let prompt = build_revision_prompt(
            &request,
            &issue,
            &repository,
            &[],
            &redo_specs,
            &[],
            &[],
            "(empty)",
        );

        assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
        assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
        assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
        assert!(prompt.contains("最后必须输出一个 nonce sentinel JSON block"));
        assert!(prompt.contains("后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT"));
        assert!(prompt.contains("不要输出 Markdown code fence"));
    }

    #[test]
    fn revision_prompt_requests_progress_before_long_operations() {
        let (request, issue, repository) = split_prompt_fixture();
        let redo_specs = vec![RedoSpec {
            old_id: "work_item_0001".to_string(),
            feedback: "拆得太粗".to_string(),
        }];
        let prompt = build_revision_prompt(
            &request,
            &issue,
            &repository,
            &[],
            &redo_specs,
            &[],
            &[],
            "(empty)",
        );

        assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
        assert!(prompt.contains("先输出一行简短可读状态"));
        assert!(prompt.contains("每完成一组探索后输出一句当前发现摘要"));
    }

    fn valid_outline_author_output() -> serde_json::Value {
        serde_json::json!({
            "outline": {
                "id": "outline_artifact_1",
                "project_id": "project_0001",
                "issue_id": "issue_0001",
                "source_story_spec_ids": ["story_spec_0001"],
                "source_design_spec_ids": ["design_spec_0001"],
                "strategy_summary": "先后端后前端",
                "work_item_outlines": [
                    {
                        "outline_id": "outline_backend",
                        "title": "后端 API",
                        "kind": "backend",
                        "goal": "实现 API",
                        "scope": ["src/product"],
                        "non_goals": [],
                        "source_story_spec_ids": ["story_spec_0001"],
                        "source_design_spec_ids": ["design_spec_0001"],
                        "exclusive_write_scopes": ["src/product/**"],
                        "forbidden_write_scopes": ["web/**"],
                        "depends_on": [],
                        "verification_intent": ["cargo test --locked --lib api"],
                        "handoff_notes": "提供 API contract"
                    },
                    {
                        "outline_id": "outline_frontend",
                        "title": "前端 UI",
                        "kind": "frontend",
                        "goal": "接入 API",
                        "scope": ["web/src"],
                        "non_goals": [],
                        "source_story_spec_ids": ["story_spec_0001"],
                        "source_design_spec_ids": ["design_spec_0001"],
                        "exclusive_write_scopes": ["web/src/**"],
                        "forbidden_write_scopes": ["src/product/**"],
                        "depends_on": ["outline_backend"],
                        "verification_intent": ["pnpm -C web test"],
                        "handoff_notes": "消费 API contract"
                    }
                ],
                "dependency_graph": [
                    {
                        "from_outline_id": "outline_backend",
                        "to_outline_id": "outline_frontend"
                    }
                ],
                "risks": [],
                "handoff_strategy": "后端输出 contract 给前端",
                "status": "draft"
            },
            "context_blockers": []
        })
    }

    fn valid_work_item_draft_candidate_json(outline_id: &str) -> serde_json::Value {
        serde_json::json!({
            "outline_id": outline_id,
            "title": "后端 API",
            "kind": "backend",
            "goal": "实现 API",
            "implementation_context": "实现 API handler 与 product service。",
            "exclusive_write_scopes": ["src/product/**"],
            "forbidden_write_scopes": ["web/**"],
            "depends_on_outline_ids": [],
            "required_handoff_from_outline_ids": [],
            "handoff_summary": "输出 SessionStatusDto",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_backend",
                        "label": "cargo test",
                        "command": "cargo test --locked --lib session",
                        "cwd": "",
                        "purpose": "验证后端 API",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "local"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_backend"]
            }
        })
    }

    fn sample_draft_record(
        draft_id: &str,
        outline_id: &str,
        candidate: WorkItemDraftCandidate,
    ) -> WorkItemDraftRecord {
        WorkItemDraftRecord {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "plan_0001".to_string(),
            draft_id: draft_id.to_string(),
            outline_id: outline_id.to_string(),
            generation_round_id: "round_001".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "artifact://outline/1".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            candidate,
            status: WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "node_draft_run".to_string(),
            accepted_at: Some("2026-06-22T10:00:00Z".to_string()),
            superseded_at: None,
            created_at: "2026-06-22T10:00:00Z".to_string(),
            updated_at: "2026-06-22T10:00:00Z".to_string(),
        }
    }
}
