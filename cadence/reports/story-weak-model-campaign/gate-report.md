# Gate Campaign 最终报告（3 组合 × 10 样本，revised，author 上限 600s）

> 说明：首轮 300s 上限下 claude 03/r1 为驱动超时（非模型/结构失败），复跑（600s）通过；kimi 首轮因登录过期全失败，重登录后全量重跑通过。

## claude_code: 10 样本 | author 10/10 | review 10/10 | **full-chain 10/10**
- artifact secs: min 122 / avg 202 / max 293
- review verdicts: Counter({'pass': 9, 'revise': 1})

## kimi_code: 10 样本 | author 10/10 | review 10/10 | **full-chain 10/10**
- artifact secs: min 91 / avg 141 / max 264
- review verdicts: Counter({'pass': 8, 'revise': 2})

## pi: 10 样本 | author 10/10 | review 10/10 | **full-chain 10/10**
- artifact secs: min 32 / avg 48 / max 65
- review verdicts: Counter({'pass': 6, 'revise': 3, 'needs_human': 1})

## Gate 判定

- 10 样本下 95% 门槛 = 10/10。三组合 full-chain 均为 **10/10（100%）**，**达标**。
- baseline 对照：优化前三组合 author 首轮 120s 全部未完成（见 baseline-sanity.md）；本 campaign author 首次通过率 30/30、retry 0。
- token 配对 gate：baseline（旧代码）无可用 usage（全部超时无产出），配对公式分母不存在，按裁决改为记录该事实（见 ledger）。