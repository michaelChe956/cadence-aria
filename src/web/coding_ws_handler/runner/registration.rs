use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry};

pub(super) struct CodingRunnerRegistrationGuard {
    registry: CodingRunRegistry,
    attempt_key: CodingAttemptRunKey,
    run_id: u64,
}

impl CodingRunnerRegistrationGuard {
    pub(super) fn new(
        registry: CodingRunRegistry,
        attempt_key: CodingAttemptRunKey,
        run_id: u64,
    ) -> Self {
        Self {
            registry,
            attempt_key,
            run_id,
        }
    }
}

impl Drop for CodingRunnerRegistrationGuard {
    fn drop(&mut self) {
        self.registry.remove(&self.attempt_key, self.run_id);
    }
}
