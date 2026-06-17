//! 集成测试入口：product 域。
#[path = "it_product/product_coding_attempt_store.rs"]
mod product_coding_attempt_store;
#[path = "it_product/product_coding_models.rs"]
mod product_coding_models;
#[path = "it_product/product_coding_workspace_engine.rs"]
mod product_coding_workspace_engine;
#[path = "it_product/product_coding_workspace_runner.rs"]
mod product_coding_workspace_runner;
#[path = "it_product/product_git_workspace_service.rs"]
mod product_git_workspace_service;
#[path = "it_product/product_index.rs"]
mod product_index;
#[path = "it_product/product_lifecycle_store.rs"]
mod product_lifecycle_store;
#[path = "it_product/product_runtime_compat.rs"]
mod product_runtime_compat;
#[path = "it_product/product_test_executor.rs"]
mod product_test_executor;
#[path = "it_product/product_tester_agent_loop.rs"]
mod product_tester_agent_loop;
#[path = "it_product/product_work_item_models.rs"]
mod product_work_item_models;
#[path = "it_product/product_work_item_split_engine.rs"]
mod product_work_item_split_engine;
#[path = "it_product/product_work_item_split_validator.rs"]
mod product_work_item_split_validator;
