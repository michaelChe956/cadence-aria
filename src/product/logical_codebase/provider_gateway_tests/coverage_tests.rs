//! Task 11 覆盖率:验证逻辑代码库同步与流式 provider 全入口经 gateway 接线后,
//! 每类真实启动都在 gateway 留下可审计记录。本模块是 gateway 侧契约测试(B2):
//! 4 个 `run_logical_*` helper 分别模拟逻辑代码库的 work-item split(同步)、
//! planning/coding/review(流式)启动,各向 gateway 发一次真实 `run_sync`/
//! `start_streaming`,断言审计计数(sync==1、stream==3)与全部携带 policy digest。
//!
//! 该测试是 4 个真实入口(engine.rs / provider_drive.rs / provider_stream.rs /
//! initializer.rs)逻辑分支接线是否完成的验收门:只要 gateway 是唯一启动入口且
//! 记录审计,计数即为契约。
use super::*;

#[tokio::test]
async fn logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    fixture.run_logical_work_item_split().await.unwrap();
    fixture.run_logical_planning_stream().await.unwrap();
    fixture.run_logical_coding_stream().await.unwrap();
    fixture.run_logical_review_stream().await.unwrap();

    assert_eq!(fixture.gateway_audit().sync_launches(), 1);
    assert_eq!(fixture.gateway_audit().stream_launches(), 3);
    assert!(fixture.gateway_audit().all_have_policy_digest());
}
