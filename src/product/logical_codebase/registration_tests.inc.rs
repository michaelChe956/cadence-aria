#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
    use crate::product::logical_codebase::{
        LogicalCodebaseManifest, LogicalCodebaseStore,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    include!("registration_tests_detect_preflight.inc.rs");
    include!("registration_tests_attach.inc.rs");
    include!("registration_tests_aggregate_root_and_cold_start.inc.rs");
}
