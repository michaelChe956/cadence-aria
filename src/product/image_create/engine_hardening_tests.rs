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
