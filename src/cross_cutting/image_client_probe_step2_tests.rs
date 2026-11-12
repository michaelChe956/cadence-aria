//! 探测第 2 步（POST /v1/images/generations）模型 id 选择策略的用例。
//! 清单如今可来自网关 /v1/models 的全量模型（多数是文本 id），第 2 步必须优先用
//! 图片 id 探测：文本 id 的 404 会被判据误提示成「路径不对」，误导用户。

use super::image_client_tests::{GPT_MODEL, models_endpoint, probe, probe_generations};

#[tokio::test]
async fn p14_probe_step2_prefers_first_image_model_id() {
    // 文本 id 在前、图片 id 在后 → 第 2 步请求体必须用图片 id（mock 按体精确匹配，
    // 现状实现取首项 text-model 时请求落空，本用例必红）。
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no route", 1).await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &["text-model", GPT_MODEL])
        .await
        .expect("探测本身返回报告");
    assert_eq!(
        report.steps[1].status, "ok",
        "第 2 步必须用图片 id 命中端点"
    );
    models.assert_async().await;
    generations.assert_async().await;

    // 大小写不敏感：含 IMAGE 大写变体的 id 同样被选中，而非首项文本 id。
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no route", 1).await;
    let generations =
        probe_generations(&mut server, "Pro-IMAGE-xl", 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &["plain", "Pro-IMAGE-xl"])
        .await
        .expect("探测本身返回报告");

    assert_eq!(report.steps[1].status, "ok", "image 匹配必须忽略大小写");
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p15_probe_step2_falls_back_to_first_without_image_ids() {
    // 清单里没有任何含 "image" 的 id → 回退首项（既有行为，契约固化）。
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no route", 1).await;
    let generations = probe_generations(&mut server, "text-a", 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &["text-a", "text-b"])
        .await
        .expect("探测本身返回报告");

    assert_eq!(report.steps[1].status, "ok", "无图片 id 时仍用首项探测");
    models.assert_async().await;
    generations.assert_async().await;
}
