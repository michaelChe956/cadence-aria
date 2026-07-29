use super::*;

impl RepositoryRegistrationCoordinator {
    pub(super) async fn git_finalize(
        &self,
        git_root: &Path,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, String> {
        self.require_git_finalize_success(
            git_root,
            vec!["add".to_string(), "-A".to_string()],
            cancellation.clone(),
            "git_finalize_add",
        )
        .await?;

        let staged = self
            .run_git(
                git_root,
                vec![
                    "diff".to_string(),
                    "--cached".to_string(),
                    "--quiet".to_string(),
                ],
                cancellation.clone(),
            )
            .await
            .map_err(|error| format!("git_finalize_diff: {error}"))?;
        match staged.exit_code {
            Some(0)
                if !staged.timed_out
                    && !staged.cancelled
                    && !staged.stdout_truncated
                    && !staged.stderr_truncated =>
            {
                return Ok(None);
            }
            Some(1)
                if !staged.timed_out
                    && !staged.cancelled
                    && !staged.stdout_truncated
                    && !staged.stderr_truncated => {}
            _ => {
                return Err(format!(
                    "git_finalize_diff: {}",
                    command_diagnostic(&staged)
                ));
            }
        }

        self.require_git_finalize_success(
            git_root,
            vec![
                "commit".to_string(),
                "-m".to_string(),
                "初始化cadence-aria 代码库".to_string(),
            ],
            cancellation.clone(),
            "git_finalize_commit",
        )
        .await?;

        let remotes = self
            .run_git(git_root, vec!["remote".to_string()], cancellation.clone())
            .await
            .map_err(|error| format!("git_finalize_remote: {error}"))?;
        if !command_succeeded(&remotes) || remotes.stdout_truncated || remotes.stderr_truncated {
            return Err(format!(
                "git_finalize_remote: {}",
                command_diagnostic(&remotes)
            ));
        }
        if remotes.stdout.trim().is_empty() {
            return Ok(Some(
                "git_finalize: 无 remote，已跳过 push，请手动推送".to_string(),
            ));
        }

        let upstream = self
            .run_git(
                git_root,
                vec![
                    "rev-parse".to_string(),
                    "--abbrev-ref".to_string(),
                    "--symbolic-full-name".to_string(),
                    "@{u}".to_string(),
                ],
                cancellation.clone(),
            )
            .await
            .map_err(|error| format!("git_finalize_upstream: {error}"))?;
        if !command_succeeded(&upstream) || upstream.stdout_truncated || upstream.stderr_truncated {
            return Ok(Some(
                "git_finalize: 无 upstream，已跳过 push，请手动推送".to_string(),
            ));
        }

        self.require_git_finalize_success(
            git_root,
            vec!["push".to_string()],
            cancellation,
            "git_finalize_push",
        )
        .await?;
        Ok(None)
    }

    pub(super) async fn require_git_finalize_success(
        &self,
        git_root: &Path,
        argv: Vec<String>,
        cancellation: CancellationToken,
        stage: &str,
    ) -> Result<(), String> {
        let result = self
            .run_git(git_root, argv, cancellation)
            .await
            .map_err(|error| format!("{stage}: {error}"))?;
        if command_succeeded(&result) && !result.stdout_truncated && !result.stderr_truncated {
            Ok(())
        } else {
            Err(format!("{stage}: {}", command_diagnostic(&result)))
        }
    }
}
