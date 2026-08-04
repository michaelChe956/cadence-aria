# Task for reviewer

Scoped re-review：验证 final whole-branch review 的 fix wave（只读不改）。这是最终审查的收尾验证。

## 上一轮 final review 的 findings（逐条核验）
1. **Critical：API key 可经网关错误正文回显泄露**——修法：ImageClientError::HttpStatus body 脱敏（key→[REDACTED]）+ 截断 ≤500 字符；REST/事件/持久化不含 key；加测试。
2. **Important：generate 路由无 DefaultBodyLimit（Axum 默认 2MiB 与参考图 ≤10MiB 契约冲突）**——修法：仅 generate 路由加 11MiB DefaultBodyLimit；validator 仍严格 10MiB；加 2-10MiB 成功/>10MiB 失败测试。
3. **Important：openSession 无 request token（A→B→A 乱序覆盖）**——修法：openSessionRequestSequence + stale check；加 A→B、A→B→A 测试。
4. **Important：resume 失效识别过宽**——修法：收紧为明确 session/resume 失效；加普通错误不 fallback 负例测试。
5. **Minor：openspec tasks.md 未勾选**——修法：25 个已完成项勾 [x]。
6. **Recommendation：ImageGenRequest 无 model 字段**——修法：后端 body 加固定 model="gpt-image-2"（generations/edits 都带）。

## Fix diff（本次审查对象）
`.superpowers/sdd/2026-08-03_计划文档_功能开发_图片创作agent_v1.9/review-ccd7e111..98a4e32b.diff`

## 审查要求
1. **6 findings 是否 ADDRESSED**：逐条核验（Critical 重点——key 是否真在 REST/事件/持久化三层都不出现；脱敏是否覆盖完整 key；截断是否正确）。可参考源码 `src/cross_cutting/image_client.rs`、`src/web/app.rs`、`src/web/handlers/image_create.rs`、`src/product/image_create/prompt_iteration.rs`、`web/src/state/image-create-store.ts`、`openspec/changes/add-image-create-agent/tasks.md`。
2. **fix diff 有无新 breakage**：只审 fix diff（ccd7e111..98a4e32b），新 Critical/Important breakage 才提。
3. 实现者声称：后端 fmt/clippy/311+doc 全过、前端 tsc + 770 测试全过（含新增回归测试）。

## 输出
- 中文。
- 6 findings 逐条：ADDRESSED / NOT ADDRESSED + 证据。
- fix diff 新 breakage（无则明说「无」）。
- 末尾：**最终结论——分支是否 Ready to merge（是/否）**；若还有残留，列阻塞项（应已没有或极少）。
- 只读，不改任何文件。

## Acceptance Contract
Acceptance level: checked
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Return concrete findings with file paths and severity when applicable

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