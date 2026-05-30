//! 集成测试入口：task_run 域。
#[path = "it_task_run/task_init_and_intake.rs"]
mod task_init_and_intake;
#[path = "it_task_run/task_run_command.rs"]
mod task_run_command;
#[path = "it_task_run/task_run_interactive_runner.rs"]
mod task_run_interactive_runner;
#[path = "it_task_run/task_run_openspec_bootstrap.rs"]
mod task_run_openspec_bootstrap;
#[path = "it_task_run/task_run_orchestrator.rs"]
mod task_run_orchestrator;
#[path = "it_task_run/task_run_provider_factory.rs"]
mod task_run_provider_factory;
#[path = "it_task_run/task_run_step_runner.rs"]
mod task_run_step_runner;
#[path = "it_task_run/task_run_store.rs"]
mod task_run_store;
