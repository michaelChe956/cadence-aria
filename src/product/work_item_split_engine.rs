use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueRecord, IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleWorkItemRecord, ProviderName, RepositoryProfile,
    RepositoryProfileConfidence, RepositoryRecord, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationManualCheck,
    VerificationPlan, VerificationScope, WorkItemContextBudget, WorkItemExecutionPlanStatus,
    WorkItemKind, WorkItemPlanStatus, WorkItemStatus,
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

#[derive(Debug, Clone)]
pub struct WorkItemSplitProviderOutput {
    pub repository_profile: RepositoryProfile,
    pub plan: IssueWorkItemPlan,
    pub work_items: Vec<LifecycleWorkItemRecord>,
    pub verification_plans: Vec<VerificationPlan>,
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

impl WorkItemSplitEngine {
    pub fn new(provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>) -> Self {
        Self { provider_adapter }
    }

    pub async fn generate(
        &self,
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
    ) -> ApiResult<WorkItemSplitProviderOutput> {
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

        let provider_output = self
            .invoke_provider(&prompt, repository, author_provider, lifecycle, issue)
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

        let provider_output = self
            .invoke_provider(&prompt, repository, author_provider, lifecycle, issue)
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
            &provider_output.run_ref,
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
            provider_output.run_ref,
            profile_id,
            merged_work_items,
            parsed_profile,
            redo_verification_plans,
            dependency_graph,
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

fn design_kind_text(kind: &crate::product::models::DesignKind) -> &'static str {
    match kind {
        crate::product::models::DesignKind::Frontend => "frontend",
        crate::product::models::DesignKind::Backend => "backend",
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
                "Design Spec: {} ({}) kind={}\n{}",
                spec.title,
                spec.id,
                design_kind_text(&spec.design_kind),
                markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
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

fn build_split_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
) -> String {
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
         严格按 src/product/work_item_split_output_schema.json 的 JSON schema 输出，顶层对象包裹在 <ARIA_STRUCTURED_OUTPUT>...</ARIA_STRUCTURED_OUTPUT> 标签内。\n\
         work_items 数组顺序即执行顺序；depends_on 使用同数组中的 0-based 索引。verification_plans 数组与 work_items 一一对应。",
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
    )
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
         严格按 src/product/work_item_split_output_schema.json 的 JSON schema 输出 redo-only 结果，顶层对象包裹在 <ARIA_STRUCTURED_OUTPUT>...</ARIA_STRUCTURED_OUTPUT> 标签内。\n\
         work_items 数组必须且仅包含重做项，顺序对应 redo_work_items 列表；verification_plans 与 work_items 一一对应；depends_on 使用 0-based 索引。",
        title = issue.title,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        retained_section = retained_section,
        redo_section = redo_section,
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
            .filter_map(|dep_index| work_item_ids.get(*dep_index).cloned())
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
    use crate::product::models::{IssuePhase, IssueStatus};
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
}
