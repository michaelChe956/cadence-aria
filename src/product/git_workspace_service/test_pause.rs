use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

#[derive(Default)]
struct GitCommandPauseState {
    cwd: PathBuf,
    command_prefix: String,
    entered: Notify,
    released: Notify,
}

static NEXT_GIT_COMMAND_PAUSE_ID: AtomicU64 = AtomicU64::new(1);
static GIT_COMMAND_PAUSES: OnceLock<Mutex<HashMap<u64, Arc<GitCommandPauseState>>>> =
    OnceLock::new();

fn registered_pauses() -> &'static Mutex<HashMap<u64, Arc<GitCommandPauseState>>> {
    GIT_COMMAND_PAUSES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct GitCommandPause {
    id: u64,
    state: Arc<GitCommandPauseState>,
}

impl GitCommandPause {
    pub async fn wait_until_reached(&self) {
        self.state.entered.notified().await;
    }

    pub fn release(&self) {
        self.state.released.notify_one();
    }
}

impl Drop for GitCommandPause {
    fn drop(&mut self) {
        self.release();
        let mut registered = registered_pauses()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registered.remove(&self.id);
    }
}

pub fn pause_next_git_command_after_exit(cwd: &Path, command_prefix: &str) -> GitCommandPause {
    let id = NEXT_GIT_COMMAND_PAUSE_ID.fetch_add(1, Ordering::Relaxed);
    let state = Arc::new(GitCommandPauseState {
        cwd: cwd.to_path_buf(),
        command_prefix: command_prefix.to_string(),
        ..Default::default()
    });
    let mut registered = registered_pauses()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registered.insert(id, state.clone());
    GitCommandPause { id, state }
}

pub(super) async fn pause_git_command_after_exit_if_configured(cwd: &Path, args: &str) {
    let state = {
        let mut registered = registered_pauses()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let matched_id = registered.iter().find_map(|(id, state)| {
            (cwd == state.cwd && args.starts_with(&state.command_prefix)).then_some(*id)
        });
        matched_id.and_then(|id| registered.remove(&id))
    };
    if let Some(state) = state {
        state.entered.notify_one();
        state.released.notified().await;
    }
}
