use chrono::Utc;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, validate_relative_id, write_json};
use crate::product::models::ProviderReviewRoundRecord;

use super::{
    AppendProviderReviewRoundInput, LifecycleStore, count_json_files, ensure_target_absent,
};

impl LifecycleStore {
    pub fn append_provider_review_round(
        &self,
        input: AppendProviderReviewRoundInput,
    ) -> Result<ProviderReviewRoundRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.session_id)?;

        let root = self.provider_review_rounds_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("review_round", count_json_files(&root)?);
        let record = ProviderReviewRoundRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            session_id: input.session_id,
            round_index: input.round_index,
            author_provider: input.author_provider,
            reviewer_provider: input.reviewer_provider,
            review_result: input.review_result,
            revision_result: input.revision_result,
            created_at: Utc::now().to_rfc3339(),
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &record)?;
        Ok(record)
    }
}
