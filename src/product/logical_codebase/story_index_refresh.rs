//! Story 前共享聚合索引刷新入口（片7，REQ-PLN-18 / Plan §11）。
//!
//! 这是 Story 专用的**窄 trait** 编排层，与既有规划路径的
//! `PlanningIndexFreshness`（`planning_context_resolver.rs`）完全独立：
//! 既有 trait 及其 3 个实现一行不改。三分支语义（顺序固定）：
//!
//! 1. **没有任何可读世代** ⇒ 首次建库（走既有
//!    `AggregateIndexOperation::build(project_id, expected_membership_revision)`）；
//! 2. **能确认没变**（Active-only 复用门 + 四条判据）⇒ 复用当前这一版，
//!    **不**触发同步；
//! 3. **有变化或不能确认没变** ⇒ 增量同步并重新验收（走既有
//!    `operation.sync_and_verify(project_id, prior)`）；仍非 Active ⇒
//!    停在 AI 启动之前。
//!
//! 失败面是**专用错误枚举**（稳定码 + detail），供上层映射为 422：
//! 不得让 `ProductStoreError::Io` 的通用 500 吞掉刷新失败。

use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
};

/// 同步成功但返回记录仍非 Active 时没有失败封套可解析，落该兜底稳定码
/// （与外层码同名，已登记于 `failure_copy_table.inc.rs`）。
const FALLBACK_STABLE_CODE: &str = "aggregate_index_unavailable";

/// Story 前刷新的三种结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryIndexRefreshOutcome {
    /// 分支 1：没有任何可读世代 ⇒ 首次建库成功。
    Rebuilt { aggregate_index_id: String },
    /// 分支 2：能确认没变 ⇒ 复用当前这一版（**未**触发同步）。
    Reused { aggregate_index_id: String },
    /// 分支 3：有变化或不能确认没变 ⇒ 增量同步 + 重新验收成功。
    Synced { aggregate_index_id: String },
}

/// Story 前刷新的失败（**停机**：AI 不启动）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryIndexRefreshError {
    /// 世代记录存在但读不出来 / 归属不明 / 彼此矛盾 ⇒ 按既有失败规则报错。
    /// **不得**当成「没有索引」静默首建。
    UnreadableGeneration { code: String, message: String },
    /// 刷新或验收失败 ⇒ 带稳定码 + 中文成品（REQ-PLN-18 失败面要求）。
    VerificationFailed { code: String, message: String },
}

/// Story 专用后端（**窄 trait**，与既有 `PlanningIndexFreshness` 完全独立）。
pub trait StoryIndexFreshness: Send + Sync {
    /// 读该 LC 的当前可读世代：
    ///   `Ok(None)` = 没有任何可读世代（分支 1 的入口条件）；
    ///   `Err`      = 记录读不出来 / 归属不明 / 彼此矛盾（**不得**当 None）。
    fn readable_generation(
        &self,
        project_id: &str,
    ) -> Result<Option<AggregateIndexRecord>, AggregateIndexError>;
    /// 当前快照（用于「能确认没变」的判定）。
    fn current_snapshots(
        &self,
        project_id: &str,
    ) -> Result<Vec<AggregateIndexMemberSnapshot>, AggregateIndexError>;
    /// 增量同步 + 重新验收（走既有 `operation.sync_and_verify(project_id, prior)`）。
    fn sync_and_verify(
        &self,
        project_id: &str,
        prior: AggregateIndexRecord,
    ) -> Result<AggregateIndexRecord, AggregateIndexError>;
    /// 首次建库（走既有 `operation.build(project_id, expected_membership_revision)`）。
    fn build(&self, project_id: &str) -> Result<AggregateIndexRecord, AggregateIndexError>;
    /// 按 lc_id 重新绑定（与既有 `PlanningIndexFreshness::for_lc` 同语义）。
    fn for_lc(&self, lc_id: &str) -> std::sync::Arc<dyn StoryIndexFreshness>;
}

/// 纯函数：Story 路径的「能确认没变」判定（**不依赖 freshness 的全局漂移判据**）。
///
/// 五条判据缺一不可（REQ-PLN-18 分支 2）：
/// 1. **前置（R3，Active-only）**：published 世代 `status == Active`；
///    `Degraded`（被降级的 LKG）/ `Stale`（已判漂移）一律不可复用；
/// 2. 成员集合未变（长度一致且逐位成员 id 相同）；
/// 3. 主工作目录身份未变（逐成员 `checkout_id` 相同）；
/// 4. Git 提交未变（逐成员 `revision` 相同）；
/// 5. **上次与本次都没有未提交修改**（逐成员 `!published.dirty && !current.dirty`，
///    以 `git status --porcelain` 为准；被忽略的文件不参与，见 G4）。
///
/// 注意（G2 已撤回）：「当前世代不含跳过成员」**不是**复用条件——可见范围内
/// 的任何变化已被「脏 ⇒ 同步」覆盖，被忽略的内容两处都不可见、不构成变更。
pub fn story_can_reuse(
    published: &AggregateIndexRecord,
    current: &[AggregateIndexMemberSnapshot],
) -> bool {
    // 1. 前置（R3，Active-only）：只有已验收为 Active 的世代才谈得上
    //    「能确认没变」；Degraded 是被降级的 LKG、Stale 已被判漂移。
    if published.status != AggregateIndexStatus::Active {
        return false;
    }
    // 2. 成员集合未变（长度一致且逐位成员 id 相同）。
    if published.member_snapshots.len() != current.len() {
        return false;
    }
    published
        .member_snapshots
        .iter()
        .zip(current)
        .all(|(before, now)| {
            before.logical_repository_id == now.logical_repository_id
            // 3. 主工作目录身份未变。
            && before.checkout_id == now.checkout_id
            // 4. Git 提交未变。
            && before.revision == now.revision
            // 5. 上次与本次都没有未提交修改（git status --porcelain 为准）。
            && !before.dirty
            && !now.dirty
        })
}

/// Story 前刷新的三分支编排（lc 解析不在本函数——由片7W 入口先
/// `resolve_issue_authority_lc_id` + `for_lc(lc_id)` 再调这里）。
pub fn refresh_for_story(
    project_id: &str,
    backend: &dyn StoryIndexFreshness,
) -> Result<StoryIndexRefreshOutcome, StoryIndexRefreshError> {
    let published = match backend.readable_generation(project_id) {
        // 分支 1：没有任何可读世代 ⇒ 首次建库并验收。
        Ok(None) => {
            return backend
                .build(project_id)
                .map(|record| StoryIndexRefreshOutcome::Rebuilt {
                    aggregate_index_id: record.aggregate_index_id,
                })
                .map_err(|error| verification_failed(&error));
        }
        // 记录读坏 / 归属不明 / 彼此矛盾 ⇒ 按既有失败规则报错，
        // 不得当成「没有索引」静默首建。
        Err(error) => {
            let (code, message) = stable_parts(&error);
            return Err(StoryIndexRefreshError::UnreadableGeneration { code, message });
        }
        Ok(Some(record)) => record,
    };
    // 分支 2（Active-only 复用门）：published 为 Active 且四条判据全部成立
    // 才复用。`current_snapshots` 返回 Err = 不能确认没变 ⇒ 落分支 3
    //（不是 Reused、也不是直接停机）。
    if published.status == AggregateIndexStatus::Active
        && backend
            .current_snapshots(project_id)
            .is_ok_and(|current| story_can_reuse(&published, &current))
    {
        return Ok(StoryIndexRefreshOutcome::Reused {
            aggregate_index_id: published.aggregate_index_id,
        });
    }
    // 分支 3：有变化或不能确认没变 ⇒ 增量同步 + 重新验收。
    // 判定依据是 sync_and_verify **返回的那份记录**（它就是本次验收发布的
    // 记录），不再跑一次 assess()——assess 会用当前工作区重新判 dirty，
    // 可能否定一次已经成功的验收（M5）。
    match backend.sync_and_verify(project_id, published) {
        Ok(record) if record.status == AggregateIndexStatus::Active => {
            Ok(StoryIndexRefreshOutcome::Synced {
                aggregate_index_id: record.aggregate_index_id,
            })
        }
        // 成功但返回记录仍非 Active ⇒ 停机（AI 不启动）；无失败封套可解析，
        // 落兜底稳定码。
        Ok(record) => Err(StoryIndexRefreshError::VerificationFailed {
            code: FALLBACK_STABLE_CODE.to_string(),
            message: format!(
                "aggregate index for {project_id} is {:?} after sync, expected active",
                record.status
            ),
        }),
        Err(error) => Err(verification_failed(&error)),
    }
}

/// 失败封套 ⇒ 停机错误：稳定码逐字透传，不得借道通用 500 丢码。
fn verification_failed(error: &AggregateIndexError) -> StoryIndexRefreshError {
    let (code, message) = stable_parts(error);
    StoryIndexRefreshError::VerificationFailed { code, message }
}

/// 失败封套 →（稳定码, 消息）：`Failed`/`Degraded` 两个变体都携带
/// `&'static str` 码，逐字透传（不得借道 `ProductStoreError::Io` 丢码）。
/// 「解析不出 ⇒ 兜底码」只发生在无封套的「同步后仍非 Active」路径。
fn stable_parts(error: &AggregateIndexError) -> (String, String) {
    match error {
        AggregateIndexError::Failed { code, message }
        | AggregateIndexError::Degraded { code, message } => ((*code).to_string(), message.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use super::*;
    use crate::product::logical_codebase::aggregate_index::types::AggregateIndexMemberSkipReason;
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};

    const MEMBER: &str = "018f0f8e-2c2d-7a10-8a11-111111111111";
    const CHECKOUT: &str = "018f0f8e-2c2d-7a10-8a11-222222222222";
    const ALT_CHECKOUT: &str = "018f0f8e-2c2d-7a10-8a11-333333333333";

    /// 快照构造：默认成员/主工作目录固定，revision 与 dirty 可变。
    fn snapshot(revision: &str, dirty: bool) -> AggregateIndexMemberSnapshot {
        snapshot_with(CHECKOUT, revision, dirty)
    }

    fn snapshot_with(checkout: &str, revision: &str, dirty: bool) -> AggregateIndexMemberSnapshot {
        AggregateIndexMemberSnapshot::indexed(
            LogicalRepositoryId(Uuid::parse_str(MEMBER).unwrap()),
            RepositoryCheckoutId(Uuid::parse_str(checkout).unwrap()),
            revision.to_string(),
            dirty,
            "2026-11-18T00:00:00Z".to_string(),
        )
    }

    /// 与 `active_generation()` 的成员快照完全一致的「当前快照」。
    fn clean_current() -> Vec<AggregateIndexMemberSnapshot> {
        vec![snapshot("rev-a", false)]
    }

    fn generation(
        aggregate_index_id: &str,
        status: AggregateIndexStatus,
        snapshots: Vec<AggregateIndexMemberSnapshot>,
    ) -> AggregateIndexRecord {
        AggregateIndexRecord {
            aggregate_index_id: aggregate_index_id.to_string(),
            project_id: "project_story".to_string(),
            membership_revision: 7,
            status,
            member_snapshots: snapshots,
            observed_after_member_snapshots: Vec::new(),
            codegraph_version: "1.6.3".to_string(),
            codegraph_root: PathBuf::new(),
            config_digest: String::new(),
            created_at: "2026-11-18T00:00:00Z".to_string(),
            updated_at: "2026-11-18T00:00:00Z".to_string(),
            command_id: None,
            supersedes_aggregate_index_id: None,
            warning: None,
            source_scan: None,
        }
    }

    /// 可读的 Active 世代（分支 2 的复用候选）。
    fn active_generation() -> AggregateIndexRecord {
        generation(
            "aggregate_index_story_published",
            AggregateIndexStatus::Active,
            clean_current(),
        )
    }

    fn failed_envelope(code: &'static str) -> AggregateIndexError {
        AggregateIndexError::Failed {
            code,
            message: format!("{code}: scripted failure"),
        }
    }

    #[derive(Default)]
    struct Calls {
        build_count: usize,
        sync_count: usize,
        priors: Vec<AggregateIndexRecord>,
    }

    /// 测试桩：结果可编排、调用可审计（计数 + 记录 `sync_and_verify`
    /// 收到的 `prior` 实参，钉住两实参签名）。
    struct ScriptedStoryBackend {
        /// 弹出式脚本：多于一个元素时弹出队首；只剩一个时重复它；空 ⇒ `Ok(None)`。
        readable: Mutex<VecDeque<Result<Option<AggregateIndexRecord>, AggregateIndexError>>>,
        current: Result<Vec<AggregateIndexMemberSnapshot>, AggregateIndexError>,
        sync_outcome: Result<AggregateIndexRecord, AggregateIndexError>,
        build_outcome: Result<AggregateIndexRecord, AggregateIndexError>,
        calls: Arc<Mutex<Calls>>,
    }

    impl ScriptedStoryBackend {
        #[allow(clippy::too_many_arguments)]
        fn new(
            readable: Vec<Result<Option<AggregateIndexRecord>, AggregateIndexError>>,
            current: Result<Vec<AggregateIndexMemberSnapshot>, AggregateIndexError>,
            sync_outcome: Result<AggregateIndexRecord, AggregateIndexError>,
            build_outcome: Result<AggregateIndexRecord, AggregateIndexError>,
        ) -> Self {
            Self {
                readable: Mutex::new(readable.into()),
                current,
                sync_outcome,
                build_outcome,
                calls: Arc::new(Mutex::new(Calls::default())),
            }
        }

        fn build_count(&self) -> usize {
            self.calls.lock().unwrap().build_count
        }

        fn sync_count(&self) -> usize {
            self.calls.lock().unwrap().sync_count
        }

        fn recorded_priors(&self) -> Vec<AggregateIndexRecord> {
            self.calls.lock().unwrap().priors.clone()
        }
    }

    impl StoryIndexFreshness for ScriptedStoryBackend {
        fn readable_generation(
            &self,
            _project_id: &str,
        ) -> Result<Option<AggregateIndexRecord>, AggregateIndexError> {
            let mut script = self.readable.lock().unwrap();
            if script.len() > 1 {
                script.pop_front().unwrap()
            } else {
                script.front().cloned().unwrap_or(Ok(None))
            }
        }

        fn current_snapshots(
            &self,
            _project_id: &str,
        ) -> Result<Vec<AggregateIndexMemberSnapshot>, AggregateIndexError> {
            self.current.clone()
        }

        fn sync_and_verify(
            &self,
            _project_id: &str,
            prior: AggregateIndexRecord,
        ) -> Result<AggregateIndexRecord, AggregateIndexError> {
            let mut calls = self.calls.lock().unwrap();
            calls.sync_count += 1;
            calls.priors.push(prior);
            drop(calls);
            self.sync_outcome.clone()
        }

        fn build(&self, _project_id: &str) -> Result<AggregateIndexRecord, AggregateIndexError> {
            self.calls.lock().unwrap().build_count += 1;
            self.build_outcome.clone()
        }

        fn for_lc(&self, _lc_id: &str) -> Arc<dyn StoryIndexFreshness> {
            // 桩语义：lc 重绑定共享同一份调用审计，配置原样保留。
            Arc::new(Self {
                readable: Mutex::new(self.readable.lock().unwrap().clone()),
                current: self.current.clone(),
                sync_outcome: self.sync_outcome.clone(),
                build_outcome: self.build_outcome.clone(),
                calls: Arc::clone(&self.calls),
            })
        }
    }

    #[test]
    fn story_can_reuse_is_true_when_nothing_changed_and_no_skips() {
        let published = active_generation();
        let current = clean_current();
        assert!(story_can_reuse(&published, &current));
    }

    #[test]
    fn story_can_reuse_is_false_when_published_or_current_is_dirty() {
        // 上次脏 / 本次脏 / 两边都脏（revision 相同）⇒ 一律不可复用。
        for (published_dirty, current_dirty) in [(true, false), (false, true), (true, true)] {
            let published = generation(
                "aggregate_index_story_published",
                AggregateIndexStatus::Active,
                vec![snapshot("rev-a", published_dirty)],
            );
            let current = vec![snapshot("rev-a", current_dirty)];
            assert!(
                !story_can_reuse(&published, &current),
                "published_dirty={published_dirty}, current_dirty={current_dirty}"
            );
        }
    }

    #[test]
    fn story_can_reuse_is_true_when_generation_contains_skipped_member() {
        // G2 已撤回「当前世代不含跳过成员」条件：含跳过成员不再强制同步。
        let mut skipped = snapshot("rev-a", false);
        skipped.mark_source_less();
        let published = generation(
            "aggregate_index_story_published",
            AggregateIndexStatus::Active,
            vec![skipped.clone()],
        );
        assert_eq!(
            published.member_snapshots[0].skip_reason,
            Some(AggregateIndexMemberSkipReason::NoSource)
        );
        assert!(story_can_reuse(&published, &[skipped]));
    }

    #[test]
    fn story_can_reuse_is_false_when_main_checkout_identity_changed() {
        let published = active_generation();
        let current = vec![snapshot_with(ALT_CHECKOUT, "rev-a", false)];
        assert!(!story_can_reuse(&published, &current));
    }

    #[test]
    fn story_can_reuse_is_false_when_revision_changed() {
        let published = active_generation();
        let current = vec![snapshot("rev-b", false)];
        assert!(!story_can_reuse(&published, &current));
    }

    #[test]
    fn refresh_for_story_builds_when_no_readable_generation() {
        let built = generation(
            "aggregate_index_story_built",
            AggregateIndexStatus::Active,
            clean_current(),
        );
        let backend = ScriptedStoryBackend::new(
            vec![Ok(None)],
            Err(failed_envelope("aggregate_index_store_error")),
            Err(failed_envelope("aggregate_index_member_coverage_failed")),
            Ok(built),
        );
        let outcome = refresh_for_story("project_story", &backend).expect("无索引 ⇒ 首次建库");
        assert_eq!(
            outcome,
            StoryIndexRefreshOutcome::Rebuilt {
                aggregate_index_id: "aggregate_index_story_built".to_string()
            }
        );
        assert_eq!(backend.build_count(), 1);
        assert_eq!(backend.sync_count(), 0, "分支 1 不得触发增量同步");
    }

    #[test]
    fn refresh_for_story_reuses_when_unchanged_and_does_not_sync() {
        let backend = ScriptedStoryBackend::new(
            vec![Ok(Some(active_generation()))],
            Ok(clean_current()),
            Err(failed_envelope("aggregate_index_member_coverage_failed")),
            Err(failed_envelope("aggregate_index_active_missing")),
        );
        let outcome =
            refresh_for_story("project_story", &backend).expect("能确认没变 ⇒ 复用当前版本");
        assert_eq!(
            outcome,
            StoryIndexRefreshOutcome::Reused {
                aggregate_index_id: "aggregate_index_story_published".to_string()
            }
        );
        assert_eq!(backend.sync_count(), 0, "复用门生效 ⇒ 不触发同步");
        assert_eq!(backend.build_count(), 0, "复用门生效 ⇒ 不触发建库");
    }

    #[test]
    fn refresh_for_story_syncs_when_visible_source_was_added() {
        // 可见范围内新增未跟踪文件 ⇒ `git status --porcelain` 非空 ⇒ 本次脏。
        let published = active_generation();
        let dirty_current = vec![snapshot("rev-a", true)];
        let backend = ScriptedStoryBackend::new(
            vec![Ok(Some(published.clone()))],
            Ok(dirty_current),
            Ok(generation(
                "aggregate_index_story_synced",
                AggregateIndexStatus::Active,
                vec![snapshot("rev-a", true)],
            )),
            Err(failed_envelope("aggregate_index_active_missing")),
        );
        let outcome = refresh_for_story("project_story", &backend).expect("变化 ⇒ 增量同步");
        assert_eq!(
            outcome,
            StoryIndexRefreshOutcome::Synced {
                aggregate_index_id: "aggregate_index_story_synced".to_string()
            }
        );
        assert_eq!(backend.sync_count(), 1);
        assert_eq!(backend.build_count(), 0);
        // 钉住 sync_and_verify(&self, project_id, prior) 的两实参签名：
        // prior 必须是本次读到的 published 世代。
        assert_eq!(backend.recorded_priors(), vec![published]);
    }

    #[test]
    fn refresh_for_story_reports_unreadable_generation_without_building() {
        let backend = ScriptedStoryBackend::new(
            vec![
                Err(failed_envelope("aggregate_index_active_ambiguous")),
                Ok(None),
            ],
            Err(failed_envelope("aggregate_index_store_error")),
            Err(failed_envelope("aggregate_index_member_coverage_failed")),
            Ok(generation(
                "aggregate_index_story_built",
                AggregateIndexStatus::Active,
                clean_current(),
            )),
        );
        // 归属不明 ⇒ UnreadableGeneration，不得当成「没有索引」静默首建。
        let first = refresh_for_story("project_story", &backend).unwrap_err();
        assert_eq!(
            first,
            StoryIndexRefreshError::UnreadableGeneration {
                code: "aggregate_index_active_ambiguous".to_string(),
                message: "aggregate_index_active_ambiguous: scripted failure".to_string(),
            }
        );
        assert_eq!(backend.build_count(), 0);
        assert_eq!(backend.sync_count(), 0);
        // 同一后端在**无记录**时返回 Ok(None)（与 ambiguous 区分）⇒ 此时才允许首建。
        let second = refresh_for_story("project_story", &backend).expect("Ok(None) ⇒ 首次建库");
        assert_eq!(
            second,
            StoryIndexRefreshOutcome::Rebuilt {
                aggregate_index_id: "aggregate_index_story_built".to_string()
            }
        );
        assert_eq!(backend.build_count(), 1);
    }

    #[test]
    fn refresh_for_story_stops_when_sync_result_is_not_active() {
        let published = active_generation();
        let dirty_current = vec![snapshot("rev-a", true)];
        let backend = ScriptedStoryBackend::new(
            vec![Ok(Some(published))],
            Ok(dirty_current),
            Ok(generation(
                "aggregate_index_story_synced",
                AggregateIndexStatus::Degraded,
                vec![snapshot("rev-a", true)],
            )),
            Err(failed_envelope("aggregate_index_active_missing")),
        );
        let error = refresh_for_story("project_story", &backend)
            .expect_err("同步后仍非 Active ⇒ 停在 AI 启动前");
        let StoryIndexRefreshError::VerificationFailed { code, message } = error else {
            panic!("同步后仍非 Active 必须是 VerificationFailed");
        };
        // 无失败封套可解析 ⇒ 兜底稳定码（与外层 422 码同名）。
        assert_eq!(code, "aggregate_index_unavailable");
        assert!(
            message.contains("Degraded"),
            "消息应点名实际状态：{message}"
        );
        assert_eq!(backend.sync_count(), 1);
    }

    #[test]
    fn refresh_for_story_reports_verification_failure_with_stable_code() {
        let published = active_generation();
        let backend = ScriptedStoryBackend::new(
            vec![Ok(Some(published))],
            Ok(vec![snapshot("rev-a", true)]),
            Err(failed_envelope("aggregate_index_member_coverage_failed")),
            Err(failed_envelope("aggregate_index_active_missing")),
        );
        let error = refresh_for_story("project_story", &backend).expect_err("同步失败 ⇒ 停机");
        assert_eq!(
            error,
            StoryIndexRefreshError::VerificationFailed {
                code: "aggregate_index_member_coverage_failed".to_string(),
                message: "aggregate_index_member_coverage_failed: scripted failure".to_string(),
            }
        );
    }

    #[test]
    fn refresh_for_story_reuses_only_when_published_generation_is_active() {
        // R3：复用门 = Active-only。Degraded / Stale 各一例——成员/revision/dirty
        // 全同、看起来「没变」——也必须走分支 3（同步 + 重新验收）。
        for status in [AggregateIndexStatus::Degraded, AggregateIndexStatus::Stale] {
            let published = generation("aggregate_index_story_published", status, clean_current());
            // 直接判定：非 Active 世代一票否决复用。
            assert!(!story_can_reuse(&published, &clean_current()), "{status:?}");
            let backend = ScriptedStoryBackend::new(
                vec![Ok(Some(published))],
                Ok(clean_current()),
                Ok(generation(
                    "aggregate_index_story_synced",
                    AggregateIndexStatus::Active,
                    clean_current(),
                )),
                Err(failed_envelope("aggregate_index_active_missing")),
            );
            let outcome = refresh_for_story("project_story", &backend)
                .unwrap_or_else(|_| panic!("{status:?} ⇒ 必须走同步分支"));
            assert_eq!(
                outcome,
                StoryIndexRefreshOutcome::Synced {
                    aggregate_index_id: "aggregate_index_story_synced".to_string()
                },
                "{status:?}"
            );
            assert_eq!(backend.sync_count(), 1, "{status:?}");
        }
        // 对照：唯一差别是 status == Active ⇒ Reused。
        let backend = ScriptedStoryBackend::new(
            vec![Ok(Some(active_generation()))],
            Ok(clean_current()),
            Ok(generation(
                "aggregate_index_story_synced",
                AggregateIndexStatus::Active,
                clean_current(),
            )),
            Err(failed_envelope("aggregate_index_active_missing")),
        );
        assert_eq!(
            refresh_for_story("project_story", &backend).unwrap(),
            StoryIndexRefreshOutcome::Reused {
                aggregate_index_id: "aggregate_index_story_published".to_string()
            }
        );
        assert_eq!(backend.sync_count(), 0);
    }

    #[test]
    fn refresh_for_story_syncs_when_current_snapshots_unavailable() {
        // 取不到当前快照 = 不能确认没变 ⇒ 落分支 3；
        // 不是 Reused、不是 UnreadableGeneration/VerificationFailed 提前停机。
        let published = active_generation();
        let backend = ScriptedStoryBackend::new(
            vec![Ok(Some(published))],
            Err(failed_envelope("aggregate_index_store_error")),
            Ok(generation(
                "aggregate_index_story_synced",
                AggregateIndexStatus::Active,
                clean_current(),
            )),
            Err(failed_envelope("aggregate_index_active_missing")),
        );
        let outcome =
            refresh_for_story("project_story", &backend).expect("不能确认没变 ⇒ 走同步分支");
        assert_eq!(
            outcome,
            StoryIndexRefreshOutcome::Synced {
                aggregate_index_id: "aggregate_index_story_synced".to_string()
            }
        );
        assert_eq!(backend.sync_count(), 1);
        assert_eq!(backend.build_count(), 0);
    }

    // 绿态补充：生产 `impl StoryIndexFreshness` 的直接行为测试（片报告注明）。
    // 13 条红测试用桩钉编排语义；这里钉 from_shared 注入点与 active_missing
    // ⇒ Ok(None) 的映射（后者是生产实现的行为，桩测不到）。
    #[test]
    fn from_shared_story_backend_maps_missing_to_none_and_coerces_to_both_traits() {
        use crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner;
        use crate::product::app_paths::ProductAppPaths;
        use crate::product::logical_codebase::aggregate_index::{
            AggregateIndexFreshnessService, AggregateIndexOperation, CodeGraphCli,
            CodeGraphExcludeGenerator,
        };
        use crate::product::logical_codebase::planning_context_resolver::PlanningIndexFreshness;

        let paths = ProductAppPaths::new(format!("/tmp/aria_story_refresh_{}", Uuid::new_v4()));
        let operation = AggregateIndexOperation::new(
            paths,
            CodeGraphCli::new(Arc::new(TokioBoundedCommandRunner), "codegraph".to_string()),
            CodeGraphExcludeGenerator,
        );
        let service = Arc::new(AggregateIndexFreshnessService::from_shared(Arc::new(
            operation,
        )));
        // P10 注入点：同一个 Arc 双 coerce（禁止两套 operation / 两套 service）。
        let _planning: Arc<dyn PlanningIndexFreshness> = service.clone();
        let story: Arc<dyn StoryIndexFreshness> = service;
        // 没有任何可读世代 ⇒ Ok(None)（分支 1 入口条件），不是 Err。
        assert_eq!(story.readable_generation("project_story").unwrap(), None);
        // 快照采集失败（无 manifest）保持 Err：不能确认没变，由编排层落分支 3。
        let error = story.current_snapshots("project_story").unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_manifest_missing"),
            "unexpected error: {error:?}"
        );
    }
}
