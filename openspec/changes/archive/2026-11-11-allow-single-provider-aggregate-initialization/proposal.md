# 只装一家 provider 也能跑聚合初始化

## Why

真机反馈（2026-11-11）：LC 聚合初始化失败后，界面给出的两处信息让用户以为**四家 provider 必须全部安装**：

1. 「重新检测完成：仍有 2 个 provider 不可用（Claude Code、Codex），按下面的安装指引处理后点「重试」。」——把未安装的 provider 说成必修项；
2. 失败面把 provider turn 的失败与「安装其它 provider」并列展示，用户看不出真实失败原因在哪。

而产品契约上，provider 选择器（`select_aggregate_provider`）本来就是**按 owner 顺序选第一家已核验且可用的**（Pi > Claude Code > Codex），**一家就够**。真正与「必须都装」相悖的是另一处：Cadence-skills 四条命令的链接层校验（`ensure_skills_commands_linked`）要求 shared / codex / claude **三层**都装齐——没装 Codex 时 codex 层缺失，整条聚合初始化被 `skills_commands_missing` 拦死。

用户诉求：**允许只装一个 provider，不强制四家都装。**

## What Changes

- **skills 链接层校验只查被选中 provider 那一层**：选了 Pi 就只要求 shared 层有那四条命令；codex / claude 层缺失（没装那两家）不再阻断本次执行。
- **重新检测结论改成「一家可用即可」**：可用 ≥1 家时明确说「已具备执行条件；另有 N 家未安装或不可用——不影响本次执行，需要换一家时再按指引安装」；一家都没有时才说「至少要有一家可用」。
- **诊断折叠标题点明「真实失败原因」**：环境类失败的真实原因就在这段诊断里，标题点明它，用户知道该点开看什么（仍保持折叠，不把 provider stderr 平铺在卡片上）。

## Non-goals

- 不改 provider 选择规则（owner 顺序选第一家已核验且可用的）——本来就支持一家。
- 不改「能力未核验 ⇒ 先核验」的既有门禁（那是**能力证据**问题，不是安装问题）。
- 不放宽聚合根可写、codegraph 可用等其它环境前置。
