# Tasks

## 1. 只装一家 provider 也能跑（后端）

- [ ] 1.1 `ensure_skills_commands_linked` 增加 provider 参数：给出 provider 时只校验它那一层（Pi→shared / Claude Code→claude / Codex→codex），缺省时保留三层全查；验证：单测覆盖「只装 Pi + codex/claude 层缺失 ⇒ 通过」与「选中 provider 的层缺失 ⇒ 拒绝」
- [ ] 1.2 调用点（coordinator 链接层校验）传入 operation 输入里的 provider；验证：编译通过 + 既有单测不回归

## 2. 知识库操作的无人值守执行（后端）

- [ ] 2.1 provider turn 的 prompt 追加无人值守指令（不提问、不请求授权、不等待回答；无法安全继续时停止并记录）；验证：单测断言 prompt 以命令开头且含「无人值守」
- [ ] 2.2 provider 发问/请求授权 ⇒ 独立稳定码 `knowledge_base_provider_interaction_required` + 失败文案登记；诊断里写入 provider 的提问原文；验证：单测断言新稳定码与 cause

## 3. 失败面的 provider 口径（前端）

- [ ] 3.1 重新检测结论改口径：可用 ≥1 家 ⇒ 「已具备执行条件；其余未安装或不可用——不影响本次执行」；0 家 ⇒ 「至少要有一家可用」；验证：组件测试断言两种措辞，且不再出现「按下面的安装指引处理后点重试」
- [ ] 3.2 环境类失败的诊断折叠标题点明「真实失败原因」（仍保持折叠）；验证：组件测试断言标题文案

## 4. 验收与归档

- [ ] 4.1 门禁：`cargo test --locked`、`cargo fmt --check`、`pnpm -C web test`、`pnpm -C web build`、`pnpm -C web lint`
- [ ] 4.2 勾选工作包并执行 OpenSpec sync/archive
