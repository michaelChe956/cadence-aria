use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::cross_cutting::adapter_compatibility::{CommandSpec, default_compatibility_matrix};
use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use crate::cross_cutting::kimi_code_provider::MIN_KIMI_VERSION;
use crate::product::models::ProviderName;
use crate::protocol::contracts::ProviderType;

const PROVIDER_HEALTH_SCHEMA_VERSION: u32 = 1;
const DEFAULT_HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_OUTPUT_LIMIT: usize = 4096;
const MAX_REASON_LENGTH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthReasonCode {
    CommandMissing,
    Timeout,
    NonZeroExit,
    VersionUnparseable,
    VersionTooLow,
    IoError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthEntry {
    pub provider: ProviderName,
    pub command: String,
    pub available: bool,
    pub version: Option<String>,
    pub reason_code: Option<ProviderHealthReasonCode>,
    pub reason: Option<String>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthSnapshot {
    pub schema_version: u32,
    pub generation: u64,
    pub checked_at: DateTime<Utc>,
    pub providers: Vec<ProviderHealthEntry>,
}

impl ProviderHealthSnapshot {
    pub fn entry(&self, provider: &ProviderName) -> Option<&ProviderHealthEntry> {
        self.providers
            .iter()
            .find(|entry| &entry.provider == provider)
    }

    pub fn real_workflow_blocked(&self) -> bool {
        [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
        .iter()
        .all(|provider| !self.entry(provider).is_some_and(|entry| entry.available))
    }
}

pub trait ProviderHealthClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProviderHealthClock;

impl ProviderHealthClock for SystemProviderHealthClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderHealthRefreshError {
    #[error("failed to persist provider health snapshot: {0}")]
    Persist(String),
}

struct ProviderHealthState {
    snapshot: Arc<ProviderHealthSnapshot>,
    latest_diagnostic: Arc<ProviderHealthSnapshot>,
    degraded: bool,
}

pub struct ProviderHealthService {
    paths: AriaStatePaths,
    runner: Arc<dyn BoundedCommandRunner>,
    clock: Arc<dyn ProviderHealthClock>,
    timeout: Duration,
    output_limit: usize,
    state: RwLock<ProviderHealthState>,
    refresh_lock: Mutex<()>,
}

impl ProviderHealthService {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self::with_dependencies(
            AriaStatePaths::from_workspace_root(workspace_root),
            Arc::new(TokioBoundedCommandRunner),
            Arc::new(SystemProviderHealthClock),
            DEFAULT_HEALTH_TIMEOUT,
            DEFAULT_OUTPUT_LIMIT,
        )
    }

    pub fn with_dependencies(
        paths: AriaStatePaths,
        runner: Arc<dyn BoundedCommandRunner>,
        clock: Arc<dyn ProviderHealthClock>,
        timeout: Duration,
        output_limit: usize,
    ) -> Self {
        let initial = Arc::new(uninitialized_snapshot());
        Self {
            paths,
            runner,
            clock,
            timeout,
            output_limit,
            state: RwLock::new(ProviderHealthState {
                snapshot: initial.clone(),
                latest_diagnostic: initial,
                degraded: true,
            }),
            refresh_lock: Mutex::new(()),
        }
    }

    pub fn paths(&self) -> &AriaStatePaths {
        &self.paths
    }

    pub fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.state
            .read()
            .expect("provider health state")
            .snapshot
            .clone()
    }

    pub fn latest_diagnostic(&self) -> Arc<ProviderHealthSnapshot> {
        self.state
            .read()
            .expect("provider health state")
            .latest_diagnostic
            .clone()
    }

    pub fn degraded(&self) -> bool {
        self.state.read().expect("provider health state").degraded
    }

    pub fn real_workflow_blocked(&self) -> bool {
        self.degraded() || self.snapshot().real_workflow_blocked()
    }

    pub async fn refresh(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Arc<ProviderHealthSnapshot>, ProviderHealthRefreshError> {
        let _refresh_guard = self.refresh_lock.lock().await;
        let generation = self.latest_diagnostic().generation.saturating_add(1);
        let checked_at = self.clock.now();
        let matrix = default_compatibility_matrix();
        let claude = matrix
            .entry_for(ProviderType::ClaudeCode)
            .expect("default Claude compatibility entry")
            .version_command
            .clone();
        let codex = matrix
            .entry_for(ProviderType::Codex)
            .expect("default Codex compatibility entry")
            .version_command
            .clone();
        let pi = pi_version_command();
        let kimi = kimi_version_command();

        let (claude, codex, pi, kimi) = tokio::join!(
            self.probe_provider(
                ProviderName::ClaudeCode,
                claude,
                checked_at,
                cancellation.clone()
            ),
            self.probe_provider(ProviderName::Codex, codex, checked_at, cancellation.clone()),
            self.probe_provider(ProviderName::Pi, pi, checked_at, cancellation.clone()),
            self.probe_provider(ProviderName::KimiCode, kimi, checked_at, cancellation)
        );
        let diagnostic = Arc::new(ProviderHealthSnapshot {
            schema_version: PROVIDER_HEALTH_SCHEMA_VERSION,
            generation,
            checked_at,
            providers: vec![claude, codex, pi, kimi],
        });

        {
            let mut state = self.state.write().expect("provider health state");
            state.latest_diagnostic = diagnostic.clone();
        }

        if let Err(error) = persist_snapshot(&self.paths, diagnostic.as_ref()).await {
            self.state.write().expect("provider health state").degraded = true;
            return Err(ProviderHealthRefreshError::Persist(error.to_string()));
        }

        let mut state = self.state.write().expect("provider health state");
        state.snapshot = diagnostic.clone();
        state.latest_diagnostic = diagnostic.clone();
        state.degraded = false;
        Ok(diagnostic)
    }

    async fn probe_provider(
        &self,
        provider: ProviderName,
        command: CommandSpec,
        checked_at: DateTime<Utc>,
        cancellation: CancellationToken,
    ) -> ProviderHealthEntry {
        let mut environment = BTreeMap::new();
        if let Some(path) = std::env::var_os("PATH") {
            environment.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        }
        let request = BoundedCommandRequest {
            executable: command.program.clone(),
            argv: command.args.clone(),
            working_dir: self.paths.workspace_root().to_path_buf(),
            timeout: self.timeout,
            cancellation,
            environment,
            stdout_limit: self.output_limit,
            stderr_limit: self.output_limit,
        };
        let command_text = format_command(&command);

        match self.runner.run(request).await {
            Ok(result) => entry_from_result(provider, command_text, checked_at, result),
            Err(BoundedCommandError::CommandMissing { details, .. }) => unavailable_entry(
                provider,
                command_text,
                checked_at,
                ProviderHealthReasonCode::CommandMissing,
                details,
            ),
            Err(BoundedCommandError::Io { details }) => unavailable_entry(
                provider,
                command_text,
                checked_at,
                ProviderHealthReasonCode::IoError,
                details,
            ),
        }
    }
}

fn pi_version_command() -> CommandSpec {
    CommandSpec::new("pi", vec!["--version".to_string()])
}

fn kimi_version_command() -> CommandSpec {
    CommandSpec::new("kimi", vec!["--version".to_string()])
}

fn uninitialized_snapshot() -> ProviderHealthSnapshot {
    let checked_at = Utc
        .timestamp_opt(0, 0)
        .single()
        .expect("Unix epoch timestamp");
    let matrix = default_compatibility_matrix();
    let commands = [
        (
            ProviderName::ClaudeCode,
            matrix
                .entry_for(ProviderType::ClaudeCode)
                .expect("default Claude compatibility entry")
                .version_command
                .clone(),
        ),
        (
            ProviderName::Codex,
            matrix
                .entry_for(ProviderType::Codex)
                .expect("default Codex compatibility entry")
                .version_command
                .clone(),
        ),
        (ProviderName::Pi, pi_version_command()),
        (ProviderName::KimiCode, kimi_version_command()),
    ];
    let providers = commands
        .into_iter()
        .map(|(provider, command)| {
            unavailable_entry(
                provider,
                format_command(&command),
                checked_at,
                ProviderHealthReasonCode::IoError,
                "provider health has not been refreshed",
            )
        })
        .collect();
    ProviderHealthSnapshot {
        schema_version: PROVIDER_HEALTH_SCHEMA_VERSION,
        generation: 0,
        checked_at,
        providers,
    }
}

fn entry_from_result(
    provider: ProviderName,
    command: String,
    checked_at: DateTime<Utc>,
    result: BoundedCommandResult,
) -> ProviderHealthEntry {
    if result.timed_out {
        return unavailable_entry(
            provider,
            command,
            checked_at,
            ProviderHealthReasonCode::Timeout,
            "provider version command timed out",
        );
    }
    if result.cancelled {
        return unavailable_entry(
            provider,
            command,
            checked_at,
            ProviderHealthReasonCode::IoError,
            "provider version command was cancelled",
        );
    }
    if result.exit_code != Some(0) {
        let reason = first_non_empty_line(&result.stderr)
            .or_else(|| first_non_empty_line(&result.stdout))
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "provider version command exited with {:?}",
                    result.exit_code
                )
            });
        return unavailable_entry(
            provider,
            command,
            checked_at,
            ProviderHealthReasonCode::NonZeroExit,
            reason,
        );
    }

    let stdout = first_non_empty_line(&result.stdout);
    let stderr = first_non_empty_line(&result.stderr);
    let version = stdout
        .and_then(parse_version_token)
        .or_else(|| stderr.and_then(parse_version_token));
    let Some(version) = version else {
        return unavailable_entry(
            provider,
            command,
            checked_at,
            ProviderHealthReasonCode::VersionUnparseable,
            stdout
                .or(stderr)
                .unwrap_or("provider version command produced no output"),
        );
    };

    if provider == ProviderName::KimiCode && !kimi_version_supported(&version) {
        return unavailable_entry(
            provider,
            command,
            checked_at,
            ProviderHealthReasonCode::VersionTooLow,
            format!(
                "Kimi Code version {version} is below the minimum supported version {MIN_KIMI_VERSION}; upgrade Kimi Code"
            ),
        );
    }

    ProviderHealthEntry {
        provider,
        command,
        available: true,
        version: Some(version),
        reason_code: None,
        reason: None,
        checked_at,
    }
}

fn unavailable_entry(
    provider: ProviderName,
    command: String,
    checked_at: DateTime<Utc>,
    reason_code: ProviderHealthReasonCode,
    reason: impl AsRef<str>,
) -> ProviderHealthEntry {
    ProviderHealthEntry {
        provider,
        command,
        available: false,
        version: None,
        reason_code: Some(reason_code),
        reason: Some(sanitize_reason(reason.as_ref())),
        checked_at,
    }
}

fn first_non_empty_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

fn parse_version_token(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+'))
        });
        token
            .chars()
            .any(|character| character.is_ascii_digit())
            .then(|| token.to_string())
    })
}

fn kimi_version_supported(version: &str) -> bool {
    let numeric = version.trim_start_matches(|character: char| !character.is_ascii_digit());
    let mut components = numeric.split('.').map(|component| {
        component
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .parse::<u64>()
            .ok()
    });
    let Some(major) = components.next().flatten() else {
        return false;
    };
    let Some(minor) = components.next().flatten() else {
        return false;
    };
    let patch = components.next().flatten().unwrap_or(0);
    let minimum = MIN_KIMI_VERSION
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .expect("MIN_KIMI_VERSION must be a three-part numeric version");
    (major, minor, patch) >= (minimum[0], minimum[1], minimum[2])
}

fn sanitize_reason(reason: &str) -> String {
    let cleaned = reason
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>();
    cleaned.trim().chars().take(MAX_REASON_LENGTH).collect()
}

fn format_command(command: &CommandSpec) -> String {
    std::iter::once(command.program.as_str())
        .chain(command.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn persist_snapshot(
    paths: &AriaStatePaths,
    snapshot: &ProviderHealthSnapshot,
) -> std::io::Result<()> {
    let destination = paths.provider_health_file();
    let parent = destination
        .parent()
        .expect("provider health path has parent");
    tokio::fs::create_dir_all(parent).await?;
    let temp_path = parent.join(format!(".provider-health.json.tmp-{}", Uuid::new_v4()));
    let serialized = serde_json::to_vec_pretty(snapshot).map_err(std::io::Error::other)?;
    let write_result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await?;
        file.write_all(&serialized).await?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp_path, &destination).await
    }
    .await;
    if write_result.is_err() {
        let _ = tokio::fs::remove_file(&temp_path).await;
    }
    write_result
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use chrono::{DateTime, TimeZone, Utc};
    use tokio_util::sync::CancellationToken;

    use super::{
        ProviderHealthClock, ProviderHealthReasonCode, ProviderHealthService,
        ProviderHealthSnapshot, kimi_version_command,
    };
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::models::ProviderName;

    #[derive(Clone)]
    struct FixedClock(DateTime<Utc>);

    impl ProviderHealthClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    struct ScriptedRunner {
        responses:
            Mutex<HashMap<String, VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        delay: Duration,
    }

    impl ScriptedRunner {
        fn new(
            claude: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            codex: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            pi: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
        ) -> Self {
            Self::with_kimi(
                claude,
                codex,
                pi,
                vec![success("kimi 0.34.0\n", ""), success("kimi 0.34.0\n", "")],
            )
        }

        fn with_kimi(
            claude: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            codex: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            pi: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            kimi: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
        ) -> Self {
            Self {
                responses: Mutex::new(HashMap::from([
                    ("claude".to_string(), claude.into()),
                    ("codex".to_string(), codex.into()),
                    ("pi".to_string(), pi.into()),
                    ("kimi".to_string(), kimi.into()),
                ])),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                delay: Duration::ZERO,
            }
        }

        fn with_delay(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
        }

        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            assert_eq!(request.argv, vec!["--version"]);
            assert!(request.environment.keys().all(|key| key == "PATH"));
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let response = self
                .responses
                .lock()
                .expect("responses")
                .get_mut(&request.executable)
                .and_then(VecDeque::pop_front)
                .expect("scripted response");
            self.active.fetch_sub(1, Ordering::SeqCst);
            response
        }
    }

    fn clock() -> Arc<dyn ProviderHealthClock> {
        Arc::new(FixedClock(
            Utc.with_ymd_and_hms(2026, 7, 12, 2, 41, 39).unwrap(),
        ))
    }

    fn success(stdout: &str, stderr: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 5,
        })
    }

    fn non_zero(stderr: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(7),
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 5,
        })
    }

    fn timed_out() -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 30,
        })
    }

    fn service(
        root: &std::path::Path,
        runner: Arc<dyn BoundedCommandRunner>,
    ) -> ProviderHealthService {
        ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(root),
            runner,
            clock(),
            Duration::from_secs(1),
            1024,
        )
    }

    #[test]
    fn kimi_version_command_uses_kimi_binary() {
        let command = kimi_version_command();
        assert_eq!(command.program, "kimi");
        assert_eq!(command.args, vec!["--version".to_string()]);
    }

    #[tokio::test]
    async fn provider_health_marks_kimi_below_minimum_version_unavailable() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::with_kimi(
            vec![success("claude 1.0\n", "")],
            vec![success("codex 1.0\n", "")],
            vec![success("pi 0.83.0\n", "")],
            vec![success("kimi 0.33.9\n", "")],
        ));
        let health = service(root.path(), runner);

        let snapshot = health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        let kimi = snapshot.entry(&ProviderName::KimiCode).expect("Kimi entry");

        assert!(!kimi.available);
        assert_eq!(
            kimi.reason_code,
            Some(ProviderHealthReasonCode::VersionTooLow)
        );
        assert!(kimi.reason.as_deref().unwrap().contains("0.34.0"));
    }

    #[tokio::test]
    async fn provider_health_refresh_probes_all_real_providers_in_parallel() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(
            ScriptedRunner::new(
                vec![success("claude 1.2.3\n", "")],
                vec![success("", "codex-cli 0.124.0\n")],
                vec![success("pi 0.83.0\n", "")],
            )
            .with_delay(Duration::from_millis(20)),
        );
        let health = service(root.path(), runner.clone());
        let snapshot = health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        let codex = snapshot.entry(&ProviderName::Codex).unwrap();
        let pi = snapshot.entry(&ProviderName::Pi).unwrap();
        assert_eq!(snapshot.generation, 1);
        assert_eq!(runner.max_active(), 4);
        assert_eq!(snapshot.providers.len(), 4);
        assert!(snapshot.entry(&ProviderName::ClaudeCode).unwrap().available);
        assert_eq!(codex.version.as_deref(), Some("0.124.0"));
        assert_eq!(pi.version.as_deref(), Some("0.83.0"));
        assert!(!snapshot.real_workflow_blocked());
        assert!(!health.degraded());
    }

    #[tokio::test]
    async fn provider_health_snapshot_excludes_fake_and_blocks_only_when_all_real_unavailable() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::with_kimi(
            vec![Err(BoundedCommandError::CommandMissing {
                executable: "claude".to_string(),
                details: "not found".to_string(),
            })],
            vec![non_zero("license expired\n")],
            vec![non_zero("pi unavailable\n")],
            vec![non_zero("kimi unavailable\n")],
        ));
        let health = service(root.path(), runner);
        let snapshot = health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        let claude = snapshot.entry(&ProviderName::ClaudeCode).unwrap();
        let codex = snapshot.entry(&ProviderName::Codex).unwrap();
        assert!(snapshot.entry(&ProviderName::Fake).is_none());
        assert!(snapshot.entry(&ProviderName::Pi).is_some());
        assert!(snapshot.entry(&ProviderName::KimiCode).is_some());
        assert!(snapshot.real_workflow_blocked());
        assert_eq!(
            claude.reason_code,
            Some(ProviderHealthReasonCode::CommandMissing)
        );
        assert_eq!(
            codex.reason_code,
            Some(ProviderHealthReasonCode::NonZeroExit)
        );
    }

    #[tokio::test]
    async fn provider_health_single_unavailable_provider_does_not_block_all_real_workflows() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![Err(BoundedCommandError::CommandMissing {
                executable: "claude".to_string(),
                details: "not found".to_string(),
            })],
            vec![success("codex 1.0\n", "")],
            vec![non_zero("pi unavailable\n")],
        ));
        let health = service(root.path(), runner);
        let snapshot = health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        assert!(!snapshot.real_workflow_blocked());
        assert!(!snapshot.entry(&ProviderName::ClaudeCode).unwrap().available);
        assert!(snapshot.entry(&ProviderName::Codex).unwrap().available);
    }

    #[tokio::test]
    async fn provider_health_accepts_parseable_version_from_either_output_stream() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![success(
                "Claude Code release channel stable\n",
                "claude 1.2.3\n",
            )],
            vec![success("codex 1.0\n", "")],
            vec![success("pi 0.83.0\n", "")],
        ));
        let health = service(root.path(), runner);
        let snapshot = health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        let claude = snapshot.entry(&ProviderName::ClaudeCode).unwrap();
        assert_eq!(claude.version.as_deref(), Some("1.2.3"));
    }

    #[tokio::test]
    async fn provider_health_normalizes_timeout_unparseable_and_io_reasons() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![timed_out(), success("claude version unknown\n", "")],
            vec![
                Err(BoundedCommandError::Io {
                    details: "broken pipe\nwith control\u{0000}".to_string(),
                }),
                success("codex 1.0\n", ""),
            ],
            vec![success("pi 0.83.0\n", ""), success("pi 0.83.0\n", "")],
        ));
        let health = service(root.path(), runner);
        let first = health
            .refresh(CancellationToken::new())
            .await
            .expect("first refresh");
        let first_claude = first.entry(&ProviderName::ClaudeCode).unwrap();
        let first_codex = first.entry(&ProviderName::Codex).unwrap();
        assert_eq!(
            first_claude.reason_code,
            Some(ProviderHealthReasonCode::Timeout)
        );
        assert_eq!(
            first_codex.reason_code,
            Some(ProviderHealthReasonCode::IoError)
        );
        assert!(!first_codex.reason.as_deref().unwrap().contains('\0'));
        let second = health
            .refresh(CancellationToken::new())
            .await
            .expect("second refresh");
        assert_eq!(second.generation, 2);
        assert_eq!(
            second.entry(&ProviderName::ClaudeCode).unwrap().reason_code,
            Some(ProviderHealthReasonCode::VersionUnparseable)
        );
    }

    #[tokio::test]
    async fn provider_health_refresh_atomically_replaces_persisted_snapshot() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![success("claude 1.0\n", ""), success("claude 2.0\n", "")],
            vec![success("codex 1.0\n", ""), success("codex 2.0\n", "")],
            vec![success("pi 0.83.0\n", ""), success("pi 0.84.0\n", "")],
        ));
        let health = service(root.path(), runner);
        health
            .refresh(CancellationToken::new())
            .await
            .expect("first refresh");
        let second = health
            .refresh(CancellationToken::new())
            .await
            .expect("second refresh");
        let persisted: ProviderHealthSnapshot = serde_json::from_slice(
            &fs::read(health.paths().provider_health_file()).expect("persisted snapshot"),
        )
        .expect("valid JSON");
        assert_eq!(persisted, *second);
        let state_dir = health
            .paths()
            .provider_health_file()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(fs::read_dir(state_dir).expect("state dir").all(|entry| {
            !entry
                .expect("state entry")
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        }));
    }

    #[tokio::test]
    async fn provider_health_persistence_failure_keeps_shared_snapshot_and_fails_closed() {
        let root = tempfile::tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![success("claude 1.0\n", ""), success("claude 2.0\n", "")],
            vec![success("codex 1.0\n", ""), success("codex 2.0\n", "")],
            vec![success("pi 0.83.0\n", ""), success("pi 0.84.0\n", "")],
        ));
        let health = service(root.path(), runner);
        let first = health
            .refresh(CancellationToken::new())
            .await
            .expect("first refresh");
        let state_dir = health
            .paths()
            .provider_health_file()
            .parent()
            .unwrap()
            .to_path_buf();
        fs::remove_dir_all(&state_dir).expect("remove state dir");
        fs::write(&state_dir, "not a directory").expect("block state dir creation");
        let error = health
            .refresh(CancellationToken::new())
            .await
            .expect_err("persistence must fail");
        assert!(error.to_string().contains("persist"));
        assert!(health.degraded());
        assert_eq!(health.snapshot().generation, first.generation);
        assert_eq!(health.latest_diagnostic().generation, first.generation + 1);
        assert!(health.real_workflow_blocked());
    }
}
