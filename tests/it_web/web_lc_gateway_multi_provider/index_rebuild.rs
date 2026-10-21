//! 聚合索引路由构造与 missing grace 决策(唯一计算点)。
//!
//! T25 诊断(t23 偶发 `index_rebuild_rejected (404)`):
//! - 产品把索引建库刻意 detach(初始化 Completed → Building 记录落盘之间
//!   有毫秒级空窗),`GET /aggregate-indexes/active` 可合法返回
//!   `state=missing`;
//! - harness 旧逻辑首见 missing 即 POST `aggregate-indexes:rebuild`(冒号式
//!   动作段只存在于 story-specs/design-specs 一族,索引端点从未注册)→
//!   路由未匹配 404 空 body,整轮硬失败且无重试。
//!
//! 抽离理由:`harness` 已按祖父条款登记 14966 行上限(棘轮只许删不许涨);
//! 本修复的路由构造与 grace 状态机整体落到本文件,harness 只留薄接线。

use std::time::{Duration, Instant};

/// 聚合索引 rebuild 触发路由(唯一注册形态:斜杠动作段,`src/web/app.rs`)。
///
pub(crate) fn aggregate_rebuild_uri(project_id: &str, lc_id: &str) -> String {
    format!("/api/projects/{project_id}/logical-codebases/{lc_id}/aggregate-indexes/rebuild")
}

/// 聚合索引 active 投影路由(读取口;与 rebuild 同族斜杠形态)。
pub(crate) fn aggregate_active_uri(project_id: &str, lc_id: &str) -> String {
    format!("/api/projects/{project_id}/logical-codebases/{lc_id}/aggregate-indexes/active")
}

/// missing 持续多久才允许触发一次 rebuild(有界 grace ≤30s:初始化
/// Completed 后 detached build 通常毫秒级落 Building 记录,真缺失才重建)。
pub(crate) const INDEX_MISSING_GRACE: Duration = Duration::from_secs(30);

/// missing grace 状态机:missing 是初始化 Completed 与 Building 记录落盘
/// 之间的合法瞬态,持续缺失满 grace 才触发一次 rebuild;观察到任何非
/// missing 态(building/终态)即复位——detached build 已在场就绝不重建。
pub(crate) struct IndexMissingGrace {
    grace: Duration,
    missing_since: Option<Instant>,
}

impl IndexMissingGrace {
    pub(crate) fn new(grace: Duration) -> Self {
        Self {
            grace,
            missing_since: None,
        }
    }

    /// 观察到 `state=missing`(或无记录):首次仅记起点;返回 true 表示
    /// 持续缺失已满 grace,调用方应触发一次 rebuild 并随即 `reset`。
    pub(crate) fn observe_missing(&mut self, now: Instant) -> bool {
        let since = *self.missing_since.get_or_insert(now);
        now.duration_since(since) >= self.grace
    }

    /// 复位 grace 窗口(观察到非 missing 态,或 rebuild 已触发):下次
    /// missing 重新吃满完整 grace,不连环 POST。
    pub(crate) fn reset(&mut self) {
        self.missing_since = None;
    }
}

#[cfg(test)]
mod index_rebuild_tests {
    use super::*;

    /// T25 负测 1:索引路由必须命中注册路由(斜杠动作段),全程无冒号
    /// (冒号式 `aggregate-indexes:rebuild` 未注册任何路由 → 404 空 body)。
    #[test]
    fn lcg_index_route_uris_match_registered_slash_routes() {
        let rebuild = aggregate_rebuild_uri("project_0001", "lc_123");
        assert_eq!(
            rebuild,
            "/api/projects/project_0001/logical-codebases/lc_123/aggregate-indexes/rebuild"
        );
        let active = aggregate_active_uri("project_0001", "lc_123");
        assert_eq!(
            active,
            "/api/projects/project_0001/logical-codebases/lc_123/aggregate-indexes/active"
        );
        assert!(
            !rebuild.contains(':') && !active.contains(':'),
            "索引路由不得出现冒号动作段:{rebuild} / {active}"
        );
    }

    /// T25 负测 2:harness 源内不得再出现冒号式 aggregate-indexes 路由
    /// (防修复被内联回退;唯一 rebuild 路由是斜杠形态)。
    #[test]
    fn lcg_harness_source_has_no_colon_rebuild_route() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/it_web/web_lc_gateway_multi_provider/harness.rs"
        ));
        assert!(
            !source.contains("aggregate-indexes:"),
            "harness 不得再写冒号式 aggregate-indexes 路由(T25:未匹配路由 404 空 body)"
        );
    }

    /// T25 行为测:先 missing 后 Building → grace 内不触发 rebuild
    /// (毫秒级空窗是合法瞬态,detached build 在场绝不重建)。
    #[test]
    fn lcg_missing_grace_waits_through_transient_missing_before_building() {
        let t0 = Instant::now();
        let mut grace = IndexMissingGrace::new(INDEX_MISSING_GRACE);
        assert!(
            !grace.observe_missing(t0),
            "首次 missing 只记起点,不得立即 rebuild"
        );
        assert!(
            !grace.observe_missing(t0 + Duration::from_millis(11)),
            "初始化 Completed 后 ~10ms 级空窗内的 missing 是合法瞬态,不得 rebuild"
        );
        // detached build 落了 Building 记录 → 复位;此后再 missing 重新计满。
        grace.reset();
        assert!(
            !grace.observe_missing(t0 + Duration::from_secs(60)),
            "Building 之后又见 missing 必须重新吃满 grace,不得沿用旧窗口"
        );
    }

    /// T25 行为测:持续缺失满 grace 才允许触发一次;触发复位后重新武装
    /// (不连环 POST rebuild——已有任务在跑会 409)。
    #[test]
    fn lcg_missing_grace_triggers_once_only_after_continuous_grace() {
        let t0 = Instant::now();
        let mut grace = IndexMissingGrace::new(INDEX_MISSING_GRACE);
        assert!(!grace.observe_missing(t0));
        assert!(
            !grace.observe_missing(t0 + INDEX_MISSING_GRACE - Duration::from_millis(1)),
            "grace 差 1ms 也不得触发"
        );
        assert!(
            grace.observe_missing(t0 + INDEX_MISSING_GRACE),
            "持续缺失满 grace 必须允许触发一次 rebuild"
        );
        grace.reset();
        assert!(
            !grace.observe_missing(t0 + INDEX_MISSING_GRACE * 3),
            "触发并复位后必须重新吃满 grace,不连环 POST"
        );
    }
}
