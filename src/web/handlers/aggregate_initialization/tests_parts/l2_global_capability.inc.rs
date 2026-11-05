// -------------------------------------------------------------------
// REQ-BOOT-14（owner 2026-11-04）：**全局核验一次 ⇒ 聚合初始化自动开始**。
//
// owner 裁定：provider 能力核验从 per-LC 改为**项目级全局**——能力是本机
// provider CLI 的事实，与逻辑代码库的代码内容无关 ⇒ 一次核验全项目可用。
//
// ⇒ 用户面收益：**聚合根不需要再放 `AGENTS.md` + `.claude/rules/language.md`**
// 这两个判定文件来解锁「聚合初始化自动开始」——四家 provider 在项目级全局
// 核验一次之后，冷启动推进到聚合初始化时 provider 选择门（L2）即放行 ⇒
// operation 被创建并开始。
//
// 本片段覆盖：
// - 4.2：聚合根**无**两个判定文件 + 四家全局能力已 `Confirmed` ⇒ 冷启动
//   推进到聚合初始化 ⇒ operation 被创建并开始执行；
// - 4.3：聚合根**无**两个判定文件 + 全局记录中无任何候选为 `Confirmed` ⇒
//   **不创建 operation**，返回既有「需要核验」失败码与可点击核验入口；
// - 4.4（回归）：聚合根**有**两个判定文件 ⇒ 继续走「零 provider turn」路径。
//
// 判定门本身的强度不变：L1（命令可用）与 L2（能力 `Confirmed`）两层判定
// 一字未改，`Denied`/`Unknown` 一律不放行（见 selection_failure.inc.rs 的
// Denied 回归用例）。
// -------------------------------------------------------------------

/// 把 provider 能力种为**机器级** Confirmed 行（一次核验整机可用的形态）。
/// 落盘路径 = `provider-capabilities/capabilities.json`（REQ-PCR-07）。
///
/// 只种 ClaudeCode：root-recipe 相位已对 provider 无关（REQ-BOOT-15，Claude-only
/// 硬拒已删除），但本夹具要验的是「**有一家已核验 ⇒ L2 放行 ⇒ 聚合初始化自动
/// 开始**」这条链，只需种一家 Confirmed 即可。其余三家保持「未核验」⇒ 选择器
/// 按 owner 顺序跳过它们、选中 ClaudeCode（既有语义：第一个命令可用且已核验的候选）。
/// ⚠ 不要按「recipe 相位只认 Claude」来理解这条种子——那是被删除的旧契约。
fn seed_claude_code_confirmed_globally(fixture: &TrustRouteFixture) {
    crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
        &fixture.paths,
        "project_0001",
        &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
    );
}

/// 断言 capability 记录确实落在**机器级**、且项目/逻辑代码库子树下没有它
/// （REQ-PCR-07「唯一作用域」：出现项目级或 LC 子树副本即为第二作用域 = 缺陷）。
fn assert_capability_scope_is_machine_global(fixture: &TrustRouteFixture) {
    let global_path = fixture
        .paths
        .provider_capabilities_root()
        .join("capabilities.json");
    assert!(
        global_path.is_file(),
        "the capability record must live in the machine-level file: {}",
        global_path.display()
    );
    let project_path = fixture
        .paths
        .logical_codebase_root("project_0001")
        .join("capabilities.json");
    assert!(
        !project_path.exists(),
        "no capability record may exist under a project subtree: {}",
        project_path.display()
    );
    let lc_path = fixture
        .paths
        .logical_codebases_root("project_0001")
        .join(&fixture.lc_id)
        .join("capabilities.json");
    assert!(
        !lc_path.exists(),
        "no capability record may exist under a logical codebase subtree: {}",
        lc_path.display()
    );
}

/// REQ-BOOT-14 场景 1（owner 2026-11-04 主线解法）：聚合根**没有**两个判定
/// 文件 + provider 在**项目级全局**已核验 ⇒ 冷启动推进到聚合初始化 ⇒
/// operation 被自动创建并开始执行（不再是
/// `aggregate_initialization_provider_needs_verification`）。
///
/// 这条用例是「用户不再需要往聚合根放 AGENTS.md + .claude/rules/language.md
/// 才能让聚合初始化自动开始」的**唯一落点**。
#[tokio::test]
async fn advance_auto_starts_after_one_global_capability_verification() {
    // 夹具：L1 health 四家全可用，但**不种**任何已核验能力（先确认起点）。
    let fixture = trust_route_fixture_unverified_providers();
    // 聚合根**没有**两个判定文件（本用例的前提：不需要它们）。
    assert!(
        !fixture
            .root
            .join("aggregate-root")
            .join("AGENTS.md")
            .exists(),
        "the aggregate root must not carry the AGENTS.md marker in this scenario"
    );
    assert!(
        !fixture
            .root
            .join("aggregate-root")
            .join(".claude")
            .join("rules")
            .join("language.md")
            .exists(),
        "the aggregate root must not carry the language rule marker in this scenario"
    );
    // 用户执行一次「一键核验全部」⇒ provider 在机器级被核验。
    seed_claude_code_confirmed_globally(&fixture);
    // 记录落在机器级全局、项目与 LC 子树下都没有副本（唯一作用域）。
    assert_capability_scope_is_machine_global(&fixture);
    advance_seed_member(&fixture);

    let (status, body) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        advanced_step_names(&body).contains(&"rules_policy".to_string()),
        "the global capability verification must let the root recipe start: {body}"
    );

    // operation 被创建并开始执行（不是「需要核验」失败面）。
    let operations = root_policy_operations(&fixture);
    let operation_id = sole_operation_id(&fixture);
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "the root recipe must run to completion after one global verification: {:?}",
        operation.error
    );
    assert!(
        !operation.full_recipe_requested,
        "the advance path must not force the full recipe"
    );
    // provider 选择门放行 ⇒ 冻结了 provider 与候选表（不是「不需要 provider」
    // 的 not_required）。
    let selection = operation
        .provider_selection
        .as_ref()
        .expect("a selected provider must freeze its candidate table");
    assert_eq!(selection.candidates.len(), 3, "{selection:?}");
    assert_eq!(
        selection.selected, "claude_code",
        "the recipe phase requires the fixed claude provider: {selection:?}"
    );
    assert_eq!(operation.input.provider, "claude_code", "{body}");
}

/// REQ-BOOT-14 场景 2：全局记录中无任何候选为 `Confirmed` ⇒ **不创建
/// operation**，返回既有「需要核验」失败码与可点击核验入口（**不得**自动
/// 发起核验）。
///
/// 这是 REQ-BOOT-07 的既有语义（判定门强度不变），本次全局化只改「读哪份
/// 记录」，不改「读到什么就不放行」。
#[tokio::test]
async fn advance_still_needs_verification_when_the_global_record_is_unverified() {
    // 夹具：L1 health 四家全可用，且**不种**任何已核验能力。
    let fixture = trust_route_fixture_unverified_providers();
    advance_seed_member(&fixture);

    let (status, body) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 稳定失败码逐字不变。
    assert_eq!(
        body["stopped_reason_code"],
        "aggregate_initialization_provider_needs_verification",
        "{body}"
    );
    // 不创建 operation（不绕门）。
    assert!(
        root_policy_operations(&fixture)
            .list("project_0001")
            .expect("operation list")
            .is_empty(),
        "an unverified global record must keep the five-step operation uncreated: {body}"
    );
    // 零 provider turn（不得自动发起核验、不得启动任何 provider）。
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        0,
        "a rejected create chain must not start any provider turn"
    );
}

/// REQ-BOOT-14 场景 3（回归）：聚合根**有**两个判定文件 ⇒ 继续走「整条链零
/// provider turn」路径（免过 provider 选择门、`provider_selection` 为空）。
///
/// 该路径 MUST NOT 被本次全局化取消或改写——它是**另一条**独立路径。
#[tokio::test]
async fn advance_keeps_the_zero_provider_turn_path_when_the_markers_exist() {
    // 夹具：**不种**任何已核验能力（若零 provider turn 路径被取消，这里必然
    // 被 provider 选择门拦下 ⇒ 断言会红）。
    let fixture = trust_route_fixture_unverified_providers();
    preinitialize_aggregate_root(&fixture);
    advance_seed_member(&fixture);

    let (status, body) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let operations = root_policy_operations(&fixture);
    let operation_id = sole_operation_id(&fixture);
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "the zero-provider path must still complete: {:?}",
        operation.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        0,
        "the whole chain must run zero provider turns (zero quota, zero probe)"
    );
    // 免过 provider 选择门 ⇒ 没有候选表（没有候选）。
    assert!(
        operation.provider_selection.is_none(),
        "a chain without providers must not freeze a provider candidate table"
    );
    assert_eq!(
        operation.input.provider,
        crate::product::logical_codebase::AGGREGATE_PROVIDER_NOT_REQUIRED,
        "the durable record must state that this chain needs no provider"
    );
}
