# Proposal

## Why

owner 2026-10-30 真机跑全流程时的原话：

> 「人工介入是系统该有的行为，但反馈出来的人工介入问题，用户（我）根本看不懂，也就无法合理的输入需要添加的优化 prompt」

现场原文（owner 在结论卡上唯一能看到的文案）：

> 「WI-004 的 write_policy.exclusive_scopes 仍声明 scripts/dev.sh，但该路径位于仓库根本地文件区（根目录不是 git 仓库，仅 busi/gateway/frontend 各自入库）…由人工裁决 WI-004 归属模型，二选一：(1) 将 WI-004 标注为纯验证项并清空 exclusive_scopes；(2) 把 scripts/ 纳入可绑定写范围…」

答案其实就在里面，但埋在 `write_policy.exclusive_scopes`、`fingerprint`、`AC-00x`、`done_when_refs` 这类内部术语里。根因是**契约缺位**：结构化输出 schema 没有任何「问用户一个问题」「给用户几个选项」的字段，模型想说「请你选 A 还是 B」只能塞进 `required_action` 自由文本，前端又原样透传。于是 owner「看不懂 → 写不出补充指令」是必然结果，不是模型不努力。

## What Changes

- **契约层**：`findings[]` 增加两个面向用户的字段——`owner_question`（一句话人话提问）与 `owner_options[{label, consequence}]`（2–3 个互斥选项 + 后果）。
- **fail-closed 校验**：`verdict=needs_human` 时 `owner_question` 与 `owner_options` 必填且非空白；缺失或空白即判结构化输出不合规，**复用既有 `structured_output_diagnostic` 通道**收口（新诊断码 `missing_owner_guidance`），不新造上报/展示机制。
- **Prompt 层**：reviewer 输出契约共享口径（`prompts.rs::OWNER_GUIDANCE_RULES`）加进全部 7 处 reviewer prompt builder；每处 schema 同步给出字段形状。禁止内部术语、`pass`/`revise` 留空。
- **界面层**：`ReviewVerdictEntry` 在 `needs_human` 时先渲染「人话问题 + 选项(label+consequence) + 指引」，技术 findings 折叠进「技术细节」；`pass`/`revise` 渲染零变化。
- **投影**：`owner_question`/`owner_options` 随 `ReviewFinding` 走既有 findings 通道进入 timeline entry metadata（live `review_complete` 与 rebuild `detail.verdict.findings` 两条路径都逐字透传），无需新增第二套投影。

## Capabilities

### Modified Capabilities

- `work-item-plan-single-candidate`：ADDED REQ-WSC-11「面向 owner 的人工裁决呈现义务」——`needs_human` 必须带人话问题与选项，不合规输出按既有诊断通道处置，界面不得把技术术语当唯一呈现。

## Impact

- 后端：`src/web/workspace_ws_types/review.rs`（新 `OwnerOption` + `ReviewFinding` 两字段）、`src/product/workspace_engine/parsers.rs`（字段解析 + `missing_owner_guidance` fail-closed）、`src/product/workspace_engine/prompts.rs` + `prompts/review.rs` + `prompts/review_parts/*.inc.rs`（7 处 prompt/schema）。
- 前端：`web/src/api/types/workspace.ts`、`web/src/components/chat-workspace/finding-list.tsx`、`entries/ReviewVerdictEntry.tsx`。
- 兼容性：两个新字段 `Option` + `skip_serializing_if`，旧记录/旧 wire 字节不变；`pass`/`revise` 与 `revise_batch`/`plan_reopen_required` 等自动返修路径不新增要求。
- 不影响执行/授权/预算：本 change 只改「怎么问 owner」，不改任何 confirm/compile/advance 许可，也不新增 provider 调用机会。

## Non-Goals

- 不做 owner 回答的自动解析或回填（选项名原文即补充指令，由 owner 手写发回）。
- 不为 `revise` 增加「可确认」语义，不改 review gate 的优先级（`blocking`/`must_fix` 仍压过 `needs_human`）。
- 不把技术细节删掉：owner 可能要看，只折叠。
- 不改 coding 侧 code review 契约（其 verdict 词汇与门语义另属 capability）。
