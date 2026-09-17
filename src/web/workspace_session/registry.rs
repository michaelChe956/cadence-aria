use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use crate::web::workspace_ws_handler::OutboundControl;
use tokio::sync::Mutex as AsyncMutex;

use super::WorkspaceSessionManager;

/// 每个 durable workspace session 仅保留一个运行期 manager。
///
/// 附着注册与 idle 摘除在同一 `sessions` 互斥下完成：已通过身份核对的 manager
/// 不会在新连接登记 attachment 的间隙被回收。
#[derive(Clone, Default)]
pub struct WorkspaceSessionRegistry {
    sessions: Arc<AsyncMutex<HashMap<String, Arc<WorkspaceSessionManager>>>>,
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
        let mut sessions = self.sessions.lock().await;
        if let Some(manager) = sessions.get(session_id).cloned() {
            return Ok(manager);
        }

        let manager = factory().await?;
        Ok(sessions
            .entry(session_id.to_string())
            .or_insert_with(|| manager.clone())
            .clone())
    }

    /// 原子地获取或创建 manager 并登记 attachment。摘除使用同一互斥，从而不会把
    /// 已登记新连接的 manager 从 registry 中移走。
    pub(crate) async fn get_or_create_and_attach<F, Fut>(
        &self,
        session_id: &str,
        connection_id: &str,
        outbound_tx: tokio::sync::mpsc::Sender<OutboundControl>,
        factory: F,
    ) -> Result<Arc<WorkspaceSessionManager>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Arc<WorkspaceSessionManager>, String>>,
    {
        let mut sessions = self.sessions.lock().await;
        let manager = if let Some(manager) = sessions.get(session_id).cloned() {
            manager
        } else {
            let manager = factory().await?;
            sessions
                .entry(session_id.to_string())
                .or_insert_with(|| manager.clone())
                .clone()
        };
        manager.register_attachment(connection_id, outbound_tx);
        Ok(manager)
    }

    /// 返回已创建的 session manager，供集成测试观察运行期单例状态。
    pub async fn get(&self, session_id: &str) -> Option<Arc<WorkspaceSessionManager>> {
        self.sessions.lock().await.get(session_id).cloned()
    }

    /// 仅供测试及后续 Task 3 回收点观察；生产代码不遍历。
    pub async fn session_ids(&self) -> Vec<String> {
        let mut ids = self
            .sessions
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// 仅当 session 仍指向该 manager 且其在锁内复核为 idle 时摘除。
    pub async fn remove_if_idle(
        &self,
        session_id: &str,
        expected: &Arc<WorkspaceSessionManager>,
    ) -> bool {
        let mut sessions = self.sessions.lock().await;
        let Some(current) = sessions.get(session_id) else {
            return false;
        };
        if !Arc::ptr_eq(current, expected) || !expected.is_recyclable() {
            return false;
        }
        sessions.remove(session_id);
        true
    }

    /// 仅供单元测试验证幂等移除。
    pub async fn remove(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }
}
