# Task 7 Report — Pi structured asking

## What was implemented

- Added `aria-ask.ts`, embedded at compile time with `include_str!`. It registers only the `ask_user` tool and calls `ctx.ui.select()`; it does not register a `tool_call` interceptor.
- Added content-addressed extension cache delivery under the OS temp directory and supplies it to Pi through `-e <path>` while preserving Auto-only operation.
- Added Pi select request parsing and a frozen protocol fixture.
- Mapped `extension_ui_request(method=select)` to `ProviderEvent::ChoiceRequest` with `ProviderChoice` source, string option id/label preservation, single-select, and free-text enabled.
- Refactored command waiting to handle both aborts and choice responses, including during RPC handshakes. Responses map to full `extension_ui_response` envelopes; empty responses use `{type,id,cancelled:true}`. Incorrect response IDs emit `CHOICE_ID_UNMATCHED` and continue waiting.
- Added bounded Pi version probing in `start()`: known versions below 0.83.0 are rejected before spawning; unparseable, missing, and timed-out probes remain non-blocking unknown versions.
- Updated Pi workspace guidance to require `ask_user` instead of the textual pause fallback.

## TDD evidence

### RED

- `cargo test -p cadence-aria pi_provider_aria_ask_extension` initially failed because `ARIA_ASK_EXTENSION`, select parsing, and version helpers were undefined.
- `cargo test -p cadence-aria pi_provider` initially failed select duplex tests because no choice event was emitted (the select mapping had not been implemented).
- `cargo test -p cadence-aria pi_story_context_requires_ask_user_tool` initially failed against the prior guidance assertion before the expectation was adjusted to distinguish the Pi branch from unrelated global text-fallback documentation.

### GREEN

- `cargo test -p cadence-aria --lib` — passed, 1468 tests.
- `cargo test -p cadence-aria pi_provider` — passed, 30 Pi-provider tests.
- `cargo test -p cadence-aria pi_version` — passed, 7 matching tests.
- `cargo test -p cadence-aria parse_pi_select` — passed.
- `cargo test -p cadence-aria session_tool_call` — passed.
- `cargo test -p cadence-aria workspace_context` — passed, 24 matching tests after adding Pi guidance coverage.
- `cargo clippy -p cadence-aria --all-targets` — passed.
- `cargo fmt --check` — passed.
- `cd web && npm test && cd ..` — passed, 91 files / 732 tests.

## Files changed

- `src/cross_cutting/pi_provider/aria-ask.ts` — structured Pi extension.
- `src/cross_cutting/pi_provider/mod.rs` — extension delivery, command args, version probe/check.
- `src/cross_cutting/pi_provider/parse.rs` — select request parser.
- `src/cross_cutting/pi_provider/session.rs` — select/choice response protocol bridge and command-wait refactor.
- `src/cross_cutting/pi_provider/tests.rs` — extension, parser, protocol, abort, mismatch, Auto-only, and version tests.
- `src/cross_cutting/pi_provider/tests/fixtures/select_request.jsonl` — frozen select protocol envelope.
- `src/web/workspace_context/prompts.rs` — Pi ask-user guidance.
- `src/web/workspace_context/tests.rs` — Pi guidance test.

## Self-review

- Verified select requests are handled after JSON-RPC response dispatch in both the normal reader and handshake response-wait loops.
- Verified choice responses are not discarded by an abort-only waiter; the shared command handler receives and routes them.
- Verified cancelled response includes both protocol `type` and select `id`.
- Verified ordinary tool execution remains audit-only and does not generate permission or choice requests.
- Verified below-minimum version rejection occurs before process spawn through an executable fake-Pi test.
- Verified formatting, clippy, full Rust library tests, focused provider tests, and frontend tests.

## Concerns

None. Pi version detection intentionally treats probe command failures as unknown/non-blocking so the existing availability gate remains the authoritative missing-command error path.

## Fix round 1 — Task 7 quality findings

### H1 — private, validated extension cache

- Replaced the shared predictable temp-directory cache with `$HOME/.cache/cadence-aria` and restricts the cache directory to mode `0700` on Unix; newly created extension files use `0600`.
- The extension cache fast path now uses `symlink_metadata`, rejects symbolic links and non-regular files, and reads the existing content to require an exact match to `ARIA_ASK_EXTENSION`.
- Publication uses exclusive creation (`create_new`), with `O_NOFOLLOW` on Unix. A concurrent creator is revalidated before reuse. Content mismatches fail closed rather than being passed to Pi.
- Added tests for exact-content reuse, tampered-content rejection, and Unix symlink rejection.

### M1 — version probe cleanup

- Replaced the raw `tokio::process::Command::output()` timeout wrapper with the existing `TokioBoundedCommandRunner`. Its managed process path kills and reaps processes on timeout.
- Added real probe tests: a nonexistent executable returns `PiVersion::Unknown(CommandFailed)`, and a Unix `sleep 30` fixture times out at 50 ms and returns `PiVersion::Unknown(TimedOut)`.

### M2 — closed command channel

- Removed the `command_rx.is_closed()` select guard. A closed receiver now executes `recv()`, yields `None`, and follows the existing abort path rather than disabling the branch and deadlocking.

### L2 — platform guard

- Moved the executable-fixture helper and below-minimum Pi-start test behind `#[cfg(unix)]`; no Unix-only import remains unconditional.

### Validation

- `cargo test -p cadence-aria --lib` — passed, 1470 tests.
- `cargo test -p cadence-aria pi_provider` — passed, 32 matching tests, including the real missing-command and timeout probes.
- `cargo clippy -p cadence-aria --all-targets` — passed with zero warnings.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.

### Protocol scope confirmation

The structured-asking protocol was not changed: select mapping, `ChoiceResponse`, `ProviderChoice` source, and cancelled response envelope behavior remain untouched. This fix only changes cache security, probe process lifecycle, closed-channel waiting, and test portability/coverage.
- Added a closed-command-channel select-wait regression test; it verifies receiver closure sends Pi an abort and completes the session.
- Final validation after that regression test: `cargo test -p cadence-aria --lib` passed 1471 tests; `cargo test -p cadence-aria pi_provider` passed 33 matching tests.
