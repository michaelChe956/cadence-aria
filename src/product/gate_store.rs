use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{GateRecord, GateStatus, GateType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGateInput {
    pub project_id: String,
    pub issue_id: String,
    pub binding_id: String,
    pub node_id: String,
    pub gate_type: GateType,
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GateStore {
    paths: ProductAppPaths,
}

impl GateStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn list(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<GateRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self.gates_root(project_id, issue_id);
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
            })?;
            let entry_path = entry.path();
            if entry_path.extension().and_then(|value| value.to_str()) == Some("json") {
                entries.push(entry_path);
            }
        }
        entries.sort();

        let mut gates = Vec::with_capacity(entries.len());
        for entry in entries {
            gates.push(read_json(&entry)?);
        }
        Ok(gates)
    }

    pub fn create(&self, input: CreateGateInput) -> Result<GateRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.binding_id)?;
        validate_relative_id(&input.node_id)?;

        let gates = self.list(&input.project_id, &input.issue_id)?;
        if let Some(existing) = gates.iter().find(|gate| {
            gate.binding_id == input.binding_id
                && gate.node_id == input.node_id
                && gate.status == GateStatus::Open
        }) {
            return Ok(existing.clone());
        }

        let id = next_available_gate_id(&gates);
        let now = Utc::now().to_rfc3339();
        let gate = GateRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            binding_id: input.binding_id,
            node_id: input.node_id,
            gate_type: input.gate_type,
            status: GateStatus::Open,
            artifact_refs: input.artifact_refs,
            created_at: now.clone(),
            updated_at: now,
            resolved_at: None,
            comment: None,
            requested_change: None,
        };

        write_json(
            &self.gate_path(&gate.project_id, &gate.issue_id, &id),
            &gate,
        )?;
        Ok(gate)
    }

    pub fn get(
        &self,
        project_id: &str,
        issue_id: &str,
        gate_id: &str,
    ) -> Result<GateRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(gate_id)?;
        let path = self.gate_path(project_id, issue_id, gate_id);
        if !path_exists(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "gate",
                id: gate_id.to_string(),
            });
        }

        read_json(&path)
    }

    pub fn resolve(
        &self,
        project_id: &str,
        issue_id: &str,
        gate_id: &str,
        status: GateStatus,
        comment: Option<String>,
        requested_change: Option<String>,
    ) -> Result<GateRecord, ProductStoreError> {
        let mut gate = self.get(project_id, issue_id, gate_id)?;
        let now = Utc::now().to_rfc3339();
        gate.status = status;
        gate.updated_at = now.clone();
        gate.resolved_at = Some(now);
        gate.comment = comment;
        gate.requested_change = requested_change;

        write_json(&self.gate_path(project_id, issue_id, gate_id), &gate)?;
        Ok(gate)
    }

    pub fn resolve_by_issue(
        &self,
        issue_id: &str,
        gate_id: &str,
        status: GateStatus,
        comment: Option<String>,
        requested_change: Option<String>,
    ) -> Result<GateRecord, ProductStoreError> {
        let project_ids = self.project_ids_for_gate(issue_id, gate_id)?;
        match project_ids.as_slice() {
            [project_id] => self.resolve(
                project_id,
                issue_id,
                gate_id,
                status,
                comment,
                requested_change,
            ),
            [] => Err(ProductStoreError::NotFound {
                kind: "gate",
                id: gate_id.to_string(),
            }),
            _ => Err(ProductStoreError::Io("gate_ambiguous".to_string())),
        }
    }

    pub fn project_ids_for_gate(
        &self,
        issue_id: &str,
        gate_id: &str,
    ) -> Result<Vec<String>, ProductStoreError> {
        validate_relative_id(issue_id)?;
        validate_relative_id(gate_id)?;
        let projects_root = self.paths.projects_root();
        if !path_exists(&projects_root)? {
            return Ok(Vec::new());
        }

        let mut projects = Vec::new();
        for entry in fs::read_dir(&projects_root).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", projects_root.display()))
        })? {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", projects_root.display()))
            })?;
            projects.push(entry.path());
        }
        projects.sort();

        let mut project_ids = Vec::new();
        for project_path in projects {
            let Some(project_id) = project_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let gate_path = self.gate_path(&project_id, issue_id, gate_id);
            if path_exists(&gate_path)? {
                project_ids.push(project_id);
            }
        }

        Ok(project_ids)
    }

    fn gates_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths.issue_root(project_id, issue_id).join("gates")
    }

    fn gate_path(&self, project_id: &str, issue_id: &str, gate_id: &str) -> PathBuf {
        self.gates_root(project_id, issue_id)
            .join(format!("{gate_id}.json"))
    }
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("try_exists {}: {error}", path.display())))
}

fn next_available_gate_id(gates: &[GateRecord]) -> String {
    let mut index = 1;
    loop {
        let id = format!("gate_{index:04}");
        if gates.iter().all(|gate| gate.id != id) {
            return id;
        }
        index += 1;
    }
}
