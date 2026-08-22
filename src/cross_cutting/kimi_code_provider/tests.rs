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
#[path = "tests/live_kimi_tests.rs"]
mod live_kimi_tests;
