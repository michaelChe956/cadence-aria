#[cfg(unix)]
#[path = "approval_tests.rs"]
mod approval_tests;

#[cfg(unix)]
#[path = "mcp_bundle_tests.rs"]
mod mcp_bundle_tests;

#[cfg(unix)]
#[path = "tests/session_tests.rs"]
mod session_tests;

#[cfg(unix)]
#[path = "tests/empty_output.rs"]
mod empty_output;

#[cfg(unix)]
#[path = "tests/live_kimi_tests.rs"]
mod live_kimi_tests;

#[cfg(unix)]
#[path = "tests/local_usage_tests.rs"]
mod local_usage_tests;

#[cfg(unix)]
#[path = "tests/tool_policy_zero_change.rs"]
mod tool_policy_zero_change;
