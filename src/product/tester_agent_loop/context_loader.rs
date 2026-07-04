use serde::Serialize;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::WorkspaceType;

use super::tools::LoadTestContextInput;

const MAX_LOAD_TEST_CONTEXT_SNIPPET_CHARS: usize = 2_000;
const MAX_LOAD_TEST_CONTEXT_TOTAL_CHARS: usize = 8_000;

#[derive(Debug, Clone)]
pub struct TestContextLoader {
    paths: ProductAppPaths,
    attempt: CodingExecutionAttempt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadedTestContext {
    pub snippets: Vec<LoadedTestContextSnippet>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadedTestContextSnippet {
    pub artifact_ref: String,
    pub selector: String,
    pub text: String,
}

impl TestContextLoader {
    pub fn new(paths: ProductAppPaths, attempt: CodingExecutionAttempt) -> Self {
        Self { paths, attempt }
    }

    pub fn load(&self, input: &LoadTestContextInput) -> Result<LoadedTestContext, String> {
        let lifecycle = LifecycleStore::new(self.paths.clone());
        let mut warnings = Vec::new();
        let artifact_refs = if input.artifact_refs.is_empty() {
            default_artifact_refs(&lifecycle, &self.attempt, &mut warnings)?
        } else {
            input.artifact_refs.clone()
        };
        let mut snippets = Vec::new();
        let mut total_chars = 0usize;

        for artifact_ref in artifact_refs {
            let Some(markdown) = load_markdown_for_ref(&lifecycle, &self.attempt, &artifact_ref)?
            else {
                warnings.push(format!(
                    "load_test_context_artifact_not_found:{artifact_ref}"
                ));
                continue;
            };
            for selector in &input.selectors {
                if total_chars >= MAX_LOAD_TEST_CONTEXT_TOTAL_CHARS {
                    warnings.push("load_test_context_total_truncated".to_string());
                    break;
                }
                let Some(text) = extract_selector_snippet(&markdown, selector) else {
                    warnings.push(format!(
                        "load_test_context_selector_not_found:{artifact_ref}:{selector}"
                    ));
                    continue;
                };
                let remaining = MAX_LOAD_TEST_CONTEXT_TOTAL_CHARS - total_chars;
                let text = take_chars(&text, remaining.min(MAX_LOAD_TEST_CONTEXT_SNIPPET_CHARS));
                total_chars += text.chars().count();
                snippets.push(LoadedTestContextSnippet {
                    artifact_ref: artifact_ref.clone(),
                    selector: selector.clone(),
                    text,
                });
            }
        }

        Ok(LoadedTestContext { snippets, warnings })
    }
}

fn default_artifact_refs(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    let work_items = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| format!("load_test_context_list_work_items_failed:{error}"))?;
    let current_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let Some(work_item) = work_items
        .iter()
        .find(|work_item| work_item.id == current_work_item_id)
    else {
        warnings.push(format!(
            "load_test_context_work_item_not_found:{current_work_item_id}"
        ));
        return Ok(Vec::new());
    };
    Ok(work_item
        .story_spec_ids
        .iter()
        .chain(work_item.design_spec_ids.iter())
        .cloned()
        .collect())
}

fn load_markdown_for_ref(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    artifact_ref: &str,
) -> Result<Option<String>, String> {
    let (artifact_id, version_ref) = artifact_ref
        .split_once('@')
        .map(|(artifact_id, version_ref)| (artifact_id, Some(version_ref)))
        .unwrap_or((artifact_ref, None));
    if lifecycle
        .list_story_specs(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| format!("load_test_context_list_story_specs_failed:{error}"))?
        .iter()
        .any(|story| story.id == artifact_id)
    {
        return load_spec_markdown(lifecycle, attempt, artifact_id, version_ref);
    }
    if lifecycle
        .list_design_specs(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| format!("load_test_context_list_design_specs_failed:{error}"))?
        .iter()
        .any(|design| design.id == artifact_id)
    {
        return load_spec_markdown(lifecycle, attempt, artifact_id, version_ref);
    }
    load_work_item_markdown(lifecycle, attempt, artifact_id, version_ref)
}

fn load_spec_markdown(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    artifact_id: &str,
    version_ref: Option<&str>,
) -> Result<Option<String>, String> {
    let version = lifecycle
        .list_versions(&attempt.project_id, &attempt.issue_id, artifact_id)
        .map_err(|error| format!("load_test_context_list_versions_failed:{error}"))?
        .into_iter()
        .filter(|version| version_ref_matches(version_ref, &version.id, version.version))
        .max_by_key(|version| version.version);
    Ok(version.map(|version| version.markdown))
}

fn load_work_item_markdown(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    artifact_id: &str,
    version_ref: Option<&str>,
) -> Result<Option<String>, String> {
    let work_items = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| format!("load_test_context_list_work_items_failed:{error}"))?;
    if !work_items
        .iter()
        .any(|work_item| work_item.id == artifact_id)
    {
        return Ok(None);
    }
    let sessions = lifecycle
        .list_workspace_sessions(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| format!("load_test_context_list_sessions_failed:{error}"))?;
    let Some(session) = sessions
        .iter()
        .filter(|session| {
            session.entity_id == artifact_id && session.workspace_type == WorkspaceType::WorkItem
        })
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.created_at.cmp(&right.created_at))
        })
    else {
        return Ok(None);
    };
    let version = lifecycle
        .list_artifact_versions(&session.id)
        .map_err(|error| format!("load_test_context_list_artifact_versions_failed:{error}"))?
        .into_iter()
        .filter(|version| {
            version_ref_matches(
                version_ref,
                &format!("artifact_version_{:04}", version.version),
                version.version,
            )
        })
        .max_by_key(|version| version.version);
    Ok(version.map(|version| version.to_markdown_string()))
}

fn version_ref_matches(version_ref: Option<&str>, version_id: &str, version: u32) -> bool {
    version_ref.is_none_or(|value| {
        value == version_id
            || value == format!("version_{version:04}")
            || value == version.to_string()
    })
}

fn extract_selector_snippet(markdown: &str, selector: &str) -> Option<String> {
    let match_start = markdown.find(selector).or_else(|| {
        (!selector.starts_with('[')).then(|| markdown.find(&format!("[{selector}]")))?
    })?;
    let match_end = match_start + selector.len();
    let start = markdown[..match_start]
        .rfind("\n\n")
        .map(|index| index + 2)
        .unwrap_or(0);
    let end = markdown[match_end..]
        .find("\n\n")
        .map(|index| match_end + index)
        .unwrap_or(markdown.len());
    Some(take_chars(
        markdown[start..end].trim(),
        MAX_LOAD_TEST_CONTEXT_SNIPPET_CHARS,
    ))
}

fn take_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let taken = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{taken}\n...[truncated]")
    } else {
        taken
    }
}
