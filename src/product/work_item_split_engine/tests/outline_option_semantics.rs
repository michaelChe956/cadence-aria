// Outline prompts must explain what the enabled split/test option flags mean.
// Regression: Pi classified a pure library outline as kind=other because the
// prompt only passed `force_frontend_backend_split: true` as a bare flag; the
// strict Final Compile validator then rejected the plan with
// `frontend_backend_split_required`. Both the initial outline prompt and the
// incremental revision prompt must spell out the required kind composition.

use crate::product::work_item_split_engine::prompts::build_outline_prompt_with_nonce;

#[test]
fn initial_outline_prompt_spells_out_enabled_split_option_semantics() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.force_frontend_backend_split = Some(true);
    request.include_integration_tests = Some(true);
    request.include_e2e_tests = Some(true);

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
    );

    assert!(
        prompt.contains("[user_option_semantics]"),
        "outline prompt must carry an option semantics section when flags are enabled"
    );
    for required in [
        "force_frontend_backend_split=true：本次拆分必须产出至少一个 kind=backend 和至少一个 kind=frontend 的 outline",
        "被拆离页面/演示的纯库函数、共享实现、核心逻辑归 backend；页面、UI、演示内容归 frontend",
        "为满足该前后端拆分要求而产出的 backend/frontend 两方均不得标为 other",
        "include_integration_tests=true：必须产出至少一个 kind=integration 的 outline",
        "include_e2e_tests=true：必须产出至少一个 kind=e2e 的 outline",
    ] {
        assert!(prompt.contains(required), "missing option semantics: {required}");
    }
}

#[test]
fn initial_outline_prompt_omits_semantics_when_options_disabled() {
    let (request, issue, repository) = split_prompt_fixture();

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
    );

    assert!(
        !prompt.contains("[user_option_semantics]"),
        "disabled options must not inject a semantics section"
    );
}

#[test]
fn initial_outline_prompt_only_lists_enabled_flags() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.force_frontend_backend_split = Some(true);

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
    );

    assert!(prompt.contains("force_frontend_backend_split=true：本次拆分必须产出至少一个 kind=backend"));
    assert!(
        !prompt.contains("include_integration_tests=true："),
        "disabled integration flag must not render its rule"
    );
    assert!(
        !prompt.contains("include_e2e_tests=true："),
        "disabled e2e flag must not render its rule"
    );
}

#[test]
fn outline_revision_prompt_spells_out_enabled_split_option_semantics() {
    let (mut request, issue, _repository) = split_prompt_fixture();
    request.force_frontend_backend_split = Some(true);
    request.include_integration_tests = Some(true);
    request.include_e2e_tests = Some(true);

    let (prompt, _nonce) = build_outline_revision_prompt(&request, &issue, "修正 kind 分类");

    assert!(
        prompt.contains("[user_option_semantics]"),
        "revision prompt must restate option semantics, not rely on session history"
    );
    for required in [
        "force_frontend_backend_split=true：本次拆分必须产出至少一个 kind=backend 和至少一个 kind=frontend 的 outline",
        "include_integration_tests=true：必须产出至少一个 kind=integration 的 outline",
        "include_e2e_tests=true：必须产出至少一个 kind=e2e 的 outline",
    ] {
        assert!(prompt.contains(required), "missing revision option semantics: {required}");
    }
}

#[test]
fn outline_revision_prompt_omits_semantics_when_options_disabled() {
    let (request, issue, _repository) = split_prompt_fixture();

    let (prompt, _nonce) = build_outline_revision_prompt(&request, &issue, "修正 kind 分类");

    assert!(
        !prompt.contains("[user_option_semantics]"),
        "disabled options must not inject a semantics section into the revision prompt"
    );
}
