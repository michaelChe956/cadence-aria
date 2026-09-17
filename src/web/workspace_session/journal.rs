use std::collections::VecDeque;

/// run 完成后仍保留的最近事件数；活跃 run 不受该尾窗裁剪，以完整回放流式输出。
pub(crate) const JOURNAL_TAIL: u64 = 1_024;
/// 无论 run 是否活跃都不可突破的内存护栏。触顶代表历史已丢失，必须走快照恢复。
pub(crate) const JOURNAL_HARD_CAP: u64 = 65_536;

/// manager 私有的、单 session 事件回放窗口。
///
/// router 写入时已经对 JSON 注入 `event_seq`；这里保留该最终 wire 文本，保证同一份
/// 有序事件同时用于实时 fan-out 与未来重放。hard cap 发生后不猜测可否安全补发，直接
/// 返回 `None` 让调用方走 snapshot 基线。
#[derive(Default)]
pub(crate) struct EventJournal {
    pub(crate) entries: VecDeque<(u64, String)>,
    pub(crate) truncated: bool,
    pub(crate) run_active: bool,
}

impl EventJournal {
    pub(crate) fn push(&mut self, seq: u64, stamped: String) {
        self.entries.push_back((seq, stamped));
        while self.entries.len() > JOURNAL_HARD_CAP as usize {
            self.entries.pop_front();
            self.truncated = true;
        }
        if !self.run_active {
            self.trim_to_tail();
        }
    }

    pub(crate) fn mark_run_started(&mut self) {
        self.run_active = true;
    }

    pub(crate) fn mark_run_terminal(&mut self) {
        self.run_active = false;
        self.trim_to_tail();
    }

    /// `Some` 表示 cursor 位于未截断的当前窗口；回放只包含严格大于 cursor 的事件。
    /// Task 10 重订阅入口会消费该 API；Task 9 先建立并验证该回放边界。
    #[allow(dead_code)]
    pub(crate) fn replay_after(&self, cursor: u64) -> Option<Vec<String>> {
        if self.truncated {
            return None;
        }
        let oldest = self.oldest_seq()?;
        let latest = self.entries.back()?.0;
        if cursor < oldest.saturating_sub(1) || cursor > latest {
            return None;
        }
        Some(
            self.entries
                .iter()
                .filter(|(seq, _)| *seq > cursor)
                .map(|(_, stamped)| stamped.clone())
                .collect(),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn oldest_seq(&self) -> Option<u64> {
        self.entries.front().map(|(seq, _)| *seq)
    }

    fn trim_to_tail(&mut self) {
        while self.entries.len() > JOURNAL_TAIL as usize {
            self.entries.pop_front();
        }
    }
}
