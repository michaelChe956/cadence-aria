#[test]
fn artifact_retry_prompt_includes_validation_reasons() {
    let previous_output = "# Story Spec\n\n## 范围\n缺少其余章节。";
    let reasons = vec![
        "缺少 heading: 用户故事".to_string(),
        "缺少 source id".to_string(),
    ];

    let prompt = build_artifact_retry_prompt(&WorkspaceType::Story, previous_output, &reasons);

    assert!(prompt.contains("具体失败原因"));
    assert!(prompt.contains("缺少 heading: 用户故事"));
    assert!(prompt.contains("缺少 source id"));
    assert_eq!(
        prompt.matches(AUTHOR_ARTIFACT_NEGATIVE_LIST).count(),
        1,
        "artifact retry must inject the shared negative list exactly once, not a second copy"
    );
    assert!(prompt.contains("AskUserQuestion"));
    assert!(prompt.contains("待确认项"));
    assert!(prompt.contains("结构化交互"));
    assert!(prompt.contains("用户确认决策"));
    assert!(prompt.contains("author-decision"));
    assert!(prompt.contains("[REQ-"));
    assert!(prompt.contains("[AC-"));
    assert!(
        !prompt.contains("[cadence_project_rules]"),
        "artifact retry must repair the current artifact instead of reopening the routing lifecycle"
    );
}

/// REQ-ACS-03：三个注入点（web 初次 output schema、共享 author output contract、
/// artifact retry contract）必须注入同一份 artifact 输出负面清单教学。
#[test]
fn artifact_author_prompts_teach_the_shared_negative_list() {
    const NEGATIVE_LIST_KEYWORDS: [&str; 5] = [
        "一个",
        "artifact fence 之外",
        "<thinking>",
        "四反引号",
        "不得作为最终候选回显",
    ];

    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
        WorkspaceType::WorkItemPlan,
    ] {
        let retry = build_artifact_retry_prompt(
            &workspace_type,
            "上一轮输出",
            &["缺少 heading".to_string()],
        );

        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session(&format!("sess_negative_list_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        let (_tmp, store) = setup();
        let engine = WorkspaceEngine::new(store, event_tx, session);
        // F-60 P0：共享 author output contract 腿改经 build_streaming_input 出口
        //（装配块 = markdown_author_output_contract_block），仍断言同一份常量。
        let contract = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("author input")
            .prompt;

        for (kind, prompt) in [("retry", &retry), ("author contract", &contract)] {
            assert!(
                prompt.contains(AUTHOR_ARTIFACT_NEGATIVE_LIST),
                "{workspace_type:?} {kind} prompt must inject the shared negative list constant"
            );
            assert_eq!(
                prompt.matches(AUTHOR_ARTIFACT_NEGATIVE_LIST).count(),
                1,
                "{workspace_type:?} {kind} prompt must not carry a second copy of the negative list"
            );
            for keyword in NEGATIVE_LIST_KEYWORDS {
                assert!(
                    prompt.contains(keyword),
                    "{workspace_type:?} {kind} prompt must teach negative-list keyword `{keyword}`: {prompt}"
                );
            }
        }
    }
}
