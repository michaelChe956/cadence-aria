use crate::protocol::contracts::ProviderType;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptInputMode {
    Stdin,
    Arg,
    TempFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutputMode {
    SentinelJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputParser {
    SentinelBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterCompatibilityEntry {
    pub provider_type: ProviderType,
    pub matrix_version: String,
    pub provider_command: PathBuf,
    pub probe_command: CommandSpec,
    pub version_command: CommandSpec,
    pub auth_check_command: CommandSpec,
    pub run_command: CommandSpec,
    pub unauthorized_patterns: Vec<String>,
    pub permission_denied_patterns: Vec<String>,
    pub structured_output_mode: StructuredOutputMode,
    pub prompt_input_mode: PromptInputMode,
    pub output_parser: OutputParser,
    pub supports_session: bool,
    pub supports_resume: bool,
    pub pass_worktree_path_as_arg: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterCompatibilityMatrix {
    pub entries: Vec<AdapterCompatibilityEntry>,
}

impl AdapterCompatibilityMatrix {
    pub fn entry_for(&self, provider_type: ProviderType) -> Option<&AdapterCompatibilityEntry> {
        self.entries
            .iter()
            .find(|entry| entry.provider_type == provider_type)
    }
}

pub fn default_compatibility_matrix() -> AdapterCompatibilityMatrix {
    AdapterCompatibilityMatrix {
        entries: vec![
            AdapterCompatibilityEntry {
                provider_type: ProviderType::ClaudeCode,
                matrix_version: "cli_matrix.v1".to_string(),
                provider_command: PathBuf::from("claude"),
                probe_command: CommandSpec::new("claude", vec!["--help".to_string()]),
                version_command: CommandSpec::new("claude", vec!["--version".to_string()]),
                auth_check_command: CommandSpec::new("claude", vec!["auth".to_string()]),
                run_command: CommandSpec::new(
                    "claude",
                    vec![
                        "-p".to_string(),
                        "--permission-mode".to_string(),
                        "dontAsk".to_string(),
                        "--tools".to_string(),
                        "".to_string(),
                        "--strict-mcp-config".to_string(),
                        "--no-session-persistence".to_string(),
                    ],
                ),
                unauthorized_patterns: vec![
                    "not logged in".to_string(),
                    "unauthorized".to_string(),
                    "token expired".to_string(),
                ],
                permission_denied_patterns: vec![
                    "permission denied".to_string(),
                    "operation not permitted".to_string(),
                ],
                structured_output_mode: StructuredOutputMode::SentinelJson,
                prompt_input_mode: PromptInputMode::Stdin,
                output_parser: OutputParser::SentinelBlock,
                supports_session: true,
                supports_resume: true,
                pass_worktree_path_as_arg: false,
            },
            AdapterCompatibilityEntry {
                provider_type: ProviderType::Codex,
                matrix_version: "cli_matrix.v1".to_string(),
                provider_command: PathBuf::from("codex"),
                probe_command: CommandSpec::new("codex", vec!["--help".to_string()]),
                version_command: CommandSpec::new("codex", vec!["--version".to_string()]),
                auth_check_command: CommandSpec::new("codex", vec!["auth".to_string()]),
                run_command: CommandSpec::new(
                    "codex",
                    vec![
                        "exec".to_string(),
                        "-s".to_string(),
                        "workspace-write".to_string(),
                    ],
                ),
                unauthorized_patterns: vec![
                    "not logged in".to_string(),
                    "unauthorized".to_string(),
                    "token expired".to_string(),
                ],
                permission_denied_patterns: vec![
                    "permission denied".to_string(),
                    "sandbox denied".to_string(),
                    "approval required".to_string(),
                ],
                structured_output_mode: StructuredOutputMode::SentinelJson,
                prompt_input_mode: PromptInputMode::Stdin,
                output_parser: OutputParser::SentinelBlock,
                supports_session: true,
                supports_resume: true,
                pass_worktree_path_as_arg: false,
            },
        ],
    }
}

pub fn fixture_compatibility_entry(
    provider_type: ProviderType,
    command_path: PathBuf,
) -> AdapterCompatibilityEntry {
    command_entry(provider_type, command_path.as_path())
}

fn command_entry(provider_type: ProviderType, path: &Path) -> AdapterCompatibilityEntry {
    let program = path.to_string_lossy().to_string();
    AdapterCompatibilityEntry {
        provider_type,
        matrix_version: "fixture_matrix.v1".to_string(),
        provider_command: path.to_path_buf(),
        probe_command: CommandSpec::new(program.clone(), vec!["probe".to_string()]),
        version_command: CommandSpec::new(program.clone(), vec!["version".to_string()]),
        auth_check_command: CommandSpec::new(program.clone(), vec!["auth".to_string()]),
        run_command: CommandSpec::new(program, vec!["run".to_string()]),
        unauthorized_patterns: vec![
            "not logged in".to_string(),
            "unauthorized".to_string(),
            "token expired".to_string(),
        ],
        permission_denied_patterns: vec![
            "permission denied".to_string(),
            "operation not permitted".to_string(),
            "sandbox denied".to_string(),
        ],
        structured_output_mode: StructuredOutputMode::SentinelJson,
        prompt_input_mode: PromptInputMode::Stdin,
        output_parser: OutputParser::SentinelBlock,
        supports_session: false,
        supports_resume: false,
        pass_worktree_path_as_arg: true,
    }
}
