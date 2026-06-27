use crate::product::coding_models::{CodingTimelineNode, CodingTimelineNodeStatus};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

impl super::CodingAttemptStore {
    pub fn save_timeline_node(&self, node: CodingTimelineNode) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&node.attempt_id)?;
        let path = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("timeline-nodes.json");
        let mut nodes: Vec<CodingTimelineNode> = if super::path_is_regular_file(&path)? {
            read_json(&path)?
        } else {
            Vec::new()
        };
        if let Some(existing) = nodes.iter_mut().find(|existing| existing.id == node.id) {
            *existing = node;
        } else {
            nodes.push(node);
        }
        write_json(&path, &nodes)
    }

    pub fn get_timeline_nodes(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingTimelineNode>, ProductStoreError> {
        let path = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("timeline-nodes.json");
        if !super::path_is_regular_file(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_timeline_node_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        node_id: &str,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(node_id)?;
        let path = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("timeline-nodes.json");
        let mut nodes: Vec<CodingTimelineNode> = if super::path_is_regular_file(&path)? {
            read_json(&path)?
        } else {
            return Err(ProductStoreError::NotFound {
                kind: "coding_timeline_node",
                id: node_id.to_string(),
            });
        };
        let Some(node) = nodes.iter_mut().find(|node| node.id == node_id) else {
            return Err(ProductStoreError::NotFound {
                kind: "coding_timeline_node",
                id: node_id.to_string(),
            });
        };
        node.status = status;
        node.summary = summary;
        node.completed_at = completed_at;
        write_json(&path, &nodes)
    }
}
