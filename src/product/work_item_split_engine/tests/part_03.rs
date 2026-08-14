#[test]
fn build_split_prompt_inlines_schema_and_kind_guidance() {
    // 回归 Bug: prompt 曾引用不存在的 `src/product/work_item_split_output_schema.json`,
    // 而 WORK_ITEM_SPLIT_OUTPUT_SCHEMA 常量未注入 prompt,导致 provider 不知道
    // `kind` 是必填字段,按习惯输出 `type` 触发 `missing field kind`。
    // 修复后 prompt 必须内联 schema 正文并给出 kind 合法取值。
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)", &RoutingReferenceContext::Legacy);

    assert!(
        !prompt.contains("work_item_split_output_schema.json"),
        "prompt must not reference a non-existent schema file path: {prompt}"
    );
    // schema 正文必须内联进 prompt(取 schema 常量里的标志性片段)。
    assert!(
        prompt.contains("\"kind\""),
        "prompt must inline the schema's `kind` property: {prompt}"
    );
    assert!(
        prompt.contains("\"required\""),
        "prompt must inline the schema's `required` clause: {prompt}"
    );
    // kind 合法取值引导(provider 必须知道有哪些枚举值可选)。
    for kind_value in [
        "backend",
        "frontend",
        "integration",
        "e2e",
        "docs",
        "infra",
        "other",
    ] {
        assert!(
            prompt.contains(kind_value),
            "prompt must list kind value `{kind_value}`: {prompt}"
        );
    }
}

#[test]
fn build_split_prompt_allows_readable_stream_before_final_sentinel() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)", &RoutingReferenceContext::Legacy);

    assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
    assert!(prompt.contains("最后必须输出一个 nonce sentinel JSON block"));
    assert!(prompt.contains("后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT"));
    assert!(prompt.contains("不要输出 Markdown code fence"));
}

#[test]
fn split_prompt_requests_progress_before_long_operations() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)", &RoutingReferenceContext::Legacy);

    assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
    assert!(prompt.contains("先输出一行简短可读状态"));
    assert!(prompt.contains("每完成一组探索后输出一句当前发现摘要"));
}

#[test]
fn build_revision_prompt_inlines_schema_and_kind_guidance() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
        &RoutingReferenceContext::Legacy,
    );

    assert!(
        !prompt.contains("work_item_split_output_schema.json"),
        "revision prompt must not reference a non-existent schema file path: {prompt}"
    );
    assert!(
        prompt.contains("\"kind\""),
        "revision prompt must inline the schema's `kind` property: {prompt}"
    );
    assert!(
        prompt.contains("\"required\""),
        "revision prompt must inline the schema's `required` clause: {prompt}"
    );
    for kind_value in [
        "backend",
        "frontend",
        "integration",
        "e2e",
        "docs",
        "infra",
        "other",
    ] {
        assert!(
            prompt.contains(kind_value),
            "revision prompt must list kind value `{kind_value}`: {prompt}"
        );
    }
}

#[test]
fn build_revision_prompt_allows_readable_stream_before_final_sentinel() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
    assert!(prompt.contains("最后必须输出一个 nonce sentinel JSON block"));
    assert!(prompt.contains("后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT"));
    assert!(prompt.contains("不要输出 Markdown code fence"));
}

#[test]
fn revision_prompt_requests_progress_before_long_operations() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
    assert!(prompt.contains("先输出一行简短可读状态"));
    assert!(prompt.contains("每完成一组探索后输出一句当前发现摘要"));
}
