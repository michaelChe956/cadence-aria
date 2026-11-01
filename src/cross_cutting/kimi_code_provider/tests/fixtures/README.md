# ACP fixture relationship
#
# This directory is an intentionally non-executable protocol-shape reference.
# `tests/fixtures/provider/kimi_acp_*_fixture.sh` is the executable transport
# layer for the matching scenario. Shared request shape, IDs, and event payload
# values MUST remain aligned. The source spike only covers the records labelled
# in Task 2's report; synthesized records make unobserved terminal scenarios
# testable and are not asserted to be literal captures.

## Task 1 real ACP captures (redacted)

- `acp_request_permission_bash.redacted.json` freezes a real Kimi ACP 0.38.0
  server-to-client `session/request_permission` request for the `Bash` tool and
  the client response selecting `approve_once`.
- `acp_session_update_tool_call_update.redacted.json` freezes a real
  server-to-client `session/update` `tool_call_update` notification observed
  after that approval. `session/update` is a notification, so its JSON-RPC
  response is correctly represented as `null`, rather than synthesized.

Both were captured using `pwd` in `/tmp`, then field-redacted before writing:
`sessionId`, opaque `toolCallId`, absolute paths (if present), and token/credential
fields are replaced with `<REDACTED>`. Fixture contents deliberately contain no
worktree text, real paths, sessions, credentials, or provider tokens.
