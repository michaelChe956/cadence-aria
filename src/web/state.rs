use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::web::events::EventHub;
use crate::web::runtime::WebRuntime;

#[derive(Clone)]
pub struct WebAppState {
    pub workspace_root: PathBuf,
    pub runtime: Arc<Mutex<WebRuntime>>,
    pub events: EventHub,
}

impl WebAppState {
    pub fn new(workspace_root: PathBuf, runtime: WebRuntime) -> Self {
        Self::with_events(workspace_root, runtime, EventHub::new())
    }

    pub fn with_events(workspace_root: PathBuf, runtime: WebRuntime, events: EventHub) -> Self {
        Self {
            workspace_root,
            runtime: Arc::new(Mutex::new(runtime)),
            events,
        }
    }
}
