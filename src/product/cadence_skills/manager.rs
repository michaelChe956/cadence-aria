use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{
    CadenceSkillsError, CadenceSkillsPaths, CadenceSkillsPreparationResult,
    CadenceSkillsSourceMode, ManagedSkillLinkSynchronizer,
};
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};

const GIT_TIMEOUT: Duration = Duration::from_secs(180);
const OUTPUT_LIMIT: usize = 8 * 1024;
const CANDIDATE_URLS: [&str; 3] = [
    "https://ghfast.top/https://github.com/michaelChe956/Cadence-skills.git",
    "https://gh-proxy.com/https://github.com/michaelChe956/Cadence-skills.git",
    "https://mirror.ghproxy.com/https://github.com/michaelChe956/Cadence-skills.git",
];
static PREPARE_LOCK: Mutex<()> = Mutex::const_new(());

pub struct CadenceSkillsManager {
    paths: CadenceSkillsPaths,
    home: PathBuf,
    runner: Arc<dyn BoundedCommandRunner>,
    environment: BTreeMap<String, String>,
}

impl CadenceSkillsManager {
    pub fn new(home: impl AsRef<Path>) -> Self {
        let environment = std::env::var("PATH")
            .ok()
            .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
            .unwrap_or_default();
        Self::with_dependencies(home, Arc::new(TokioBoundedCommandRunner), environment)
    }

    pub fn with_dependencies(
        home: impl AsRef<Path>,
        runner: Arc<dyn BoundedCommandRunner>,
        environment: BTreeMap<String, String>,
    ) -> Self {
        let home = home.as_ref().to_path_buf();
        let environment = environment
            .into_iter()
            .filter(|(key, _)| key == "PATH")
            .collect();
        Self {
            paths: CadenceSkillsPaths::from_home(&home),
            home,
            runner,
            environment,
        }
    }

    pub fn paths(&self) -> &CadenceSkillsPaths {
        &self.paths
    }

    pub async fn prepare(
        &self,
        cancellation: CancellationToken,
    ) -> Result<CadenceSkillsPreparationResult, CadenceSkillsError> {
        let _guard = PREPARE_LOCK.lock().await;
        let (source_mode, git_updated, mut warnings) = if !self.paths.source_root().is_dir() {
            self.clone_source(cancellation.clone()).await?;
            (CadenceSkillsSourceMode::OnlineClone, true, Vec::new())
        } else if fs::symlink_metadata(self.paths.repository_root().join(".git")).is_ok() {
            self.update_source(cancellation.clone()).await?
        } else {
            (CadenceSkillsSourceMode::Offline, false, Vec::new())
        };

        let sync_result = ManagedSkillLinkSynchronizer::new(self.paths.clone()).synchronize()?;
        warnings.extend(sync_result.warnings);
        Ok(CadenceSkillsPreparationResult {
            source_mode,
            source_root: self.paths.source_root().to_path_buf(),
            skills_root: self.paths.shared_skills_root().to_path_buf(),
            git_updated,
            link_sync_status: sync_result.status,
            warnings,
        })
    }

    async fn clone_source(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(), CadenceSkillsError> {
        if fs::symlink_metadata(self.paths.repository_root()).is_ok() {
            return Err(CadenceSkillsError::unavailable(
                "clone_target_safety",
                "repository target already exists and its ownership cannot be proven",
                &offline_action(self.paths.repository_root()),
            ));
        }
        let repository_parent = self.paths.repository_root().parent().ok_or_else(|| {
            CadenceSkillsError::unavailable(
                "clone_target_safety",
                "repository target has no parent directory",
                &offline_action(self.paths.repository_root()),
            )
        })?;
        fs::create_dir_all(repository_parent).map_err(|error| {
            CadenceSkillsError::unavailable(
                "clone_target_parent",
                &sanitize_reason(&error.to_string()),
                &offline_action(self.paths.repository_root()),
            )
        })?;
        let mut failures = Vec::new();
        for url in CANDIDATE_URLS {
            let request = self.git_request(
                vec![
                    "clone".to_string(),
                    url.to_string(),
                    self.paths.repository_root().to_string_lossy().into_owned(),
                ],
                self.home.clone(),
                cancellation.clone(),
            );
            let result = self.runner.run(request.clone()).await;
            match result {
                Ok(result) if command_succeeded(&result) => {
                    if self.paths.source_root().is_dir() {
                        return Ok(());
                    }
                    failures.push(format!(
                        "clone {} reported success but cadence-init/skills is missing{}",
                        url,
                        self.cleanup_failed_clone_reason()
                    ));
                }
                Ok(result) => failures.push(format!(
                    "clone {}{}",
                    command_summary(&request, &result),
                    self.cleanup_failed_clone_reason()
                )),
                Err(error) => failures.push(format!(
                    "clone {}{}",
                    command_summary_for_error(&request, &error),
                    self.cleanup_failed_clone_reason()
                )),
            }
        }
        let reason = format!("all mirrors failed: {}", failures.join("; "));
        Err(CadenceSkillsError::unavailable(
            "clone",
            &reason,
            clone_download_action(),
        ))
    }

    fn cleanup_failed_clone_reason(&self) -> String {
        match fs::symlink_metadata(self.paths.repository_root()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => format!("; cleanup failed: {}", sanitize_reason(&error.to_string())),
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                match failed_clone_directory_is_managed(self.paths.repository_root()) {
                    Ok(true) => match fs::remove_dir_all(self.paths.repository_root()) {
                        Ok(()) => String::new(),
                        Err(error) => {
                            format!("; cleanup failed: {}", sanitize_reason(&error.to_string()))
                        }
                    },
                    Ok(false) => {
                        "; cleanup skipped: clone target contents are not provably managed"
                            .to_string()
                    }
                    Err(error) => {
                        format!("; cleanup failed: {}", sanitize_reason(&error.to_string()))
                    }
                }
            }
            Ok(_) => "; cleanup skipped: clone target is not a regular directory".to_string(),
        }
    }

    async fn update_source(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(CadenceSkillsSourceMode, bool, Vec<String>), CadenceSkillsError> {
        let start_index = self.ensure_origin_remote(cancellation.clone()).await?;
        self.fetch_with_rotation(start_index, cancellation.clone())
            .await?;

        let upstream_request = self.git_request(
            vec![
                "rev-parse".to_string(),
                "--abbrev-ref".to_string(),
                "--symbolic-full-name".to_string(),
                "@{u}".to_string(),
            ],
            self.paths.repository_root().to_path_buf(),
            cancellation.clone(),
        );
        let upstream = self
            .runner
            .run(upstream_request.clone())
            .await
            .map_err(|error| update_command_error("upstream", &upstream_request, &error))?;
        if !command_succeeded(&upstream) {
            if upstream.timed_out || upstream.cancelled || upstream.exit_code.is_none() {
                return Err(CadenceSkillsError::update_failed(
                    "upstream",
                    &command_summary(&upstream_request, &upstream),
                    "retry when Git is available",
                ));
            }
            if is_no_upstream_failure(&upstream) {
                return Ok((
                    CadenceSkillsSourceMode::OnlineUpdate,
                    false,
                    vec!["cadence_skills_no_upstream".to_string()],
                ));
            }
            return Err(CadenceSkillsError::update_failed(
                "upstream",
                &command_summary(&upstream_request, &upstream),
                "retry when Git is available",
            ));
        }

        self.run_update_command("pull", vec!["pull", "--ff-only"], cancellation)
            .await?;
        Ok((CadenceSkillsSourceMode::OnlineUpdate, true, Vec::new()))
    }

    async fn ensure_origin_remote(
        &self,
        cancellation: CancellationToken,
    ) -> Result<usize, CadenceSkillsError> {
        let request = self.git_request(
            vec![
                "remote".to_string(),
                "get-url".to_string(),
                "origin".to_string(),
            ],
            self.paths.repository_root().to_path_buf(),
            cancellation.clone(),
        );
        let result = self
            .runner
            .run(request.clone())
            .await
            .map_err(|error| update_command_error("remote", &request, &error))?;
        if !command_succeeded(&result) {
            return Err(CadenceSkillsError::update_failed(
                "remote",
                &command_summary(&request, &result),
                "retry when Git is available",
            ));
        }
        let current_url = result.stdout.trim();
        if let Some(index) = CANDIDATE_URLS.iter().position(|url| *url == current_url) {
            return Ok(index);
        }
        self.run_update_command(
            "remote",
            vec!["remote", "set-url", "origin", CANDIDATE_URLS[0]],
            cancellation,
        )
        .await?;
        Ok(0)
    }

    async fn fetch_with_rotation(
        &self,
        start_index: usize,
        cancellation: CancellationToken,
    ) -> Result<(), CadenceSkillsError> {
        let mut failures = Vec::new();
        for offset in 0..CANDIDATE_URLS.len() {
            let index = (start_index + offset) % CANDIDATE_URLS.len();
            if offset > 0 {
                self.run_update_command(
                    "remote",
                    vec!["remote", "set-url", "origin", CANDIDATE_URLS[index]],
                    cancellation.clone(),
                )
                .await?;
            }
            match self.try_fetch(cancellation.clone()).await {
                Ok(()) => return Ok(()),
                Err(reason) => failures.push(reason),
            }
        }
        Err(CadenceSkillsError::update_failed(
            "fetch",
            &format!("all mirrors failed: {}", failures.join("; ")),
            "retry when Git and the network are available",
        ))
    }

    async fn try_fetch(&self, cancellation: CancellationToken) -> Result<(), String> {
        let request = self.git_request(
            vec!["fetch".to_string(), "--all".to_string()],
            self.paths.repository_root().to_path_buf(),
            cancellation,
        );
        match self.runner.run(request.clone()).await {
            Ok(result) if command_succeeded(&result) => Ok(()),
            Ok(result) => Err(command_summary(&request, &result)),
            Err(error) => Err(command_summary_for_error(&request, &error)),
        }
    }

    async fn run_update_command(
        &self,
        stage: &str,
        argv: Vec<&str>,
        cancellation: CancellationToken,
    ) -> Result<(), CadenceSkillsError> {
        let request = self.git_request(
            argv.into_iter().map(ToString::to_string).collect(),
            self.paths.repository_root().to_path_buf(),
            cancellation,
        );
        let result = self
            .runner
            .run(request.clone())
            .await
            .map_err(|error| update_command_error(stage, &request, &error))?;
        if !command_succeeded(&result) {
            return Err(CadenceSkillsError::update_failed(
                stage,
                &command_summary(&request, &result),
                "retry when Git and the network are available",
            ));
        }
        Ok(())
    }

    fn git_request(
        &self,
        argv: Vec<String>,
        working_dir: PathBuf,
        cancellation: CancellationToken,
    ) -> BoundedCommandRequest {
        let mut environment = self.environment.clone();
        environment.insert("LC_ALL".to_string(), "C".to_string());
        BoundedCommandRequest {
            executable: "git".to_string(),
            argv,
            working_dir,
            timeout: GIT_TIMEOUT,
            cancellation,
            environment,
            stdout_limit: OUTPUT_LIMIT,
            stderr_limit: OUTPUT_LIMIT,
        }
    }
}

fn command_succeeded(result: &BoundedCommandResult) -> bool {
    result.exit_code == Some(0) && !result.timed_out && !result.cancelled
}

fn is_no_upstream_failure(result: &BoundedCommandResult) -> bool {
    result.exit_code.is_some_and(|code| code != 0)
        && result
            .stderr
            .to_ascii_lowercase()
            .contains("no upstream configured")
}

fn failed_clone_directory_is_managed(repository_root: &Path) -> std::io::Result<bool> {
    if fs::read_dir(repository_root)?.next().is_none() {
        return Ok(true);
    }
    Ok(fs::symlink_metadata(repository_root.join(".git"))
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false))
}

fn update_command_error(
    stage: &str,
    request: &BoundedCommandRequest,
    error: &BoundedCommandError,
) -> CadenceSkillsError {
    CadenceSkillsError::update_failed(
        stage,
        &format!(
            "{}: {}",
            command_label(request),
            sanitize_reason(&error.to_string())
        ),
        "retry when Git and the network are available",
    )
}

fn command_summary(request: &BoundedCommandRequest, result: &BoundedCommandResult) -> String {
    sanitize_reason(&format!(
        "{} exit={:?} timed_out={} cancelled={} stdout={} stderr={}{}{}",
        command_label(request),
        result.exit_code,
        result.timed_out,
        result.cancelled,
        result.stdout,
        result.stderr,
        if result.stdout_truncated {
            " stdout_truncated"
        } else {
            ""
        },
        if result.stderr_truncated {
            " stderr_truncated"
        } else {
            ""
        },
    ))
}

fn command_summary_for_error(
    request: &BoundedCommandRequest,
    error: &BoundedCommandError,
) -> String {
    sanitize_reason(&format!("{}: {}", command_label(request), error))
}

fn command_label(request: &BoundedCommandRequest) -> String {
    std::iter::once(request.executable.as_str())
        .chain(request.argv.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_reason(reason: &str) -> String {
    reason
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(1024)
        .collect()
}

fn offline_action(repository_root: &Path) -> String {
    format!(
        "install or copy Cadence-skills offline into {}",
        repository_root.display()
    )
}

fn clone_download_action() -> &'static str {
    "Cadence-skills 无法下载，请找管理员获取"
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use super::{CANDIDATE_URLS, CadenceSkillsManager};
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };

    fn mirror(index: usize) -> &'static str {
        CANDIDATE_URLS[index]
    }
    use crate::product::cadence_skills::{CadenceSkillsError, CadenceSkillsSourceMode};

    struct Step {
        result: Result<BoundedCommandResult, BoundedCommandError>,
        create_source: Option<std::path::PathBuf>,
        create_directory: Option<std::path::PathBuf>,
    }

    struct RecordingRunner {
        steps: Mutex<VecDeque<Step>>,
        requests: Mutex<Vec<BoundedCommandRequest>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        delay: Duration,
    }

    impl RecordingRunner {
        fn new(steps: Vec<Step>) -> Self {
            Self {
                steps: Mutex::new(steps.into()),
                requests: Mutex::new(Vec::new()),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                delay: Duration::ZERO,
            }
        }

        fn with_delay(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
        }

        fn requests(&self) -> Vec<BoundedCommandRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for RecordingRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            self.requests.lock().unwrap().push(request);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let step = self.steps.lock().unwrap().pop_front().unwrap();
            if let Some(source) = step.create_source {
                fs::create_dir_all(source.join("demo")).unwrap();
            }
            if let Some(directory) = step.create_directory {
                fs::create_dir_all(&directory).unwrap();
                fs::create_dir(directory.join(".git")).unwrap();
                fs::write(directory.join("partial-clone"), "partial").unwrap();
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            step.result
        }
    }

    fn success() -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn success_with_stdout(stdout: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn failure(stderr: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn step(result: Result<BoundedCommandResult, BoundedCommandError>) -> Step {
        Step {
            result,
            create_source: None,
            create_directory: None,
        }
    }

    fn manager(home: &Path, runner: Arc<dyn BoundedCommandRunner>) -> CadenceSkillsManager {
        CadenceSkillsManager::with_dependencies(
            home,
            runner,
            BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]),
        )
    }

    fn prepare_online_repository(home: &Path) {
        fs::create_dir_all(home.join(".agents/Cadence-skills/cadence-init/skills/demo")).unwrap();
        fs::create_dir_all(home.join(".agents/Cadence-skills/.git")).unwrap();
    }

    fn assert_git_request(request: &BoundedCommandRequest, argv: &[&str], cwd: &Path) {
        assert_eq!(request.executable, "git");
        assert_eq!(
            request.argv,
            argv.iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(request.working_dir, cwd);
        assert_eq!(request.timeout, Duration::from_secs(180));
        assert_eq!(
            request.environment.keys().collect::<Vec<_>>(),
            vec!["LC_ALL", "PATH"]
        );
        assert_eq!(
            request.environment.get("LC_ALL").map(String::as_str),
            Some("C")
        );
    }

    #[tokio::test]
    async fn clone_branch_uses_fixed_command_and_finishes_link_sync() {
        let home = TempDir::new().unwrap();
        let source = home
            .path()
            .join(".agents/Cadence-skills/cadence-init/skills");
        let runner = Arc::new(RecordingRunner::new(vec![Step {
            result: success(),
            create_source: Some(source.clone()),
            create_directory: None,
        }]));

        let result = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(result.source_mode, CadenceSkillsSourceMode::OnlineClone);
        assert!(result.git_updated);
        assert_eq!(
            fs::read_link(result.skills_root.join("demo")).unwrap(),
            source.join("demo")
        );
        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_git_request(
            &requests[0],
            &[
                "clone",
                mirror(0),
                home.path().join(".agents/Cadence-skills").to_str().unwrap(),
            ],
            home.path(),
        );
    }

    #[tokio::test]
    async fn clone_failure_and_missing_source_are_unavailable() {
        for steps in [
            vec![
                step(failure("network\nsecret")),
                step(failure("mirror two down")),
                step(failure("mirror three down")),
            ],
            vec![step(success()), step(success()), step(success())],
        ] {
            let home = TempDir::new().unwrap();
            let error = manager(home.path(), Arc::new(RecordingRunner::new(steps)))
                .prepare(CancellationToken::new())
                .await
                .unwrap_err();
            assert_eq!(error.code(), "cadence_skills_unavailable");
            assert!(
                error
                    .to_string()
                    .contains("Cadence-skills 无法下载，请找管理员获取")
            );
            assert!(!error.to_string().contains('\n'));
            assert!(!home.path().join(".agents/Cadence-skills").exists());
        }
    }

    #[tokio::test]
    async fn clone_falls_through_to_next_mirror_after_failure_and_cleanup() {
        let home = TempDir::new().unwrap();
        let repository = home.path().join(".agents/Cadence-skills");
        let source = repository.join("cadence-init/skills");
        let runner = Arc::new(RecordingRunner::new(vec![
            Step {
                result: failure("network"),
                create_source: None,
                create_directory: Some(repository.clone()),
            },
            Step {
                result: success(),
                create_source: Some(source),
                create_directory: None,
            },
        ]));
        let manager = manager(home.path(), runner.clone());

        let result = manager.prepare(CancellationToken::new()).await.unwrap();

        assert_eq!(result.source_mode, CadenceSkillsSourceMode::OnlineClone);
        assert!(result.git_updated);
        let requests = runner.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].argv[1], mirror(0));
        assert_eq!(requests[1].argv[1], mirror(1));
    }

    #[tokio::test]
    async fn clone_all_mirrors_failing_reports_aggregated_reason() {
        let home = TempDir::new().unwrap();
        let runner = Arc::new(RecordingRunner::new(vec![
            step(failure("mirror one down")),
            step(failure("mirror two down")),
            step(failure("mirror three down")),
        ]));

        let error = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap_err();

        assert_eq!(error.code(), "cadence_skills_unavailable");
        assert!(error.to_string().contains("mirror one down"));
        assert!(error.to_string().contains("mirror two down"));
        assert!(error.to_string().contains("mirror three down"));
        assert!(error.to_string().contains(mirror(0)));
        assert!(error.to_string().contains(mirror(2)));
        assert!(
            error
                .to_string()
                .contains("Cadence-skills 无法下载，请找管理员获取")
        );
        assert!(!error.to_string().contains('\n'));
        assert_eq!(runner.requests().len(), 3);
        assert!(!home.path().join(".agents/Cadence-skills").exists());
    }

    #[tokio::test]
    async fn clone_refuses_to_overwrite_an_existing_unknown_repository_target() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".agents/Cadence-skills")).unwrap();
        let runner = Arc::new(RecordingRunner::new(Vec::new()));

        let error = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap_err();

        assert_eq!(error.code(), "cadence_skills_unavailable");
        assert!(runner.requests().is_empty());
    }

    #[tokio::test]
    async fn update_branch_fetches_detects_upstream_and_pulls_ff_only() {
        let home = TempDir::new().unwrap();
        prepare_online_repository(home.path());
        let runner = Arc::new(RecordingRunner::new(vec![
            step(success_with_stdout(&format!("{}\n", mirror(0)))),
            step(success()),
            step(success()),
            step(success()),
        ]));

        let result = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(result.source_mode, CadenceSkillsSourceMode::OnlineUpdate);
        assert!(result.git_updated);
        let cwd = home.path().join(".agents/Cadence-skills");
        let requests = runner.requests();
        assert_eq!(requests.len(), 4);
        assert_git_request(&requests[0], &["remote", "get-url", "origin"], &cwd);
        assert_git_request(&requests[1], &["fetch", "--all"], &cwd);
        assert_git_request(
            &requests[2],
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            &cwd,
        );
        assert_git_request(&requests[3], &["pull", "--ff-only"], &cwd);
    }

    #[tokio::test]
    async fn update_migrates_outdated_origin_before_fetch() {
        let home = TempDir::new().unwrap();
        prepare_online_repository(home.path());
        let runner = Arc::new(RecordingRunner::new(vec![
            step(success_with_stdout(
                "https://gitee.com/michaelChe-World/Cadence-skills.git\n",
            )),
            step(success()),
            step(success()),
            step(success()),
            step(success()),
        ]));

        let result = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(result.source_mode, CadenceSkillsSourceMode::OnlineUpdate);
        assert!(result.git_updated);
        let cwd = home.path().join(".agents/Cadence-skills");
        let requests = runner.requests();
        assert_eq!(requests.len(), 5);
        assert_git_request(&requests[0], &["remote", "get-url", "origin"], &cwd);
        assert_git_request(
            &requests[1],
            &["remote", "set-url", "origin", mirror(0)],
            &cwd,
        );
        assert_git_request(&requests[2], &["fetch", "--all"], &cwd);
        assert_git_request(
            &requests[3],
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            &cwd,
        );
        assert_git_request(&requests[4], &["pull", "--ff-only"], &cwd);
    }

    #[tokio::test]
    async fn origin_remote_failures_are_update_errors() {
        for (steps, expected_requests) in [
            (
                vec![step(failure("fatal: No such remote 'origin'"))],
                1_usize,
            ),
            (
                vec![
                    step(success_with_stdout(
                        "https://github.com/michaelChe956/Cadence-skills\n",
                    )),
                    step(failure("error: could not set remote url")),
                ],
                2_usize,
            ),
        ] {
            let home = TempDir::new().unwrap();
            prepare_online_repository(home.path());
            let runner = Arc::new(RecordingRunner::new(steps));
            let error = manager(home.path(), runner.clone())
                .prepare(CancellationToken::new())
                .await
                .unwrap_err();
            assert_eq!(error.code(), "cadence_skills_update_failed");
            assert_eq!(runner.requests().len(), expected_requests);
            assert!(!home.path().join(".agents/skills/demo").exists());
        }
    }

    #[tokio::test]
    async fn update_without_upstream_warns_and_skips_pull() {
        let home = TempDir::new().unwrap();
        prepare_online_repository(home.path());
        let runner = Arc::new(RecordingRunner::new(vec![
            step(success_with_stdout(&format!("{}\n", mirror(0)))),
            step(success()),
            step(failure("fatal: no upstream configured for branch 'main'")),
        ]));

        let result = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap();

        assert!(!result.git_updated);
        assert_eq!(result.warnings, vec!["cadence_skills_no_upstream"]);
        assert_eq!(runner.requests().len(), 3);
    }

    #[tokio::test]
    async fn unexpected_upstream_probe_failure_is_an_update_error() {
        let home = TempDir::new().unwrap();
        prepare_online_repository(home.path());
        let runner = Arc::new(RecordingRunner::new(vec![
            step(success_with_stdout(&format!("{}\n", mirror(0)))),
            step(success()),
            step(failure("fatal: bad object HEAD")),
        ]));

        let error = manager(home.path(), runner)
            .prepare(CancellationToken::new())
            .await
            .unwrap_err();

        assert_eq!(error.code(), "cadence_skills_update_failed");
        assert!(error.to_string().contains("bad object HEAD"));
    }

    #[tokio::test]
    async fn fetch_failure_rotates_mirrors_before_failing() {
        let home = TempDir::new().unwrap();
        prepare_online_repository(home.path());
        let runner = Arc::new(RecordingRunner::new(vec![
            step(success_with_stdout(&format!("{}\n", mirror(0)))),
            step(failure("fetch mirror one down")),
            step(success()),
            step(success()),
            step(success()),
            step(success()),
        ]));

        let result = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(result.source_mode, CadenceSkillsSourceMode::OnlineUpdate);
        assert!(result.git_updated);
        let cwd = home.path().join(".agents/Cadence-skills");
        let requests = runner.requests();
        assert_eq!(requests.len(), 6);
        assert_git_request(&requests[0], &["remote", "get-url", "origin"], &cwd);
        assert_git_request(&requests[1], &["fetch", "--all"], &cwd);
        assert_git_request(
            &requests[2],
            &["remote", "set-url", "origin", mirror(1)],
            &cwd,
        );
        assert_git_request(&requests[3], &["fetch", "--all"], &cwd);
        assert_git_request(
            &requests[4],
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            &cwd,
        );
        assert_git_request(&requests[5], &["pull", "--ff-only"], &cwd);
    }

    #[tokio::test]
    async fn fetch_all_mirrors_failing_reports_aggregated_reason() {
        let home = TempDir::new().unwrap();
        prepare_online_repository(home.path());
        let runner = Arc::new(RecordingRunner::new(vec![
            step(success_with_stdout(&format!("{}\n", mirror(1)))),
            step(failure("fetch mirror two down")),
            step(success()),
            step(failure("fetch mirror three down")),
            step(success()),
            step(failure("fetch mirror one down")),
        ]));

        let error = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap_err();

        assert_eq!(error.code(), "cadence_skills_update_failed");
        assert!(error.to_string().contains("fetch mirror two down"));
        assert!(error.to_string().contains("fetch mirror three down"));
        assert!(error.to_string().contains("fetch mirror one down"));
        assert!(
            error
                .to_string()
                .contains("retry when Git and the network are available")
        );
        assert!(!error.to_string().contains('\n'));
        let requests = runner.requests();
        assert_eq!(requests.len(), 6);
        let cwd = home.path().join(".agents/Cadence-skills");
        assert_git_request(
            &requests[4],
            &["remote", "set-url", "origin", mirror(0)],
            &cwd,
        );
        assert!(!home.path().join(".agents/skills/demo").exists());
    }

    #[tokio::test]
    async fn fetch_and_pull_failures_block_before_link_sync() {
        for steps in [
            vec![
                step(success_with_stdout(&format!("{}\n", mirror(0)))),
                step(failure("fetch failed one")),
                step(success()),
                step(failure("fetch failed two")),
                step(success()),
                step(failure("fetch failed three")),
            ],
            vec![
                step(success_with_stdout(&format!("{}\n", mirror(0)))),
                step(success()),
                step(success()),
                step(failure("pull failed")),
            ],
        ] {
            let home = TempDir::new().unwrap();
            prepare_online_repository(home.path());
            let error = manager(home.path(), Arc::new(RecordingRunner::new(steps)))
                .prepare(CancellationToken::new())
                .await
                .unwrap_err();
            assert_eq!(error.code(), "cadence_skills_update_failed");
            assert!(!home.path().join(".agents/skills/demo").exists());
        }
    }

    #[tokio::test]
    async fn offline_branch_runs_no_git_and_produces_equivalent_links() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(
            home.path()
                .join(".agents/Cadence-skills/cadence-init/skills/demo"),
        )
        .unwrap();
        let runner = Arc::new(RecordingRunner::new(Vec::new()));

        let result = manager(home.path(), runner.clone())
            .prepare(CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(result.source_mode, CadenceSkillsSourceMode::Offline);
        assert!(!result.git_updated);
        assert!(runner.requests().is_empty());
        assert_eq!(
            fs::read_link(home.path().join(".codex/skills/skills/demo")).unwrap(),
            home.path().join(".agents/skills/demo")
        );
    }

    #[tokio::test]
    async fn process_wide_prepare_lock_serializes_short_lived_managers() {
        let first_home = TempDir::new().unwrap();
        let second_home = TempDir::new().unwrap();
        prepare_online_repository(first_home.path());
        prepare_online_repository(second_home.path());
        let runner = Arc::new(
            RecordingRunner::new(
                (0..8)
                    .map(|_| step(success_with_stdout(&format!("{}\n", mirror(0)))))
                    .collect(),
            )
            .with_delay(Duration::from_millis(20)),
        );
        let first = manager(first_home.path(), runner.clone());
        let second = manager(second_home.path(), runner.clone());

        let (first_result, second_result) = tokio::join!(
            first.prepare(CancellationToken::new()),
            second.prepare(CancellationToken::new())
        );

        first_result.unwrap();
        second_result.unwrap();
        assert_eq!(runner.max_active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stable_error_variant_is_available_to_callers() {
        let error = CadenceSkillsError::unavailable("clone", "failed", "offline");
        assert_eq!(error.code(), "cadence_skills_unavailable");
    }
}
