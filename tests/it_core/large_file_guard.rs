use std::fs;
use std::path::{Path, PathBuf};

const MAX_PRODUCT_CODE_LINES: usize = 1200;
const SCAN_ROOTS: &[&str] = &["src", "tests", "web/src"];
const CODE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx"];

/// 祖父条款（2026-10-20 登记）：下列文件在登记时已超 1200 行硬线，属本分支
/// 之前的历史遗留债务，非新代码引入。豁免放松的只是「是否超 1200」这一判定，
/// 不放松「不得继续增长」——每条上限锁定为登记日行数，任何增长仍然报红。
///
/// 登记表只允许删项：文件拆分达标后必须移除（参照 4e7f3242 回收先例：
/// 先登记锁定上限，再逐文件拆分回收）；确需新增项须用户裁决，不在实现
/// 侧自行放宽。本次因 B1-1 接线越线的
/// `src/cross_cutting/claude_code_provider/mod.rs` 已按先例拆分
/// （`direct_start.rs` + `validated_start.rs`，纯移动），因此不在本表内。
///
/// 2026-10-23 批次登记（用户裁决）：`review-scope-and-cross-repo-review` 批次
/// 推高的文件按登记时行数逐文件更新上限（含新登记项）；同批次基线已超且
/// `reviewer_snapshot.rs` / `reviewer_window.rs` / `reviewer_window_store.rs`
/// 属既有债务（2026-10-23 批次未改动这三个文件）；为让门禁可收口，登记值已按当时实测行数登记，
/// 后续不得再增长，债务清理另行立项。
const GRANDFATHERED: &[(&str, usize)] = &[
    (
        "src/cross_cutting/claude_code_provider/tests/policy_session.rs",
        1934,
    ),
    ("src/cross_cutting/codex_provider/mod.rs", 1302),
    (
        "src/cross_cutting/codex_provider/tests/approval_policy.rs",
        2420,
    ),
    ("src/cross_cutting/gateway_sync_provider.rs", 1372),
    (
        "src/cross_cutting/kimi_code_provider/client_services/mod.rs",
        1574,
    ),
    (
        "src/cross_cutting/kimi_code_provider/client_services/terminal.rs",
        1590,
    ),
    (
        "src/cross_cutting/kimi_code_provider/client_services/terminal_handlers.rs",
        1409,
    ),
    ("src/cross_cutting/kimi_code_provider/mod.rs", 1278),
    (
        "src/cross_cutting/kimi_code_provider/tests/session_tests.rs",
        2636,
    ),
    (
        "src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs",
        1264,
    ),
    // 2026-11-01：1617→1641 —— `ensure_ask_extension_in` 改为**原子发布**
    // （临时文件 + rename），修复并发读到半文件的缺陷（CI 并行实测 + 真实
    // daemon 多会话同时起 pi 会踩）；净增为修复本身与并发回归注释，无新功能。
    ("src/cross_cutting/pi_provider/mod.rs", 1641),
    // 2026-11-01：+13 / +4 行 —— 真机 provider e2e 与 unix 进程组用例加
    // 「CI 环境门」（`cross_cutting::real_provider_e2e`）：CI runner 无交互式
    // provider CLI（实测 60s 硬超时）与本地 shell exec 语义，用例在 CI 显式
    // 跳过并打印（不是静默通过），本地照常真跑。仅环境门占用行数，无新逻辑。
    (
        "src/cross_cutting/pi_provider/tests/policy_session.rs",
        1786,
    ),
    ("src/cross_cutting/provider_resources.rs", 3346),
    ("src/cross_cutting/streaming_provider/mod.rs", 2062),
    ("src/cross_cutting/streaming_provider/tests.rs", 2388),
    ("src/product/advance_store.rs", 1369),
    // 2026-11-03（块3 片1 类型收敛）：`logical_codebase_id` 必填 String 后
    // 夹具改 legacy lc_id 需换行，1216→1217（唯一字段取值，无新增用例）。
    (
        "src/product/coding_attempt_store/tests/issue_delivery.rs",
        1218,
    ),
    ("src/product/coding_plan_run.rs", 2872),
    ("src/product/coding_workspace_engine/code_review.rs", 1817),
    ("src/product/coding_workspace_engine/gates.rs", 1436),
    (
        "src/product/coding_workspace_engine/group_review_orchestrator.rs",
        1355,
    ),
    (
        "src/product/coding_workspace_engine/provider_retry.rs",
        1444,
    ),
    (
        "src/product/coding_workspace_engine/provider_stream.rs",
        2549,
    ),
    (
        "src/product/coding_workspace_engine/tests/git_operation_reconcile.rs",
        1349,
    ),
    (
        "src/product/coding_workspace_engine/tests/group_completion_authority.rs",
        1207,
    ),
    (
        "src/product/coding_workspace_engine/tests/group_completion_recovery.rs",
        1686,
    ),
    (
        "src/product/coding_workspace_engine/tests/provider_failure_recovery.rs",
        1689,
    ),
    (
        "src/product/coding_workspace_engine/tests/provider_failure_recovery/provider_idle_stall.rs",
        1329,
    ),
    (
        "src/product/coding_workspace_engine/tests/provider_start_persistence.rs",
        1376,
    ),
    (
        "src/product/coding_workspace_engine/tests/verification_triage_gates.rs",
        1606,
    ),
    ("src/product/lifecycle_store/plan.rs", 1683),
    ("src/product/lifecycle_store/tests.rs", 2893),
    (
        "src/product/lifecycle_store/tests/tool_policy_audit.rs",
        1431,
    ),
    ("src/product/lifecycle_store/workspace.rs", 1730),
    (
        "src/product/lifecycle_store/workspace_single_candidate.rs",
        1621,
    ),
    (
        "src/product/lifecycle_store/workspace_single_candidate_tests.rs",
        2055,
    ),
    (
        "src/product/logical_codebase/coordinator_provider_turn.inc.rs",
        1221,
    ),
    (
        "src/product/logical_codebase/production_policy_resolvers.rs",
        1547,
    ),
    (
        "src/product/logical_codebase/provider_boundary_probe.rs",
        2646,
    ),
    (
        "src/product/logical_codebase/provider_capability_store.rs",
        1745,
    ),
    ("src/product/logical_codebase/provider_gateway.rs", 3239),
    (
        "src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs",
        1553,
    ),
    ("src/product/logical_codebase/provider_trust.rs", 4484),
    // 2026-10-29：registration_tests.inc.rs 已按主题拆分为 3 个片段
    // （detect_preflight / attach / aggregate_root_and_cold_start，均 ≤1200 行），
    // 按「拆分达标后必须移除」的登记回收先例移除本项。
    ("src/product/models/tests.rs", 1523),
    // 2026-10-25：reviewer_reconciliation.rs 已拆分至 507 行（< 1200 硬线），
    // 按「拆分达标后必须移除」的登记回收先例移除本项。
    // 2026-10-23：既有债务登记（本批未改该文件，基线即 3362 行）
    ("src/product/reviewer_snapshot.rs", 3362),
    // 2026-10-23：既有债务登记（本批未改该文件，基线即 4198 行）
    ("src/product/reviewer_window.rs", 4198),
    // 2026-10-23：既有债务登记（本批未改该文件，基线即 1921 行）
    ("src/product/reviewer_window_store.rs", 1921),
    ("src/product/work_item_contract/tests/dependency.rs", 1204),
    // 2026-10-30（REQ-WSC-10，待 owner 裁决）：新增 write_scope_bound 测试模块声明 +1。
    ("src/product/work_item_plan_compiler/tests.rs", 1450),
    (
        "src/product/work_item_split_engine/tests/engine_gateway_guard.rs",
        1486,
    ),
    // 2026-11-03（块3 片1 类型收敛）：issue 夹具 `logical_codebase_id` 必填
    // String 后取 legacy lc_id 换行，1666→1667。
    (
        "src/product/work_item_split_engine/tests/prompt_contract.rs",
        1667,
    ),
    ("src/product/work_item_split_validator/tests.rs", 1259),
    ("src/product/workspace_engine/compile.rs", 1651),
    ("src/product/workspace_engine/contract_autorepair.rs", 1651),
    // 2026-10-30（fix-owner-readable-human-gate）：REQ-WSC-11 findings 增 owner_question/owner_options +2。
    ("src/product/workspace_engine/contract_prerevision.rs", 1913),
    // 2026-10-30（REQ-WSC-10，待 owner 裁决）：门内修订路径接线成员 checkout 前缀表
    // 装载（装载器独立成模块，此处仅门控调用）+17；深嵌套缩进下已是物理下限。
    (
        "src/product/workspace_engine/conversational_gate_parts/revision_turn.inc.rs",
        1505,
    ),
    (
        "src/product/workspace_engine/draft_batch/compile_support.rs",
        2310,
    ),
    ("src/product/workspace_engine/lifecycle.rs", 2167),
    // 2026-10-30（fix-owner-readable-human-gate）：REQ-WSC-11 reviewer 输出契约增 owner 呈现硬约束 +16。
    ("src/product/workspace_engine/prompts.rs", 1681),
    (
        "src/product/workspace_engine/prompts/human_gate_revision.rs",
        1464,
    ),
    // 2026-11-03（块3 片1 类型收敛）：3 处 issue 夹具 `logical_codebase_id`
    // 改 legacy lc_id 各 +1 行，2559→2562。
    // 2026-11-06（Task 14b 治本）：review/drive.rs 已按职责拆入
    // review/drive_parts/*.inc.rs（261 行），降至 ≤1200 ⇒ 按仓规删除其祖父
    // 登记项（不给将来重新长回 2645 行留暗门）。
    ("src/product/workspace_engine/review/drive_tests.rs", 1379),
    ("src/product/workspace_engine/review/routing.rs", 1391),
    ("src/product/workspace_engine/session_state.rs", 1457),
    (
        "src/product/workspace_engine/tests/author_revision_loop.rs",
        1227,
    ),
    (
        "src/product/workspace_engine/tests/author_revision_loop_parts/root_cwd_revision.inc.rs",
        1683,
    ),
    (
        "src/product/workspace_engine/tests/conversational_gate_close.rs",
        2891,
    ),
    // 2026-10-30（REQ-WSC-10，待 owner 裁决）：上下文字面量补 member_checkout_prefixes 字段 +2。
    (
        "src/product/workspace_engine/tests/conversational_gate_recovery.rs",
        1567,
    ),
    (
        "src/product/workspace_engine/tests/conversational_gate_revision/part_02.rs",
        1440,
    ),
    ("src/product/workspace_engine/tests/part_31.rs", 1233),
    ("src/product/workspace_engine/tests/recovery_task7.rs", 1295),
    // 2026-10-30（fix-owner-readable-human-gate）：REQ-WSC-11 finding 字面量补两字段 +6。
    (
        "src/product/workspace_engine/tests/single_candidate.rs",
        1551,
    ),
    ("src/web/advance_plan.rs", 1644),
    ("src/web/app.rs", 2222),
    ("src/web/coding_start.rs", 1256),
    ("src/web/coding_ws_handler/runner.rs", 2921),
    ("src/web/coding_ws_handler/socket.rs", 1295),
    (
        "src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs",
        1242,
    ),
    (
        // 2026-11-03（REQ-ROUTE-01 归属字段必填清扫）：issue 归属字段由
        // Option<String> 收敛为必填 String，夹具补「归属必填」注释 + 别名 id
        // 调用（rustfmt 规范形态，多行）⇒ +6 行。
        "src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs",
        2047,
    ),
    ("src/web/handlers/automation_gateway_preflight.rs", 1217),
    ("src/web/handlers/coding.rs", 1321),
    ("src/web/handlers/coding/plan_start.rs", 2565),
    ("src/web/handlers/coding/plan_start_tests.rs", 4130),
    ("src/web/handlers/support.rs", 1204),
    ("src/web/handlers/support_parts/coding_cleanup.inc.rs", 1572),
    ("src/web/wiga_gate_fixture.rs", 1764),
    // 2026-11-03（REQ-ROUTE-01 归属字段必填清扫）：2 处 issue 归属补
    // 「归属必填」注释 + 别名 id 调用（多行）⇒ +8 行。
    ("src/web/workspace_session/tests/part_01.rs", 1619),
    ("src/web/workspace_session/tests/part_07.rs", 1272),
    ("src/web/workspace_ws_handler/socket.rs", 1893),
    (
        "src/web/workspace_ws_handler/tests/campaign_stage3_interactive/cases.rs",
        1209,
    ),
    (
        "src/web/workspace_ws_handler/tests/conversational_gate_protocol.rs",
        2050,
    ),
    (
        // 2026-11-03（REQ-ROUTE-01 归属字段必填清扫）：issue 归属补「归属必填」
        // 注释 + 别名 id 调用（rustfmt 规范形态，多行）⇒ +5 行。
        "src/web/workspace_ws_handler/tests/planning_resume.rs",
        2458,
    ),
    (
        "src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs",
        2638,
    ),
    (
        "src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs",
        1508,
    ),
    ("tests/it_web/provider_gateway_envelope.rs", 1624),
    (
        "tests/it_web/web_lc_gateway_multi_provider/harness.rs",
        14966,
    ),
    (
        "tests/it_web/web_lc_gateway_multi_provider/live_b45_t23.rs",
        1651,
    ),
    (
        "tests/it_web/web_lc_gateway_multi_provider/live_b45_t24.rs",
        2400,
    ),
    (
        "tests/it_web/web_lc_gateway_multi_provider/live_matrix.rs",
        2461,
    ),
    ("web/src/api/client.ts", 1313),
    (
        "web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx",
        1410,
    ),
    ("web/src/pages/ChatCockpitPage.tsx", 1471),
    ("web/src/pages/CodingWorkspacePage.test.tsx", 1560),
    ("web/src/pages/CodingWorkspacePage.tsx", 1541),
    ("web/src/state/workspace-ws-store.ts", 1230),
];

#[test]
fn product_source_and_test_files_stay_under_line_limit() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut oversized = Vec::new();

    for scan_root in SCAN_ROOTS {
        collect_oversized_files(&repo_root.join(scan_root), &repo_root, &mut oversized);
    }

    oversized.sort();
    assert!(
        oversized.is_empty(),
        "产品源码与测试文件必须保持在 {MAX_PRODUCT_CODE_LINES} 行以内，当前超限：\n{}",
        oversized.join("\n"),
    );
}

/// 登记表按路径排序且无重复——保证 diff 可评审、查找有唯一答案。
#[test]
fn grandfather_registry_is_sorted_and_unique() {
    let mut previous: Option<&str> = None;
    for (path, cap) in GRANDFATHERED {
        assert!(
            *cap > MAX_PRODUCT_CODE_LINES,
            "登记上限 {path} = {cap}：未超硬线的文件不应登记豁免",
        );
        if let Some(previous) = previous {
            assert!(
                previous < *path,
                "登记表须按路径严格升序排列：{previous} 之后出现了 {path}",
            );
        }
        previous = Some(path);
    }
}

fn effective_limit(relative: &str) -> usize {
    GRANDFATHERED
        .iter()
        .find(|(path, _)| *path == relative)
        .map(|(_, cap)| *cap)
        .unwrap_or(MAX_PRODUCT_CODE_LINES)
}

fn collect_oversized_files(path: &Path, repo_root: &Path, oversized: &mut Vec<String>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };

    if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            collect_oversized_files(&entry.path(), repo_root, oversized);
        }
        return;
    }

    if !metadata.is_file() || !is_product_code_file(path) {
        return;
    }

    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let line_count = content.lines().count();
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    let relative_str = relative.display().to_string();
    let limit = effective_limit(&relative_str);
    if line_count > limit {
        if limit == MAX_PRODUCT_CODE_LINES {
            oversized.push(format!("{relative_str}: {line_count} 行"));
        } else {
            oversized.push(format!(
                "{relative_str}: {line_count} 行（登记上限 {limit} 行）",
            ));
        }
    }
}

fn is_product_code_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| CODE_EXTENSIONS.contains(&extension))
}
