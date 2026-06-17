//! 集成测试入口：web 域。各子模块原为独立 tests/*.rs，合并以减少二进制数量。
#[path = "it_web/web_api_handlers.rs"]
mod web_api_handlers;
#[path = "it_web/web_cli.rs"]
mod web_cli;
#[path = "it_web/web_coding_attempt_api.rs"]
mod web_coding_attempt_api;
#[path = "it_web/web_coding_ws_handler.rs"]
mod web_coding_ws_handler;
#[path = "it_web/web_event_taxonomy.rs"]
mod web_event_taxonomy;
#[path = "it_web/web_events.rs"]
mod web_events;
#[path = "it_web/web_hard_gate.rs"]
mod web_hard_gate;
#[path = "it_web/web_lifecycle_api.rs"]
mod web_lifecycle_api;
#[path = "it_web/web_listening_line.rs"]
mod web_listening_line;
#[path = "it_web/web_node_context.rs"]
mod web_node_context;
#[path = "it_web/web_policy_runtime.rs"]
mod web_policy_runtime;
#[path = "it_web/web_product_api.rs"]
mod web_product_api;
#[path = "it_web/web_projection.rs"]
mod web_projection;
#[path = "it_web/web_provider_availability.rs"]
mod web_provider_availability;
#[path = "it_web/web_provider_execution_events.rs"]
mod web_provider_execution_events;
#[path = "it_web/web_provider_output_events.rs"]
mod web_provider_output_events;
#[path = "it_web/web_provider_probe.rs"]
mod web_provider_probe;
#[path = "it_web/web_resource_handlers.rs"]
mod web_resource_handlers;
#[path = "it_web/web_runtime_fake.rs"]
mod web_runtime_fake;
#[path = "it_web/web_runtime_persistence.rs"]
mod web_runtime_persistence;
#[path = "it_web/web_runtime_real.rs"]
mod web_runtime_real;
#[path = "it_web/web_static_assets.rs"]
mod web_static_assets;
#[path = "it_web/web_test_controls.rs"]
mod web_test_controls;
#[path = "it_web/web_types.rs"]
mod web_types;
#[path = "it_web/web_work_item_generation.rs"]
mod web_work_item_generation;
#[path = "it_web/web_work_item_plan_author.rs"]
mod web_work_item_plan_author;
#[path = "it_web/web_work_item_plan_revert.rs"]
mod web_work_item_plan_revert;
#[path = "it_web/web_work_item_plan_review.rs"]
mod web_work_item_plan_review;
#[path = "it_web/web_work_item_split_flow.rs"]
mod web_work_item_split_flow;
