use serde::{Deserialize, Serialize};

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::json_store::{ProductStoreError, read_json, write_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpecGenerationMode {
    #[default]
    Pipeline,
    GroupChat,
}

pub fn load_spec_generation_mode(paths: &AriaStatePaths) -> SpecGenerationMode {
    read_json(&paths.spec_generation_mode_file()).unwrap_or_default()
}

pub fn save_spec_generation_mode(
    paths: &AriaStatePaths,
    mode: &SpecGenerationMode,
) -> Result<(), ProductStoreError> {
    write_json(&paths.spec_generation_mode_file(), mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_spec_generation_mode_defaults_to_pipeline() {
        let temp = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(temp.path());

        assert_eq!(
            load_spec_generation_mode(&paths),
            SpecGenerationMode::Pipeline
        );
    }

    #[test]
    fn saved_group_chat_spec_generation_mode_is_loaded() {
        let temp = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(temp.path());

        save_spec_generation_mode(&paths, &SpecGenerationMode::GroupChat).unwrap();

        assert_eq!(
            paths.spec_generation_mode_file(),
            temp.path().join(".aria/spec_generation_mode.json")
        );
        assert_eq!(
            load_spec_generation_mode(&paths),
            SpecGenerationMode::GroupChat
        );
    }

    #[test]
    fn spec_generation_mode_uses_snake_case_json_values() {
        assert_eq!(
            serde_json::to_string(&SpecGenerationMode::Pipeline).unwrap(),
            "\"pipeline\""
        );
        assert_eq!(
            serde_json::to_string(&SpecGenerationMode::GroupChat).unwrap(),
            "\"group_chat\""
        );
        assert_eq!(
            serde_json::from_str::<SpecGenerationMode>("\"unknown\"").unwrap_or_default(),
            SpecGenerationMode::Pipeline
        );
    }
}
