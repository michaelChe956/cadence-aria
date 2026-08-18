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
