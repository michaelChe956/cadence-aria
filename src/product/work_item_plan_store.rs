use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    OutlineContextBlockerResolution, OutlineContextIndex, WorkItemBatchRecord, WorkItemDraftRecord,
    WorkItemDraftStatus, WorkItemDraftSupersedeReason, WorkItemGenerationMode,
    WorkItemPlanCompileTransaction, WorkItemPlanDraftActiveIndex, WorkItemPlanOutline,
};

const MAX_CONTEXT_RESOLUTIONS: usize = 20;
const MAX_CONTEXT_ESTIMATED_TOKENS: u32 = 8_000;
const SUMMARY_ESTIMATED_TOKENS: u32 = 512;

#[derive(Debug, Clone)]
pub struct WorkItemPlanStore {
    app_paths: ProductAppPaths,
}

impl WorkItemPlanStore {
    pub fn new(app_paths: ProductAppPaths) -> Self {
        Self { app_paths }
    }

    pub fn put_draft_record(&self, record: &WorkItemDraftRecord) -> Result<(), ProductStoreError> {
        validate_draft_record(record)?;
        write_json(&self.draft_record_path(record), record)
    }

    pub fn get_draft_record(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        generation_round_id: &str,
        draft_id: &str,
    ) -> Result<WorkItemDraftRecord, ProductStoreError> {
        validate_work_item_plan_path_ids(project_id, issue_id, plan_id)?;
        validate_relative_id(generation_round_id)?;
        validate_relative_id(draft_id)?;
        read_required_json(
            &self.draft_record_path_for(
                project_id,
                issue_id,
                plan_id,
                generation_round_id,
                draft_id,
            ),
            "work_item_plan_draft",
            draft_id,
        )
    }

    pub fn list_draft_records(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Vec<WorkItemDraftRecord>, ProductStoreError> {
        validate_work_item_plan_path_ids(project_id, issue_id, plan_id)?;
        let root = self.draft_plan_root(project_id, issue_id, plan_id);
        let mut records = Vec::new();

        for round_dir in child_directories(&root)? {
            for entry in json_file_paths(&round_dir)? {
                records.push(read_json(&entry)?);
            }
        }

        Ok(records)
    }

    pub fn load_active_index(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<WorkItemPlanDraftActiveIndex>, ProductStoreError> {
        validate_work_item_plan_path_ids(project_id, issue_id, plan_id)?;
        read_optional_json(&self.active_index_path_for(project_id, issue_id, plan_id))
    }

    pub fn save_active_index(
        &self,
        index: &WorkItemPlanDraftActiveIndex,
    ) -> Result<(), ProductStoreError> {
        validate_active_index(index)?;
        write_json(&self.active_index_path(index), index)
    }

    pub fn put_compile_transaction(
        &self,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), ProductStoreError> {
        validate_compile_transaction(tx)?;
        write_json(&self.compile_transaction_path(tx), tx)
    }

    pub fn get_compile_transaction(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        compile_id: &str,
    ) -> Result<WorkItemPlanCompileTransaction, ProductStoreError> {
        validate_work_item_plan_path_ids(project_id, issue_id, plan_id)?;
        validate_relative_id(compile_id)?;
        read_required_json(
            &self.compile_transaction_path_for(project_id, issue_id, plan_id, compile_id),
            "work_item_plan_compile",
            compile_id,
        )
    }

    pub fn list_compile_transactions(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Vec<WorkItemPlanCompileTransaction>, ProductStoreError> {
        validate_work_item_plan_path_ids(project_id, issue_id, plan_id)?;
        let root = self
            .app_paths
            .issue_root(project_id, issue_id)
            .join("work_item_plan_compiles")
            .join(plan_id);
        json_file_paths(&root)?
            .into_iter()
            .map(|path| read_json(&path))
            .collect()
    }

    pub fn load_outline_context_index(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<OutlineContextIndex>, ProductStoreError> {
        validate_work_item_plan_path_ids(project_id, issue_id, plan_id)?;
        read_optional_json(&self.outline_context_index_path_for(project_id, issue_id, plan_id))
    }

    pub fn save_outline_context_index(
        &self,
        index: &OutlineContextIndex,
    ) -> Result<(), ProductStoreError> {
        validate_context_index(index)?;
        let mut compacted = index.clone();
        compact_outline_context_index(&mut compacted);
        write_json(&self.outline_context_index_path(index), &compacted)
    }

    fn draft_record_path(&self, record: &WorkItemDraftRecord) -> PathBuf {
        self.draft_record_path_for(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            &record.generation_round_id,
            &record.draft_id,
        )
    }

    fn draft_record_path_for(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        generation_round_id: &str,
        draft_id: &str,
    ) -> PathBuf {
        self.draft_plan_root(project_id, issue_id, plan_id)
            .join(generation_round_id)
            .join(format!("{draft_id}.json"))
    }

    fn draft_plan_root(&self, project_id: &str, issue_id: &str, plan_id: &str) -> PathBuf {
        self.issue_root(project_id, issue_id)
            .join("work_item_plan_drafts")
            .join(plan_id)
    }

    fn active_index_path(&self, index: &WorkItemPlanDraftActiveIndex) -> PathBuf {
        self.active_index_path_for(&index.project_id, &index.issue_id, &index.plan_id)
    }

    fn active_index_path_for(&self, project_id: &str, issue_id: &str, plan_id: &str) -> PathBuf {
        self.draft_plan_root(project_id, issue_id, plan_id)
            .join("active_index.json")
    }

    fn compile_transaction_path(&self, tx: &WorkItemPlanCompileTransaction) -> PathBuf {
        self.compile_transaction_path_for(&tx.project_id, &tx.issue_id, &tx.plan_id, &tx.compile_id)
    }

    fn compile_transaction_path_for(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        compile_id: &str,
    ) -> PathBuf {
        self.issue_root(project_id, issue_id)
            .join("work_item_plan_compiles")
            .join(plan_id)
            .join(format!("{compile_id}.json"))
    }

    fn outline_context_index_path(&self, index: &OutlineContextIndex) -> PathBuf {
        self.outline_context_index_path_for(&index.project_id, &index.issue_id, &index.plan_id)
    }

    fn outline_context_index_path_for(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> PathBuf {
        self.issue_root(project_id, issue_id)
            .join("work_item_plan_outlines")
            .join(plan_id)
            .join("outline_context_index.json")
    }

    fn issue_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.app_paths.issue_root(project_id, issue_id)
    }
}

pub fn next_generation_round_id(index: &WorkItemPlanDraftActiveIndex) -> String {
    next_sequential_prefixed_id(&index.current_generation_round_id, "round_")
}

pub fn next_draft_id(index: &WorkItemPlanDraftActiveIndex) -> String {
    let mut max_suffix = 0;
    for draft_id in index.draft_statuses.keys() {
        max_suffix = max_suffix.max(numeric_suffix(draft_id, "draft_").unwrap_or(0));
    }
    for draft_id in index.outline_to_current_draft_id.values() {
        max_suffix = max_suffix.max(numeric_suffix(draft_id, "draft_").unwrap_or(0));
    }
    for batch in &index.batches {
        for draft_id in batch
            .item_draft_ids
            .iter()
            .chain(batch.validation_failed_ids.iter())
        {
            max_suffix = max_suffix.max(numeric_suffix(draft_id, "draft_").unwrap_or(0));
        }
    }
    format!("draft_{:03}", max_suffix + 1)
}

pub fn next_batch_id(index: &WorkItemPlanDraftActiveIndex, now: &str) -> String {
    let date = compact_date(now);
    let prefix = format!("batch_{date}_");
    let mut max_suffix = 0;
    for batch in index.batches.iter().filter(|batch| {
        batch.generation_round_id == index.current_generation_round_id
            && batch.batch_id.starts_with(&prefix)
    }) {
        max_suffix = max_suffix.max(numeric_suffix(&batch.batch_id, &prefix).unwrap_or(0));
    }
    format!("{prefix}{:03}", max_suffix + 1)
}

pub fn mark_draft_active(
    index: &mut WorkItemPlanDraftActiveIndex,
    outline_id: &str,
    draft_id: &str,
    status: WorkItemDraftStatus,
) {
    if let Some(previous_draft_id) = index
        .outline_to_current_draft_id
        .insert(outline_id.to_string(), draft_id.to_string())
        && previous_draft_id != draft_id
    {
        index
            .draft_statuses
            .insert(previous_draft_id, WorkItemDraftStatus::Superseded);
    }
    index.draft_statuses.insert(draft_id.to_string(), status);
}

pub fn mark_downstream_superseded(
    index: &mut WorkItemPlanDraftActiveIndex,
    outline_ids: &[String],
    _reason: WorkItemDraftSupersedeReason,
) {
    for outline_id in outline_ids {
        if let Some(draft_id) = index.outline_to_current_draft_id.remove(outline_id) {
            index
                .draft_statuses
                .insert(draft_id, WorkItemDraftStatus::Superseded);
        }
    }
}

pub fn outline_rewrite_invalidation_plan(
    outline: &WorkItemPlanOutline,
    target_outline_id: &str,
) -> Result<Vec<(String, WorkItemDraftSupersedeReason)>, String> {
    let known_outline_ids: HashSet<&str> = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.as_str())
        .collect();
    if !known_outline_ids.contains(target_outline_id) {
        return Err(format!("target outline `{target_outline_id}` not found"));
    }

    for edge in &outline.dependency_graph {
        if !known_outline_ids.contains(edge.from_outline_id.as_str()) {
            return Err(format!(
                "dependency edge references missing from_outline_id `{}`",
                edge.from_outline_id
            ));
        }
        if !known_outline_ids.contains(edge.to_outline_id.as_str()) {
            return Err(format!(
                "dependency edge references missing to_outline_id `{}`",
                edge.to_outline_id
            ));
        }
    }

    let mut planned = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([target_outline_id.to_string()]);
    visited.insert(target_outline_id.to_string());
    planned.push((
        target_outline_id.to_string(),
        WorkItemDraftSupersedeReason::DirectRewrite,
    ));

    while let Some(current) = queue.pop_front() {
        for edge in outline
            .dependency_graph
            .iter()
            .filter(|edge| edge.from_outline_id == current)
        {
            if visited.insert(edge.to_outline_id.clone()) {
                planned.push((
                    edge.to_outline_id.clone(),
                    WorkItemDraftSupersedeReason::AncestorRewritten,
                ));
                queue.push_back(edge.to_outline_id.clone());
            }
        }
    }

    Ok(planned)
}

pub fn mark_draft_record_superseded(
    record: &mut WorkItemDraftRecord,
    superseded_by_draft_id: Option<String>,
    reason: WorkItemDraftSupersedeReason,
    now: &str,
) {
    record.status = WorkItemDraftStatus::Superseded;
    record.active = false;
    record.superseded_by_draft_id = superseded_by_draft_id;
    record.supersede_reason = Some(reason);
    record.superseded_at = Some(now.to_string());
    record.updated_at = now.to_string();
}

pub fn copy_draft_for_current_round(
    index: &WorkItemPlanDraftActiveIndex,
    source: &WorkItemDraftRecord,
    generated_from_node_id: &str,
    now: &str,
) -> WorkItemDraftRecord {
    let mut copied = source.clone();
    copied.draft_id = next_draft_id(index);
    copied.generation_round_id = index.current_generation_round_id.clone();
    copied.batch_id = None;
    copied.generation_mode = WorkItemGenerationMode::Serial;
    copied.attempt_index = source.attempt_index + 1;
    copied.status = WorkItemDraftStatus::Draft;
    copied.active = true;
    copied.superseded_by_draft_id = None;
    copied.supersede_reason = None;
    copied.copied_from_draft_id = Some(source.draft_id.clone());
    copied.review_node_id = None;
    copied.review_verdict_ref = None;
    copied.generated_from_node_id = generated_from_node_id.to_string();
    copied.accepted_at = None;
    copied.superseded_at = None;
    copied.created_at = now.to_string();
    copied.updated_at = now.to_string();
    copied
}

pub fn compact_outline_context_index(index: &mut OutlineContextIndex) {
    while index.blocker_resolutions.len() > MAX_CONTEXT_RESOLUTIONS {
        let merge_count = index.blocker_resolutions.len() - MAX_CONTEXT_RESOLUTIONS + 1;
        merge_earliest_resolutions(index, merge_count);
    }

    while total_estimated_tokens(index) > MAX_CONTEXT_ESTIMATED_TOKENS
        && index.blocker_resolutions.len() > 1
    {
        merge_earliest_resolutions(index, 2);
    }
}

fn validate_work_item_plan_path_ids(
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)?;
    validate_relative_id(plan_id)?;
    Ok(())
}

fn validate_draft_record(record: &WorkItemDraftRecord) -> Result<(), ProductStoreError> {
    validate_work_item_plan_path_ids(&record.project_id, &record.issue_id, &record.plan_id)?;
    validate_relative_id(&record.draft_id)?;
    validate_relative_id(&record.outline_id)?;
    validate_relative_id(&record.generation_round_id)?;
    validate_relative_id(&record.generated_from_node_id)?;
    validate_optional_id(record.batch_id.as_deref())?;
    validate_optional_id(record.superseded_by_draft_id.as_deref())?;
    validate_optional_id(record.copied_from_draft_id.as_deref())?;
    validate_optional_id(record.review_node_id.as_deref())?;
    validate_relative_id(&record.candidate.outline_id)?;
    for outline_id in &record.candidate.depends_on_outline_ids {
        validate_relative_id(outline_id)?;
    }
    for outline_id in &record.candidate.required_handoff_from_outline_ids {
        validate_relative_id(outline_id)?;
    }
    validate_draft_batch_semantics(record)?;
    Ok(())
}

fn validate_draft_batch_semantics(record: &WorkItemDraftRecord) -> Result<(), ProductStoreError> {
    match (&record.generation_mode, record.batch_id.as_deref()) {
        (WorkItemGenerationMode::Serial, None) | (WorkItemGenerationMode::Batch, Some(_)) => Ok(()),
        (WorkItemGenerationMode::Serial, Some(_)) => Err(ProductStoreError::Json(
            "serial work item draft must not have batch_id".to_string(),
        )),
        (WorkItemGenerationMode::Batch, None) => Err(ProductStoreError::Json(
            "batch work item draft must have batch_id".to_string(),
        )),
    }
}

fn validate_active_index(index: &WorkItemPlanDraftActiveIndex) -> Result<(), ProductStoreError> {
    validate_work_item_plan_path_ids(&index.project_id, &index.issue_id, &index.plan_id)?;
    validate_relative_id(&index.current_generation_round_id)?;
    validate_outline_state(&index.outline_state)?;
    validate_optional_id(index.active_outline_id.as_deref())?;
    for (outline_id, draft_id) in &index.outline_to_current_draft_id {
        validate_relative_id(outline_id)?;
        validate_relative_id(draft_id)?;
    }
    for draft_id in index.draft_statuses.keys() {
        validate_relative_id(draft_id)?;
    }
    for batch in &index.batches {
        validate_batch_record(batch)?;
    }
    Ok(())
}

fn validate_outline_state(value: &str) -> Result<(), ProductStoreError> {
    match value {
        "confirmed" | "revising" => Ok(()),
        _ => Err(ProductStoreError::Json(format!(
            "invalid outline_state: {value}"
        ))),
    }
}

fn validate_batch_record(batch: &WorkItemBatchRecord) -> Result<(), ProductStoreError> {
    validate_relative_id(&batch.batch_id)?;
    validate_relative_id(&batch.generation_round_id)?;
    for draft_id in &batch.item_draft_ids {
        validate_relative_id(draft_id)?;
    }
    for draft_id in &batch.validation_failed_ids {
        validate_relative_id(draft_id)?;
    }
    Ok(())
}

fn validate_compile_transaction(
    tx: &WorkItemPlanCompileTransaction,
) -> Result<(), ProductStoreError> {
    validate_work_item_plan_path_ids(&tx.project_id, &tx.issue_id, &tx.plan_id)?;
    validate_relative_id(&tx.compile_id)?;
    validate_relative_id(&tx.generation_round_id)?;
    for draft_id in &tx.active_draft_ids {
        validate_relative_id(draft_id)?;
    }
    for (outline_id, work_item_id) in &tx.outline_to_work_item_id {
        validate_relative_id(outline_id)?;
        validate_relative_id(work_item_id)?;
    }
    for (outline_id, verification_plan_id) in &tx.outline_to_verification_plan_id {
        validate_relative_id(outline_id)?;
        validate_relative_id(verification_plan_id)?;
    }
    for work_item_id in &tx.created_work_item_ids {
        validate_relative_id(work_item_id)?;
    }
    for verification_plan_id in &tx.created_verification_plan_ids {
        validate_relative_id(verification_plan_id)?;
    }
    for session_id in &tx.child_session_ids {
        validate_relative_id(session_id)?;
    }
    Ok(())
}

fn validate_context_index(index: &OutlineContextIndex) -> Result<(), ProductStoreError> {
    validate_work_item_plan_path_ids(&index.project_id, &index.issue_id, &index.plan_id)?;
    validate_relative_id(&index.generation_round_id)?;
    Ok(())
}

fn validate_optional_id(value: Option<&str>) -> Result<(), ProductStoreError> {
    if let Some(value) = value {
        validate_relative_id(value)?;
    }
    Ok(())
}

fn read_required_json<T: DeserializeOwned>(
    path: &Path,
    kind: &'static str,
    id: &str,
) -> Result<T, ProductStoreError> {
    if !path_exists(path)? {
        return Err(ProductStoreError::NotFound {
            kind,
            id: id.to_string(),
        });
    }
    read_json(path)
}

fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(None);
    }
    read_json(path).map(Some)
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry type: {error}", entry_path.display()))
            })?
            .is_dir()
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry type: {error}", entry_path.display()))
            })?
            .is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn next_sequential_prefixed_id(value: &str, prefix: &str) -> String {
    let width = value
        .strip_prefix(prefix)
        .map(|suffix| suffix.len())
        .filter(|width| *width > 0)
        .unwrap_or(3);
    let next = numeric_suffix(value, prefix).unwrap_or(0) + 1;
    format!("{prefix}{next:0width$}")
}

fn numeric_suffix(value: &str, prefix: &str) -> Option<u32> {
    value.strip_prefix(prefix)?.parse().ok()
}

fn compact_date(value: &str) -> String {
    let date: String = value
        .chars()
        .filter(|character| character.is_ascii_digit())
        .take(8)
        .collect();
    if date.len() == 8 {
        date
    } else {
        "00000000".to_string()
    }
}

fn total_estimated_tokens(index: &OutlineContextIndex) -> u32 {
    index
        .blocker_resolutions
        .iter()
        .fold(0_u32, |total, resolution| {
            total.saturating_add(resolution.estimated_tokens)
        })
}

fn merge_earliest_resolutions(index: &mut OutlineContextIndex, merge_count: usize) {
    if merge_count <= 1 || index.blocker_resolutions.len() < merge_count {
        return;
    }

    let merged: Vec<_> = index.blocker_resolutions.drain(0..merge_count).collect();
    let summary = summarize_resolutions(&merged);
    index.blocker_resolutions.insert(0, summary);
}

fn summarize_resolutions(
    resolutions: &[OutlineContextBlockerResolution],
) -> OutlineContextBlockerResolution {
    let first = resolutions
        .first()
        .expect("summarize_resolutions requires non-empty input");
    let last = resolutions
        .last()
        .expect("summarize_resolutions requires non-empty input");
    let merged_count = resolutions
        .iter()
        .map(|resolution| resolution.merged_count.unwrap_or(1))
        .sum::<u32>();
    let merged_tokens = resolutions.iter().fold(0_u32, |total, resolution| {
        total.saturating_add(resolution.estimated_tokens)
    });

    OutlineContextBlockerResolution {
        blocker_node_id: format!("summary_{}_{}", first.blocker_node_id, last.blocker_node_id),
        resolution_node_id: format!(
            "summary_{}_{}",
            first.resolution_node_id, last.resolution_node_id
        ),
        resolution_artifact_ref: format!(
            "context_blocker_resolution_summary://{}/{}",
            first.blocker_node_id, last.resolution_node_id
        ),
        estimated_tokens: merged_tokens.min(SUMMARY_ESTIMATED_TOKENS),
        created_at: first.created_at.clone(),
        summary: Some(format!(
            "Summarized {merged_count} earlier outline context blocker resolutions."
        )),
        merged_count: Some(merged_count),
    }
}
