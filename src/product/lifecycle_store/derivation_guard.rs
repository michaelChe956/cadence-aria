use crate::product::models::LifecycleConfirmationStatus;

use super::LifecycleStore;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DerivationGuardError {
    #[error("story_spec_not_found")]
    StorySpecNotFound,
    #[error("story_spec_not_confirmed")]
    StorySpecNotConfirmed,
    #[error("产品存储错误：{0}")]
    Store(String),
}

impl DerivationGuardError {
    /// 返回可供群聊引擎和 API 层复用的稳定错误码。
    pub const fn code(&self) -> &'static str {
        match self {
            Self::StorySpecNotFound => "story_spec_not_found",
            Self::StorySpecNotConfirmed => "story_spec_not_confirmed",
            Self::Store(_) => "product_store_error",
        }
    }
}

pub fn validate_design_finalize_allowed(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    story_entity_id: &str,
) -> Result<(), DerivationGuardError> {
    let stories = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(|error| DerivationGuardError::Store(error.to_string()))?;
    let story = stories
        .iter()
        .find(|story| story.id == story_entity_id)
        .ok_or(DerivationGuardError::StorySpecNotFound)?;

    if story.confirmation_status != LifecycleConfirmationStatus::Confirmed {
        return Err(DerivationGuardError::StorySpecNotConfirmed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{CreateStorySpecInput, LifecycleStore};
    use crate::product::models::LifecycleConfirmationStatus;

    use super::*;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const REPOSITORY_ID: &str = "repository_0001";

    fn setup() -> (TempDir, LifecycleStore) {
        let temp_dir = TempDir::new().expect("创建临时目录");
        let store = LifecycleStore::new(ProductAppPaths::new(temp_dir.path().join(".aria")));
        (temp_dir, store)
    }

    #[test]
    fn derivation_guard_rejects_unconfirmed_story_and_accepts_confirmed_story() {
        let (_temp_dir, lifecycle) = setup();
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "Story Spec".to_string(),
            })
            .expect("创建 Story Spec");

        let error = validate_design_finalize_allowed(&lifecycle, PROJECT_ID, ISSUE_ID, &story.id)
            .expect_err("未确认 Story Spec 不允许生成 Design Spec");
        assert!(matches!(error, DerivationGuardError::StorySpecNotConfirmed));
        assert_eq!(error.code(), "story_spec_not_confirmed");

        lifecycle
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &story.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .expect("确认 Story Spec");

        assert!(
            validate_design_finalize_allowed(&lifecycle, PROJECT_ID, ISSUE_ID, &story.id).is_ok()
        );
    }
}
