//! F-27：provider run 挂起 choice 的进程级登记簿（session_id 键控）。
//!
//! 为什么不用 engine 字段：durable 投影（`WorkspaceSessionManager` attach/
//! degraded 恢复）在 run 任务持有 engine 锁期间会 new 一个临时 engine 调
//! `build_session_state`——引擎字段在投影实例上恒空。挂起集必须跨 engine
//! 实例可见，session_state 全量投影才能在「choice 挂起 + run 持锁」窗口
//!（恰是 degraded 丢卡窗口）把 pending choice 带给已连接 tab。
//!
//! 生命周期与 `PendingChoiceRequests` guard 一致：guard Drop 时只摘除本 run
//! 插入的 id，run 结束（正常/取消/超时）后登记清空，投影不残留 stale 卡。

use std::collections::HashMap;

use crate::cross_cutting::streaming_provider::ChoiceRequestData;

/// F-27/F-59：登记簿条目——挂起请求 + 挂起起始时刻（epoch ms）+ 发问角色。
/// `created_at_ms` 是前端等待提示条「已等待时长 / 901s 超时倒计时」的锚点
///（与驱动循环 choice_wait_timer 的「pending 由空转非空起算」同源：应答
/// 清空后再次插入即新时刻）；`role` 供提示条标注发问方（author/reviewer）。
pub(crate) struct RegisteredPendingChoice {
    pub(crate) request: ChoiceRequestData,
    pub(crate) created_at_ms: u64,
    pub(crate) role: &'static str,
}

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

static PENDING_CHOICE_REGISTRY: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, HashMap<String, RegisteredPendingChoice>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn registry_insert(session_id: &str, request: &ChoiceRequestData, role: &'static str) {
    let mut registry = PENDING_CHOICE_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.entry(session_id.to_string()).or_default().insert(
        request.id.clone(),
        RegisteredPendingChoice {
            request: request.clone(),
            created_at_ms: epoch_ms_now(),
            role,
        },
    );
}

fn registry_remove(session_id: &str, id: &str) {
    let mut registry = PENDING_CHOICE_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entries) = registry.get_mut(session_id) {
        entries.remove(id);
        if entries.is_empty() {
            registry.remove(session_id);
        }
    }
}

/// F-27 只读访问器：某 session 当前挂起的 provider choice 全量快照（按 id
/// 稳定排序），供 `build_session_state` 组装 `pending_choice_requests` 投影。
pub(crate) fn pending_choice_requests_snapshot(session_id: &str) -> Vec<RegisteredPendingChoice> {
    let registry = PENDING_CHOICE_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entries) = registry.get(session_id) else {
        return Vec::new();
    };
    let mut requests: Vec<RegisteredPendingChoice> = entries
        .values()
        .map(|entry| RegisteredPendingChoice {
            request: entry.request.clone(),
            created_at_ms: entry.created_at_ms,
            role: entry.role,
        })
        .collect();
    requests.sort_by(|a, b| a.request.id.cmp(&b.request.id));
    requests
}

/// drive 循环的挂起 choice 集：本地 HashMap 语义不变，插入/移除即时镜像到
/// 进程级登记簿；Drop（循环任意出口）只清本 run 登记过的 id，防泄漏也不误伤
/// 并行其他 run（测试进程内同 session_id 并行驱动的场景）。
pub(crate) struct PendingChoiceRequests {
    session_id: String,
    entries: HashMap<String, ChoiceRequestData>,
}

impl PendingChoiceRequests {
    pub(crate) fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            entries: HashMap::new(),
        }
    }

    /// F-59：插入时携带发问角色（wire label），登记时刻由登记簿统一取钟。
    pub(crate) fn insert(&mut self, request: ChoiceRequestData, role: &'static str) {
        registry_insert(&self.session_id, &request, role);
        self.entries.insert(request.id.clone(), request);
    }

    pub(crate) fn remove(&mut self, id: &str) -> Option<ChoiceRequestData> {
        registry_remove(&self.session_id, id);
        self.entries.remove(id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn ids(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }
}

impl Drop for PendingChoiceRequests {
    fn drop(&mut self) {
        for id in self.entries.keys().cloned().collect::<Vec<_>>() {
            registry_remove(&self.session_id, &id);
        }
    }
}
