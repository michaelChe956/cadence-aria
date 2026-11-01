# codex×重 格：flash-via-codex 基线限制（用户裁决 2b,2026-09-09 暂缓）

## 现状：0 通过 / 10 尝试（全部真实跑,失败谱系如下）
| 尝试 | issue | 失败形态 | 定性 |
|---|---|---|---|
| rep1 | 0158 | Codex resume stalled | 上游中继波动 |
| rep1b | 0166 | approval compile 拒:WI-001 缺 2 能力 | 作者缺陷族①能力覆盖 |
| rep1c | 0167 | 同上(WI-001 缺 2 能力) | 同族 |
| rep1d | 0168 | 同上(修好 WI-001 边,缺口挪 WI-002→WI-003) | 同族(多边挪移) |
| rep1e | 0169 | IR 校验拒:duplicate_contract_id CT-001/002×3/004 | 作者缺陷族②契约重复(教学过纠) |
| rep2e | 0170 | compile 拒:能力缺口+能力数组逗号拆分 | 族①+新子形态 |
| rep1v9 | 0177 | Codex resume stalled | 上游 |
| rep1v9b | 0178 | compile 拒:能力缺口(机械回灌 6 轮仍不修——**适配器等价性勘察证实 prompt 逐字节同源+findings 原文直达,模型读而不做上游编辑**) | 模型行为限制 |
| rep1v10 | 0184 | compile 拒:invalid_work_item_id 标识符重复 | 作者缺陷族③ID 重复 |

## 关键证据
- 适配器等价性(adapter-parity-scout.md):pi(3/3) vs codex(0/8) prompt 逐字节同源——**模型行为差异,适配器无可修**
- 结构性修复已尽:契约校验前移+逐字补丁 required_action+教学两轮+机械重驱全上,仍新角落迭出
- 对照:同模型经 pi 通道 pi×重 3/3 全过(0163/0164/0165)

## 后续选项(用户后续定)
a) 换该格模型(非 flash) b) 降语料 c) 接受限制记录进测量报告 v2 d) 等 codex harness 改进后重试
