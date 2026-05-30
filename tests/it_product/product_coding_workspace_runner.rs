use cadence_aria::product::coding_models::CodingExecutionStage;
use cadence_aria::product::coding_workspace_runner::CodingRunnerCommand;
use cadence_aria::product::models::ProviderName;
use serde_json::json;

#[test]
fn coding_runner_commands_use_stable_ws_command_contract() {
    let provider_select = CodingRunnerCommand::ProviderSelect {
        role: "author".to_string(),
        provider: ProviderName::Codex,
    };
    let stage_confirm = CodingRunnerCommand::StageGateConfirm {
        stage: CodingExecutionStage::Testing,
    };

    assert_eq!(
        serde_json::to_value(provider_select).expect("serialize provider select command"),
        json!({
            "type": "provider_select",
            "role": "author",
            "provider": "codex"
        })
    );
    assert_eq!(
        serde_json::to_value(stage_confirm).expect("serialize stage confirm command"),
        json!({
            "type": "stage_gate_confirm",
            "stage": "testing"
        })
    );
    assert_eq!(
        serde_json::to_value(CodingRunnerCommand::AbortAttempt).expect("serialize abort command"),
        json!({
            "type": "abort_attempt"
        })
    );
}
