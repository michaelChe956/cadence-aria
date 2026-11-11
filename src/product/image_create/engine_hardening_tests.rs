//! 安全加固（H2/H3/H5）的 engine 侧外置测试模块。
//! 挂在 engine_tests 模块内部（engine_tests.rs 已达行数上限），
//! 经 `use super::*` 复用其全部测试设施（Fake 会话/设置存储、Fake 图片客户端、
//! Scripted 迭代执行器），语义与内嵌 mod 完全一致。

use super::*;

#[tokio::test]
async fn h3_invalid_base_url_is_rejected_as_invalid_config_without_client_call() {
    // H3：手改设置文件把 base_url 换成非回环 http 时，出站前必须拦下，
    // 返回 InvalidConfig（HTTP 400，与保存时同类问题一致），且客户端零调用。
    let store = Arc::new(FakeSessionStore::with_record());
    let client = Arc::new(FakeImageClient::success());
    let mut settings = valid_settings();
    settings.vendors.get_mut(ImageVendor::Gpt).base_url = "http://evil.example.com".to_string();
    let engine = engine(store, settings, client.clone(), None);

    let error = engine
        .generate("session", auto_command(request("draw"), false, None))
        .await
        .expect_err("非法 base_url 必须在出站前失败");

    assert!(
        matches!(error, ImageCreateError::InvalidConfig(_)),
        "应映射为 InvalidConfig（HTTP 400），实际：{error}"
    );
    assert_eq!(client.calls().await, 0, "出站前失败，客户端零调用");
}

#[tokio::test]
async fn h5_prompt_over_32000_chars_is_rejected_without_client_call() {
    // H5：超过两家官方 32000 字符上限的 prompt 在出站前拒绝（InvalidParam），
    // 避免一次必然被上游拒收、可能计费的往返。
    let store = Arc::new(FakeSessionStore::with_record());
    let client = Arc::new(FakeImageClient::success());
    let engine = engine(store, valid_settings(), client.clone(), None);

    let error = generate_err(
        &engine,
        auto_command(request(&"长".repeat(32001)), false, None),
    )
    .await;

    assert!(
        matches!(&error, ImageCreateError::InvalidParam(message) if message.contains("32000")),
        "超长 prompt 应为 InvalidParam：{error}"
    );
    assert_eq!(client.calls().await, 0, "出站前失败，客户端零调用");
}

#[tokio::test]
async fn h5_prompt_of_exactly_32000_chars_passes() {
    let store = Arc::new(FakeSessionStore::with_record());
    let client = Arc::new(FakeImageClient::success());
    let engine = engine(store, valid_settings(), client.clone(), None);

    let result = engine
        .generate(
            "session",
            auto_command(request(&"画".repeat(32000)), false, None),
        )
        .await
        .expect("恰好 32000 字符的 prompt 应通过");

    assert_eq!(result.prompt.chars().count(), 32000);
    assert_eq!(client.calls().await, 1, "边界值放行，客户端恰好调用 1 次");
}

/// 直接返回纯文本（无结构化块）的迭代执行器：用于断言 text 回退事件也走脱敏。
struct RawTextProvider {
    text: String,
}

#[async_trait]
impl StreamingProviderAdapter for RawTextProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let completion = ProviderCompletion::from_output(
            self.text.clone(),
            None,
            Some("provider-session".to_string()),
        );
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(1);
        tokio::spawn(async move {
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// 跑完一轮迭代，返回（text 回退事件文本，prompt 事件文本）。
async fn collect_iteration_outputs(engine: &ImageCreateEngine) -> (Option<String>, Option<String>) {
    let mut events = engine
        .start_iteration("session", "完善提示词".to_string())
        .await
        .expect("start iteration");
    let mut text = None;
    let mut prompt = None;
    while let Some(event) = events.recv().await {
        match event.kind.as_str() {
            "text" => text = event.text,
            "prompt" => prompt = event.suggested_prompt,
            _ => {}
        }
    }
    (text, prompt)
}

#[tokio::test]
async fn h2_iteration_prompt_output_with_gpt_key_is_redacted_before_persist() {
    // H2：迭代执行器 cwd 在 .aria 之下，能读到明文 key 并写进输出；
    // 落库与推事件前必须把 GPT 家 key 替换为 [REDACTED]。
    let store = Arc::new(FakeSessionStore::with_record());
    let provider = Arc::new(ScriptedIterationProvider {
        suggested_prompt: Some("用 sk-gpt 重述，突出光影层次".to_string()),
    });
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        Some(provider),
    );

    let (_text, prompt_event) = collect_iteration_outputs(&engine).await;

    let record = store.record.lock().await;
    let record = record.as_ref().expect("record");
    let persisted = record.prompt_blocks[0].content.clone();
    assert!(
        !persisted.contains("sk-gpt"),
        "落盘 prompt 不得含明文 key：{persisted}"
    );
    assert!(
        persisted.contains("[REDACTED]"),
        "应替换为 [REDACTED]：{persisted}"
    );
    assert_eq!(
        record.session.current_prompt.as_deref(),
        Some(persisted.as_str()),
        "会话 meta 的 current_prompt 同步脱敏"
    );
    drop(record);
    assert_eq!(
        prompt_event.as_deref(),
        Some(persisted.as_str()),
        "prompt 事件与落盘文本一致（同样脱敏）"
    );
}

#[tokio::test]
async fn h2_iteration_prompt_output_with_minimax_key_is_redacted_before_persist() {
    // H2：脱敏必须覆盖两家，不只是本次目标家。
    let store = Arc::new(FakeSessionStore::with_record());
    let provider = Arc::new(ScriptedIterationProvider {
        suggested_prompt: Some("参考 sk-minimax 的配置重写提示词".to_string()),
    });
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        Some(provider),
    );

    let (_text, _prompt) = collect_iteration_outputs(&engine).await;

    let record = store.record.lock().await;
    let persisted = record.as_ref().expect("record").prompt_blocks[0]
        .content
        .clone();
    assert!(
        !persisted.contains("sk-minimax"),
        "落盘 prompt 不得含 MiniMax 明文 key：{persisted}"
    );
    assert!(
        persisted.contains("[REDACTED]"),
        "应替换为 [REDACTED]：{persisted}"
    );
}

#[tokio::test]
async fn h2_iteration_prompt_output_without_keys_is_verbatim() {
    // H2：输出不含 key 时必须逐字保留，不得改写用户的 prompt。
    let suggested = "水彩风格，柔和光线，留白构图";
    let store = Arc::new(FakeSessionStore::with_record());
    let provider = Arc::new(ScriptedIterationProvider {
        suggested_prompt: Some(suggested.to_string()),
    });
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        Some(provider),
    );

    let (_text, prompt_event) = collect_iteration_outputs(&engine).await;

    let record = store.record.lock().await;
    assert_eq!(
        record.as_ref().expect("record").prompt_blocks[0].content,
        suggested,
        "无 key 时落盘文本逐字不变"
    );
    drop(record);
    assert_eq!(prompt_event.as_deref(), Some(suggested), "事件文本逐字不变");
}

#[tokio::test]
async fn h2_iteration_text_fallback_event_is_redacted() {
    // H2：无 suggested_prompt 时的 text 回退事件同样脱敏（经 REST 回前端）。
    let store = Arc::new(FakeSessionStore::with_record());
    let provider = Arc::new(RawTextProvider {
        text: "读取到两把 key：sk-gpt 与 sk-minimax，建议轮换".to_string(),
    });
    let engine = engine(
        store.clone(),
        valid_settings(),
        Arc::new(FakeImageClient::success()),
        Some(provider),
    );

    let (text, prompt) = collect_iteration_outputs(&engine).await;

    let text = text.expect("无 suggested_prompt 时应推送 text 回退事件");
    assert!(
        !text.contains("sk-gpt") && !text.contains("sk-minimax"),
        "text 事件不得含明文 key：{text}"
    );
    assert_eq!(
        text.matches("[REDACTED]").count(),
        2,
        "两家 key 均被替换：{text}"
    );
    assert!(prompt.is_none());
    let record = store.record.lock().await;
    assert_eq!(
        record.as_ref().expect("record").messages[0].content,
        "完善提示词",
        "用户自己的消息不改写"
    );
}
