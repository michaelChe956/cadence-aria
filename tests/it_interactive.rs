//! 集成测试入口：interactive / repl / daemon 域。
#[path = "it_interactive/daemon_cli_process.rs"]
mod daemon_cli_process;
#[path = "it_interactive/daemon_discovery.rs"]
mod daemon_discovery;
#[path = "it_interactive/daemon_lifecycle.rs"]
mod daemon_lifecycle;
#[path = "it_interactive/interactive_checkpoint.rs"]
mod interactive_checkpoint;
#[path = "it_interactive/interactive_checkpoint_preview.rs"]
mod interactive_checkpoint_preview;
#[path = "it_interactive/interactive_controller.rs"]
mod interactive_controller;
#[path = "it_interactive/interactive_policy.rs"]
mod interactive_policy;
#[path = "it_interactive/interactive_projection.rs"]
mod interactive_projection;
#[path = "it_interactive/interactive_store.rs"]
mod interactive_store;
#[path = "it_interactive/repl_auto_start.rs"]
mod repl_auto_start;
#[path = "it_interactive/repl_daemon_handshake.rs"]
mod repl_daemon_handshake;
#[path = "it_interactive/repl_discovery.rs"]
mod repl_discovery;
#[path = "it_interactive/repl_wire.rs"]
mod repl_wire;
