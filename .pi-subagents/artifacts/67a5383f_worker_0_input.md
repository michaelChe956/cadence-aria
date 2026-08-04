# Task for worker

排查图片创作 Agent 的图片生成超时问题。这是调试任务，先定位根因，**不要急着改代码**——先看清楚错误的具体类型再决定修法。

## 现象
用户在 http://127.0.0.1:5173/image-create 上传参考图生成图片，后端调用 colorflowai 网关 `POST https://colorflowai.com/v1/images/edits`，**32.7 秒后失败**，前端报"生成失败：image client error: image provider request timed out"。

## 已有的诊断日志（关键！）
后端日志 `/tmp/aria-imgcreate-logs/backend.log`：
```
aria web listening on http://127.0.0.1:4317
[image-create] POST https://colorflowai.com/v1/images/edits (endpoint=edits, has_reference=true, size=Auto, quality=Auto) starting
[image-create] POST https://colorflowai.com/v1/images/edits FAILED after 32.713696663s: error sending request for url (https://colorflowai.com/v1/images/edits)
```

## 关键观察
1. 请求**确实发出去了**（edits 端点，有参考图，filename 修复已生效，不再是 "image is required"）
2. **32.7 秒后失败**——不是 reqwest 总超时（代码设了 `timeout(Duration::from_secs(600))`，那要等 10 分钟）
3. 错误是 reqwest 的 `error sending request for url`（Display），**但没看到具体 kind**（is_timeout? is_connect? is_body? 网关主动断开?）

## 你的任务（诊断为主）
1. **读 `src/cross_cutting/image_client.rs`** 的 `normalize_reqwest_error` 函数（看它怎么分类 reqwest 错误），以及诊断日志（`eprintln!` 那段，约 150-175 行）。
2. **关键**：当前诊断日志只打了 `{error}` 的 Display，没打 reqwest 错误的 **kind/source**。32.7s 失败可能是：
   - reqwest 的 **connect timeout**（默认或某层）——但 reqwest 默认无 connect timeout 除非显式设
   - **TLS 握手超时**
   - **网关 colorflowai 主动在 ~30s 关闭连接**（网关侧超时，gpt-image-2 生成慢被网关掐）
   - **proxy/网络层超时**
3. **改进诊断日志**，把 reqwest 错误的**完整信息**打出来：`error.is_timeout()`、`error.is_connect()`、`error.is_request()`、`error.is_body()`、`error.source()` 链（层层打印 source 直到 None）。这样下次失败能看到 32.7s 到底是哪类超时。
   - 修改 `src/cross_cutting/image_client.rs` 的失败日志那段（约 162-170 行的 `map_err(|error| {...})`），把 error 的 kind 和 source 链都 eprintln 出来。
4. **研究 colorflowai 网关**：它对 `/v1/images/edits`（gpt-image-2）是否有请求超时？gpt-image-2 生成通常要多久？是否网关在 30s 就掐断长任务？可联网搜索 "colorflowai gpt-image-2 timeout" 或看 colorflowai 文档。
5. **不要改业务逻辑**，只改诊断日志（让错误信息更详细）+ 给出根因判断和修复建议。

## 输出
- 完整诊断报告写到 `.superpowers/sdd/2026-08-03_计划文档_功能开发_图片创作agent_v1.9/diagnose-timeout-report.md`（如果该目录不存在，写到 `/tmp/diagnose-timeout-report.md`）。
- 返回：根因判断（32.7s 是哪类超时）、修复建议、是否已改进诊断日志（commit hash）。

## 🔴 纪律
- 这是诊断任务，**先把错误 kind 看清楚**再谈修复。
- 改诊断日志后 build（`cargo build --locked --bin aria`），但**不要重启后端**（controller 会重启让用户重测）。如果 build 失败，修到通过。
- 工作目录是 worktree 根。

## Acceptance Contract
Acceptance level: checked
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Implement the requested change without widening scope

Required evidence: changed-files, tests-added, commands-run, residual-risks, no-staged-files

Finish with a fenced JSON block tagged `acceptance-report` in this shape:
Use empty arrays when no items apply; array fields contain strings unless object entries are shown.
`criteriaSatisfied[].status` must be exactly one of: satisfied, not-satisfied, not-applicable.
`commandsRun[].result` must be exactly one of: passed, failed, not-run.
`manualNotes` and `notes` are optional strings; an empty string means no note and does not satisfy `manual-notes` evidence.
```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "specific proof"
    }
  ],
  "changedFiles": [
    "src/file.ts"
  ],
  "testsAddedOrUpdated": [
    "test/file.test.ts"
  ],
  "commandsRun": [
    {
      "command": "command",
      "result": "passed",
      "summary": "short result"
    }
  ],
  "validationOutput": [
    "validation output or concise summary"
  ],
  "residualRisks": [
    "none"
  ],
  "noStagedFiles": true,
  "diffSummary": "short description of the diff",
  "reviewFindings": [
    "blocker: file.ts:12 - issue found, or no blockers"
  ],
  "manualNotes": "anything else the parent should know"
}
```