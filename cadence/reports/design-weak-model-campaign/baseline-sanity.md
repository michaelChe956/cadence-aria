# Design Baseline Campaign 结果（改造前代码，2026-08-23）

## 采集口径
- 36 样本 = 3 provider × 6 形态 × 2 重复；串行 driver；author/reviewer 各 600s、hard cap 1200s
- manifest：gate-manifest-baseline.json（强化校验器 ok=true, 0 errors, 1 warning）
- 语料 digest 校验：14 文件全过

## 成绩
| Provider | full-chain finished | D04 抽象追踪（应放行） | D05 测试越界（应 must_fix） |
|---|---|---|---|
| claude_code | 12/12 | 2/2 正确 | 2/2 漏报 |
| kimi_code | 12/12 | 2/2 正确 | 2/2 漏报 |
| pi | 7/12 | 1 正确 + 1 revise(provider_error 收尾) | 2/2 漏报 |

## 关键发现
1. **测试越界漏报率 12/12 = 100%**：三 provider 的 reviewer 无一识别设计文档中的测试文件/框架/命令/职责分派——WP-1 判例加固的直接依据。
2. pi 结构失败 5/12：provider 输出对话体（缺全部 heading/ID/source），gate fail-closed 正确拦截；其中 1 例 D04 出现 revise 后仍未完成。
3. usage 全部 unavailable（driver 未从事件流提取到 usage 字段）——如实记录。
4. review_rounds_hist: {0:1, 1:35}；full_chain_one_shot=31。

## 对 revised gate 的意义
- 主 gate 36/36：claude/kimi 已达标，pi 需从 7 提升至 12（判例+入口契约的收益观察点）
- mini-campaign 边界门槛：D05 漏报需从 12/12 → 0/15；D04 保持假阳=0
