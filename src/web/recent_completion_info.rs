//! P3 WIGA（tasks.md §4.1 / REQ-WIGA-07）：issue 级有界近期 durable 完成
//! 目录投影。
//!
//! 不建第二事实源——只归一化既有 `plan_confirmed_info` /
//! `coding_final_confirm_info` 两个 durable 投影：服务端最近 24h 窗口
//! （以请求时钟过滤未来发生时间）、同 scope/`kind`/`key` 只留当前
//! durable 投影（waiting→Completed 同 key 优先当前 Completed）、按
//! `(occurred_at DESC, kind DESC, key DESC)` 稳定排序、每 issue 至多
//! `limit` 条（跨 issue 从不共用配额）。归一化时再次匹配 coding DTO
//! 的 `project_id`/`issue_id`，跨 issue 事实不得混入；坏 durable 时间
//! fail-closed 显式上抛 `ProductStoreError`，不投射假近期成功。

use crate::product::json_store::ProductStoreError;
use crate::web::coding_final_confirm_info::CodingFinalConfirmInfoDto;
use crate::web::plan_confirmed_info::PlanConfirmedInfoDto;
use std::collections::BTreeMap;

/// 服务端近期窗口：完成事实最多回看 24 小时（与前端
/// `RECENT_COMPLETION_WINDOW_MS` 同源取值）。
pub const RECENT_COMPLETION_WINDOW_HOURS: i64 = 24;
/// 每 issue 有界响应的默认与最大条数（query 校验在 handler 层）。
pub const RECENT_COMPLETION_MAX_LIMIT: usize = 32;

/// 完成事实种类：与既有两个 durable 投影一一对应（JSON snake_case）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecentCompletionKind {
    PlanConfirmed,
    CodingFinalConfirm,
}

/// 有界近期完成目录条目（`IssueLifecycleResponse.recent_completion_info`）。
/// plan：`session_id=Some`、`attempt_id`/`final_confirmed=None`；coding：
/// `session_id=None`、`attempt_id=Some`、`final_confirmed=Some`。`None`
/// 在 JSON 输出 null，字段均完整输出。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentCompletionInfoDto {
    pub kind: RecentCompletionKind,
    pub key: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub session_id: Option<String>,
    pub attempt_id: Option<String>,
    pub occurred_at: String,
    pub title: String,
    pub final_confirmed: Option<bool>,
}

/// 归一化两个既有 durable 投影为有界近期完成目录：窗口过滤（
/// `since <= occurred_at <= now`）→ 稳定排序 → 取最近 `limit` 条。
/// 坏 durable 时间 fail-closed 上抛；跨 project/issue 的 coding 事实
/// 直接跳过，不混入本 scope。
pub fn recent_completion_info(
    project_id: &str,
    issue_id: &str,
    plan: &[PlanConfirmedInfoDto],
    coding: &[CodingFinalConfirmInfoDto],
    since: chrono::DateTime<chrono::Utc>,
    limit: usize,
) -> Result<Vec<RecentCompletionInfoDto>, ProductStoreError> {
    // 请求时钟：未来发生时间（时钟漂移/坏数据）不进目录。
    let now = chrono::Utc::now();

    // 归一化：同 (kind, key) 只留当前 durable 投影；coding DTO 再次匹配
    // scope，跨 project/issue 事实不混入。
    let mut by_key: BTreeMap<
        (u8, String),
        (chrono::DateTime<chrono::Utc>, RecentCompletionInfoDto),
    > = BTreeMap::new();
    for info in plan {
        let occurred = parse_occurred_at("plan_confirmed", &info.key, &info.occurred_at)?;
        let dto = RecentCompletionInfoDto {
            kind: RecentCompletionKind::PlanConfirmed,
            key: info.key.clone(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: info.plan_id.clone(),
            session_id: Some(info.session_id.clone()),
            attempt_id: None,
            occurred_at: info.occurred_at.clone(),
            title: info.title.clone(),
            final_confirmed: None,
        };
        merge_current_projection(&mut by_key, occurred, dto);
    }
    for info in coding {
        if info.project_id != project_id || info.issue_id != issue_id {
            continue;
        }
        let occurred = parse_occurred_at("coding_final_confirm", &info.key, &info.occurred_at)?;
        let dto = RecentCompletionInfoDto {
            kind: RecentCompletionKind::CodingFinalConfirm,
            key: info.key.clone(),
            project_id: info.project_id.clone(),
            issue_id: info.issue_id.clone(),
            plan_id: info.plan_id.clone(),
            session_id: None,
            attempt_id: Some(info.attempt_id.clone()),
            occurred_at: info.occurred_at.clone(),
            title: info.title.clone(),
            final_confirmed: Some(info.final_confirmed),
        };
        merge_current_projection(&mut by_key, occurred, dto);
    }

    // 先过滤窗口（since <= occurred_at <= now），再按
    // (occurred_at DESC, kind DESC, key DESC) 稳定排序取最近 limit 条。
    let mut visible: Vec<(chrono::DateTime<chrono::Utc>, RecentCompletionInfoDto)> =
        by_key.into_values().collect();
    visible.retain(|(occurred, _)| *occurred >= since && *occurred <= now);
    visible.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(kind_rank(&b.1.kind).cmp(&kind_rank(&a.1.kind)))
            .then(b.1.key.cmp(&a.1.key))
    });
    Ok(visible
        .into_iter()
        .take(limit)
        .map(|(_, dto)| dto)
        .collect())
}

/// 同 (kind, key) 并存多投影时只留当前 durable 投影：等待→已确认并存
/// 优先当前 Completed（`final_confirmed=Some(true)`），其余以后写入为准。
fn merge_current_projection(
    by_key: &mut BTreeMap<(u8, String), (chrono::DateTime<chrono::Utc>, RecentCompletionInfoDto)>,
    occurred: chrono::DateTime<chrono::Utc>,
    dto: RecentCompletionInfoDto,
) {
    let id = (kind_rank(&dto.kind), dto.key.clone());
    match by_key.get_mut(&id) {
        Some(entry) => {
            if entry.1.final_confirmed != Some(true) || dto.final_confirmed == Some(true) {
                *entry = (occurred, dto);
            }
        }
        None => {
            by_key.insert(id, (occurred, dto));
        }
    }
}

/// 排序用种类秩（DESC：coding > plan）；不公开到 DTO 派生。
fn kind_rank(kind: &RecentCompletionKind) -> u8 {
    match kind {
        RecentCompletionKind::PlanConfirmed => 0,
        RecentCompletionKind::CodingFinalConfirm => 1,
    }
}

/// durable 发生时间解析：RFC3339 → UTC；坏时间 fail-closed 显式上抛，
/// 不静默跳过其余事实假称完整。
fn parse_occurred_at(
    kind: &str,
    key: &str,
    raw: &str,
) -> Result<chrono::DateTime<chrono::Utc>, ProductStoreError> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|ts| ts.with_timezone(&chrono::Utc))
        .map_err(|error| {
            ProductStoreError::Io(format!(
                "recent completion occurred_at is not RFC3339: kind={kind} key={key} raw={raw}: {error}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn plan_dto(key: &str, plan_id: &str, occurred_at: &str) -> PlanConfirmedInfoDto {
        PlanConfirmedInfoDto {
            key: key.to_string(),
            plan_id: plan_id.to_string(),
            session_id: format!("session-{plan_id}"),
            occurred_at: occurred_at.to_string(),
            title: crate::web::plan_confirmed_info::PLAN_CONFIRMED_INFO_TITLE.to_string(),
        }
    }

    fn coding_dto(
        key: &str,
        attempt_id: &str,
        occurred_at: &str,
        final_confirmed: bool,
    ) -> CodingFinalConfirmInfoDto {
        CodingFinalConfirmInfoDto {
            key: key.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "plan_1".to_string(),
            attempt_id: attempt_id.to_string(),
            occurred_at: occurred_at.to_string(),
            title: if final_confirmed {
                crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_CONFIRMED_TITLE
                    .to_string()
            } else {
                crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_WAITING_TITLE
                    .to_string()
            },
            final_confirmed,
        }
    }

    fn ago(hours: i64) -> String {
        (Utc::now() - Duration::hours(hours)).to_rfc3339()
    }

    /// 主证据：`occurred_at DESC` 为主序，同刻以 `kind DESC, key DESC`
    /// 稳定择优；limit 截取最新事实；两源字段映射互为对偶。
    #[test]
    fn recent_completion_orders_by_occurred_at_kind_key_and_limits() {
        let now = Utc::now();
        let plan = vec![
            plan_dto("plan_confirmed:plan-old:c1", "plan-old", &ago(4)),
            plan_dto("plan_confirmed:plan-new:c2", "plan-new", &ago(2)),
        ];
        let coding = vec![
            coding_dto("coding_final_confirm:att-b:node-b", "att-b", &ago(3), false),
            coding_dto("coding_final_confirm:att-a:node-a", "att-a", &ago(1), false),
        ];
        let all = recent_completion_info(
            "project_0001",
            "issue_0001",
            &plan,
            &coding,
            now - Duration::hours(24),
            RECENT_COMPLETION_MAX_LIMIT,
        )
        .unwrap();
        let keys: Vec<&str> = all.iter().map(|dto| dto.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "coding_final_confirm:att-a:node-a",
                "plan_confirmed:plan-new:c2",
                "coding_final_confirm:att-b:node-b",
                "plan_confirmed:plan-old:c1",
            ],
            "occurred_at DESC 为主序"
        );
        // limit 截取最新两条。
        let bounded = recent_completion_info(
            "project_0001",
            "issue_0001",
            &plan,
            &coding,
            now - Duration::hours(24),
            2,
        )
        .unwrap();
        assert_eq!(bounded.len(), 2);
        assert_eq!(bounded[0].key, "coding_final_confirm:att-a:node-a");
        assert_eq!(bounded[1].key, "plan_confirmed:plan-new:c2");
        // 字段映射：plan 有 session/无 attempt/无 final_confirmed；coding 对偶。
        assert_eq!(bounded[0].session_id, None);
        assert_eq!(bounded[0].attempt_id.as_deref(), Some("att-a"));
        assert_eq!(bounded[0].final_confirmed, Some(false));
        assert_eq!(bounded[1].session_id.as_deref(), Some("session-plan-new"));
        assert_eq!(bounded[1].attempt_id, None);
        assert_eq!(bounded[1].final_confirmed, None);
        for dto in &all {
            assert_eq!(dto.project_id, "project_0001");
            assert_eq!(dto.issue_id, "issue_0001");
        }
        // 同刻择优：kind DESC（coding 先于 plan）+ key DESC。
        let same_instant = ago(2);
        let tied = recent_completion_info(
            "project_0001",
            "issue_0001",
            &[
                plan_dto("plan_confirmed:pzz:c", "pzz", &same_instant),
                plan_dto("plan_confirmed:paa:c", "paa", &same_instant),
            ],
            &[
                coding_dto(
                    "coding_final_confirm:att-z:node",
                    "att-z",
                    &same_instant,
                    false,
                ),
                coding_dto(
                    "coding_final_confirm:att-a:node",
                    "att-a",
                    &same_instant,
                    false,
                ),
            ],
            now - Duration::hours(24),
            RECENT_COMPLETION_MAX_LIMIT,
        )
        .unwrap();
        let keys: Vec<&str> = tied.iter().map(|dto| dto.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "coding_final_confirm:att-z:node",
                "coding_final_confirm:att-a:node",
                "plan_confirmed:pzz:c",
                "plan_confirmed:paa:c",
            ],
            "同刻以 (kind DESC, key DESC) 稳定择优"
        );
    }

    /// 窗口：`since` 之前/未来发生时间不返回；`occurred_at == since` 边界
    /// 保留；RFC3339 时区偏移不同写法按同一瞬时比较。
    #[test]
    fn recent_completion_filters_window_and_future_occurrences() {
        let now = Utc::now();
        let in_window = (now - Duration::hours(1)).to_rfc3339();
        let boundary = (now - Duration::hours(2)).to_rfc3339();
        let future = (now + Duration::hours(1)).to_rfc3339();
        // 同一瞬时、+08:00 写法：窗口判定不受时区偏移影响。
        let offset_written = now
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .to_rfc3339();
        let plan = vec![plan_dto(
            "plan_confirmed:p-boundary:c",
            "p-boundary",
            &boundary,
        )];
        let coding = vec![
            coding_dto(
                "coding_final_confirm:att-in:node",
                "att-in",
                &in_window,
                false,
            ),
            coding_dto(
                "coding_final_confirm:att-future:node",
                "att-future",
                &future,
                false,
            ),
            coding_dto(
                "coding_final_confirm:att-tz:node",
                "att-tz",
                &offset_written,
                false,
            ),
        ];
        let since_boundary = now - Duration::hours(2);
        let visible = recent_completion_info(
            "project_0001",
            "issue_0001",
            &plan,
            &coding,
            since_boundary,
            RECENT_COMPLETION_MAX_LIMIT,
        )
        .unwrap();
        let keys: Vec<&str> = visible.iter().map(|dto| dto.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "coding_final_confirm:att-tz:node",
                "coding_final_confirm:att-in:node",
                "plan_confirmed:p-boundary:c",
            ],
            "边界 == since 保留；未来与 +08:00 未来瞬时（now 即界）不返回"
        );
        // since 收紧到 now-1h：边界条目出窗。
        let tightened = recent_completion_info(
            "project_0001",
            "issue_0001",
            &plan,
            &coding,
            now - Duration::hours(1),
            RECENT_COMPLETION_MAX_LIMIT,
        )
        .unwrap();
        let keys: Vec<&str> = tightened.iter().map(|dto| dto.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "coding_final_confirm:att-tz:node",
                "coding_final_confirm:att-in:node"
            ]
        );
    }

    /// 去重与授权：同 (kind, key) 只留当前 durable 投影（waiting→Completed
    /// 优先 Completed）；coding DTO 的 project/issue 不匹配本 scope 时不混入。
    #[test]
    fn recent_completion_dedups_same_key_and_rejects_foreign_scope() {
        let now = Utc::now();
        let waiting = coding_dto("coding_final_confirm:att-x:node", "att-x", &ago(1), false);
        let completed = coding_dto("coding_final_confirm:att-x:node", "att-x", &ago(1), true);
        let mut foreign_issue =
            coding_dto("coding_final_confirm:att-y:node", "att-y", &ago(1), false);
        foreign_issue.issue_id = "issue_9999".to_string();
        let mut foreign_project =
            coding_dto("coding_final_confirm:att-z:node", "att-z", &ago(1), false);
        foreign_project.project_id = "project_9999".to_string();
        let merged = recent_completion_info(
            "project_0001",
            "issue_0001",
            &[],
            &[waiting, completed, foreign_issue, foreign_project],
            now - Duration::hours(24),
            RECENT_COMPLETION_MAX_LIMIT,
        )
        .unwrap();
        assert_eq!(merged.len(), 1, "同 key 去重 + 跨 scope 跳过: {merged:?}");
        assert_eq!(merged[0].final_confirmed, Some(true));
        assert_eq!(
            merged[0].title,
            crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_CONFIRMED_TITLE
        );
        // 同 key plan 双投影：保留后写入的当前投影。
        let first = plan_dto("plan_confirmed:p-dup:c", "p-dup", &ago(1));
        let mut second = first.clone();
        second.title = "当前投影".to_string();
        let merged = recent_completion_info(
            "project_0001",
            "issue_0001",
            &[first, second],
            &[],
            now - Duration::hours(24),
            RECENT_COMPLETION_MAX_LIMIT,
        )
        .unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "当前投影");
    }

    /// 坏 durable 时间 fail-closed：不可解析的 occurred_at 显式上抛
    /// `ProductStoreError`，不静默跳过其余事实假称完整。
    #[test]
    fn recent_completion_fails_closed_on_bad_durable_time() {
        let now = Utc::now();
        let bad_plan = plan_dto("plan_confirmed:p-bad:c", "p-bad", "not-a-time");
        let error = recent_completion_info("project_0001", "issue_0001", &[bad_plan], &[], now, 32)
            .unwrap_err();
        assert!(
            matches!(&error, ProductStoreError::Io(message) if message.contains("occurred_at")),
            "plan 坏时间必须显式失败: {error:?}"
        );
        let bad_coding = coding_dto(
            "coding_final_confirm:att-bad:node",
            "att-bad",
            "2026-13-99T99:99:99Z",
            false,
        );
        let error =
            recent_completion_info("project_0001", "issue_0001", &[], &[bad_coding], now, 32)
                .unwrap_err();
        assert!(
            matches!(&error, ProductStoreError::Io(message) if message.contains("occurred_at")),
            "coding 坏时间必须显式失败: {error:?}"
        );
    }
}
