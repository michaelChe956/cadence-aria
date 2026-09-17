use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::Mutex as AsyncMutex;

use super::WorkspaceSessionManager;

/// 每个 durable workspace session 仅保留一个运行期 manager。
///
/// factory 在 map 锁外运行，避免慢 I/O 阻塞其他 session；同 session 的并发创建由
/// `creating` 互斥序列化，从而保证 engine 只会构建一次。
#[derive(Clone, Default)]
pub struct WorkspaceSessionRegistry {
    sessions: Arc<AsyncMutex<HashMap<String, Arc<WorkspaceSessionManager>>>>,
    creating: Arc<AsyncMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

impl WorkspaceSessionRegistry {
    pub async fn get_or_create<F, Fut>(
        &self,
        session_id: &str,
        factory: F,
    ) -> Result<Arc<WorkspaceSessionManager>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Arc<WorkspaceSessionManager>, String>>,
    {
        if let Some(manager) = self.sessions.lock().await.get(session_id).cloned() {
            return Ok(manager);
        }

        let creation_lock = {
            let mut creating = self.creating.lock().await;
            creating
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let _creation_guard = creation_lock.lock().await;

        if let Some(manager) = self.sessions.lock().await.get(session_id).cloned() {
            return Ok(manager);
        }

        let manager = factory().await?;
        let mut sessions = self.sessions.lock().await;
        Ok(sessions
            .entry(session_id.to_string())
            .or_insert_with(|| manager.clone())
            .clone())
    }

    /// 仅供测试及后续 Task 3 回收点观察；生产代码不遍历。
    pub async fn session_ids(&self) -> Vec<String> {
        let mut ids = self.sessions.lock().await.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// Task 3 将在满足回收不变式后调用；Task 1 只提供入口，不引入回收策略。
    pub async fn remove(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
        self.creating.lock().await.remove(session_id);
    }
}
