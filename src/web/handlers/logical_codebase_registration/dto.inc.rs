fn preflight_id() -> String {
    format!("preflight_{}", Uuid::new_v4().simple())
}

impl RegistrationPreflightItemDto {
    fn from_candidate(candidate: &crate::product::logical_codebase::RegistrationCandidate) -> Self {
        let class = match candidate.state {
            crate::product::logical_codebase::RegistrationCandidateState::Eligible => "eligible",
            crate::product::logical_codebase::RegistrationCandidateState::NonGit => "non_git",
            crate::product::logical_codebase::RegistrationCandidateState::Duplicate => "duplicate",
            crate::product::logical_codebase::RegistrationCandidateState::Nested => "nested",
            crate::product::logical_codebase::RegistrationCandidateState::Dirty
            | crate::product::logical_codebase::RegistrationCandidateState::NeedsAttention => {
                "needs_attention"
            }
            crate::product::logical_codebase::RegistrationCandidateState::Missing => "missing",
            crate::product::logical_codebase::RegistrationCandidateState::OutsideRoot => {
                "outside_root"
            }
        };
        let reason = (candidate.reason != "eligible").then(|| {
            if class == "needs_attention" && candidate.reason == "dirty_worktree" {
                "dirty".to_string()
            } else {
                candidate.reason.clone()
            }
        });
        Self {
            path: candidate.submitted_path.to_string_lossy().into_owned(),
            class: class.to_string(),
            reason,
        }
    }
}

impl RegistrationBatchDto {
    fn from_record(batch: &crate::product::logical_codebase::RegistrationBatchRecord) -> Self {
        Self {
            batch_id: batch.id.clone(),
            status: match batch.status {
                RegistrationBatchStatus::Queued => "queued",
                RegistrationBatchStatus::Running => "running",
                RegistrationBatchStatus::PartialFailed => "partial_failed",
                RegistrationBatchStatus::Completed => "completed",
                RegistrationBatchStatus::Cancelled => "cancelled",
            }
            .to_string(),
            items: batch
                .items
                .iter()
                .map(|item| RegistrationBatchItemDto {
                    path: item.submitted_path.to_string_lossy().into_owned(),
                    status: match item.status {
                        crate::product::logical_codebase::RegistrationItemStatus::Pending => {
                            "pending"
                        }
                        crate::product::logical_codebase::RegistrationItemStatus::Skipped => {
                            "skipped"
                        }
                        crate::product::logical_codebase::RegistrationItemStatus::Completed => {
                            "completed"
                        }
                        crate::product::logical_codebase::RegistrationItemStatus::Failed => {
                            "failed"
                        }
                        crate::product::logical_codebase::RegistrationItemStatus::NeedsAttention => {
                            "needs_attention"
                        }
                    }
                    .to_string(),
                    failure_reason: item.failure_reason.clone(),
                })
                .collect(),
        }
    }
}
