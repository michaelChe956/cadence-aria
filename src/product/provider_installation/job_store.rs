//! durable 安装作业存储（REQ-PCI-05 / tasks 11.6）。
//!
//! 状态机只允许 `running → completed | failed`；终态不可再迁移。作业
//! JSON 落状态根下 `provider-installation/jobs/<job_id>.json`，页面刷新
//! 或进程重启后仍可查询（「记录 running → 执行 → completed/failed」）。

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::failure_classify::InstallJobFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallJobStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallJobOutcome {
    /// 装完重探得到的本地版本；探测不到时为 `None`。
    pub installed_version: Option<String>,
    /// 命令不在 PATH 上时给出「把 `<prefix>/bin` 加入 PATH」提示。
    pub path_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallJob {
    pub job_id: String,
    /// 目标 wire 标识（claude_code/codex/pi/kimi_code/codegraph）。
    pub target: String,
    pub status: InstallJobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<InstallJobOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<InstallJobFailure>,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallJobStoreError {
    #[error("install job transition rejected: {0}")]
    InvalidTransition(String),
    #[error("install job store I/O failed: {0}")]
    Io(String),
}

impl From<std::io::Error> for InstallJobStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

const JOB_ID_MAX_LEN: usize = 64;

/// durable 作业存储；jobs_root 由 `app_paths` 决定。
pub struct InstallJobStore {
    jobs_root: PathBuf,
}

impl InstallJobStore {
    pub fn new(jobs_root: impl Into<PathBuf>) -> Self {
        Self {
            jobs_root: jobs_root.into(),
        }
    }

    /// 创建 `running` 作业（先落盘再返回，保证「记录 running → 执行」顺序）。
    pub fn create(
        &self,
        target_wire: &str,
        now: DateTime<Utc>,
    ) -> Result<InstallJob, InstallJobStoreError> {
        let job = InstallJob {
            job_id: Uuid::new_v4().to_string(),
            target: target_wire.to_string(),
            status: InstallJobStatus::Running,
            created_at: now,
            updated_at: now,
            outcome: None,
            failure: None,
        };
        self.write(&job)?;
        Ok(job)
    }

    pub fn get(&self, job_id: &str) -> Result<Option<InstallJob>, InstallJobStoreError> {
        if !valid_job_id(job_id) {
            return Ok(None);
        }
        match std::fs::read(self.job_path(job_id)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| InstallJobStoreError::Io(error.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(InstallJobStoreError::Io(error.to_string())),
        }
    }

    /// `running → completed`（附结果）。
    pub fn complete(
        &self,
        job_id: &str,
        outcome: InstallJobOutcome,
        now: DateTime<Utc>,
    ) -> Result<InstallJob, InstallJobStoreError> {
        self.transition(job_id, now, move |job| {
            if job.status != InstallJobStatus::Running {
                return Err(format!(
                    "job {} is {:?}, only running jobs can complete",
                    job.job_id, job.status
                ));
            }
            job.status = InstallJobStatus::Completed;
            job.outcome = Some(outcome);
            Ok(())
        })
    }

    /// `running → failed`（附分类失败）。
    pub fn fail(
        &self,
        job_id: &str,
        failure: InstallJobFailure,
        now: DateTime<Utc>,
    ) -> Result<InstallJob, InstallJobStoreError> {
        self.transition(job_id, now, move |job| {
            if job.status != InstallJobStatus::Running {
                return Err(format!(
                    "job {} is {:?}, only running jobs can fail",
                    job.job_id, job.status
                ));
            }
            job.status = InstallJobStatus::Failed;
            job.failure = Some(failure);
            Ok(())
        })
    }

    fn transition(
        &self,
        job_id: &str,
        now: DateTime<Utc>,
        mutate: impl FnOnce(&mut InstallJob) -> Result<(), String>,
    ) -> Result<InstallJob, InstallJobStoreError> {
        let mut job = self.get(job_id)?.ok_or_else(|| {
            InstallJobStoreError::InvalidTransition(format!("job {job_id} not found"))
        })?;
        mutate(&mut job).map_err(InstallJobStoreError::InvalidTransition)?;
        job.updated_at = now;
        self.write(&job)?;
        Ok(job)
    }

    fn job_path(&self, job_id: &str) -> PathBuf {
        self.jobs_root.join(format!("{job_id}.json"))
    }

    fn write(&self, job: &InstallJob) -> Result<(), InstallJobStoreError> {
        std::fs::create_dir_all(&self.jobs_root)?;
        let bytes =
            serde_json::to_vec(job).map_err(|error| InstallJobStoreError::Io(error.to_string()))?;
        // 原子写（temp + rename），崩溃不留半份作业 JSON。
        let temp_path = self.jobs_root.join(format!(".{}.tmp", job.job_id));
        std::fs::write(&temp_path, bytes)?;
        std::fs::rename(&temp_path, self.job_path(&job.job_id))?;
        Ok(())
    }
}

/// job_id 只允许 uuid 形态字符（数字/字母/连字符，防路径穿越）。
fn valid_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && job_id.len() <= JOB_ID_MAX_LEN
        && job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;
    use crate::product::provider_installation::failure_classify::InstallFailureKind;

    fn now(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 8, 0, 0)
            .single()
            .expect("fixed timestamp")
    }

    fn failure() -> InstallJobFailure {
        InstallJobFailure {
            kind: InstallFailureKind::Network,
            message: "装不上（网络问题）。".to_string(),
            manual_command:
                "npm install -g --registry=https://registry.npmmirror.com @openai/codex".to_string(),
        }
    }

    fn outcome() -> InstallJobOutcome {
        InstallJobOutcome {
            installed_version: Some("1.2.3".to_string()),
            path_hint: None,
        }
    }

    /// 测试点 #7：创建后重建 store 仍能 get（durable）。
    #[test]
    fn job_store_jobs_survive_store_rebuild() {
        let root = tempdir().expect("root");
        let store = InstallJobStore::new(root.path().join("jobs"));
        let job = store.create("codex", now(1)).expect("create job");

        // 模拟进程重启：全新 store 实例指向同一目录。
        let rebuilt = InstallJobStore::new(root.path().join("jobs"));
        let reloaded = rebuilt
            .get(&job.job_id)
            .expect("store io")
            .expect("durable job must survive restart");
        assert_eq!(reloaded, job);
        assert_eq!(reloaded.status, InstallJobStatus::Running);
    }

    /// 测试点 #7：状态机只允许 running→completed/failed。
    #[test]
    fn job_store_rejects_terminal_to_terminal_transitions() {
        let root = tempdir().expect("root");
        let store = InstallJobStore::new(root.path().join("jobs"));

        let job = store.create("codex", now(1)).expect("create");
        let completed = store
            .complete(&job.job_id, outcome(), now(2))
            .expect("running → completed");
        assert_eq!(completed.status, InstallJobStatus::Completed);
        assert_eq!(completed.outcome, Some(outcome()));
        assert_eq!(completed.updated_at, now(2));

        assert!(
            store
                .complete(&job.job_id, outcome(), now(3))
                .is_err_and(|error| matches!(error, InstallJobStoreError::InvalidTransition(_))),
            "completed 不可再 complete"
        );
        assert!(
            store
                .fail(&job.job_id, failure(), now(3))
                .is_err_and(|error| matches!(error, InstallJobStoreError::InvalidTransition(_))),
            "completed 不可改判 failed"
        );

        let second = store.create("pi", now(1)).expect("create second");
        let failed = store
            .fail(&second.job_id, failure(), now(2))
            .expect("running → failed");
        assert_eq!(failed.status, InstallJobStatus::Failed);
        assert_eq!(failed.failure, Some(failure()));
        assert!(
            store
                .fail(&second.job_id, failure(), now(3))
                .is_err_and(|error| matches!(error, InstallJobStoreError::InvalidTransition(_))),
            "failed 不可再迁移"
        );
    }

    #[test]
    fn job_store_rejects_path_traversal_job_ids() {
        let root = tempdir().expect("root");
        let store = InstallJobStore::new(root.path().join("jobs"));
        assert_eq!(store.get("../evil").expect("io"), None);
        assert_eq!(store.get("a/b").expect("io"), None);
        assert_eq!(store.get("").expect("io"), None);
    }
}
