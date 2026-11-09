# WIG · effort 档位实测对照（xhigh 现状 vs high 方案默认）· v1.0

> 日期：2026-11-09｜执行：controller｜性质：**装置级受控对照**（方案未实现，故对照的是「方案拟推的档位」与「现状档位」在**同一输入**上的行为差异）
> 授权：owner 2026-11-11 已授权真实 provider 调用（范围与判据见 `explicit-sc-author-effort` tasks W.1）

## 1. 为什么这个实验能回答问题

方案 §6.6 G-1 的全部收益论证都缺一个实测数字：`high` 到底比现状（宿主 `xhigh`）快多少、会不会损害质量。
裁决 §3.2 明写「没有 high 速度数据，不承诺 44 分钟→10–15 分钟」。

**关键可行性事实**：SC author 出口当前是 `env_vars: BTreeMap::new()`（**完全继承宿主**），而 `CLAUDE_CODE_EFFORT_LEVEL` 优先于会话选择与 settings。
⇒ **在宿主侧设 `high`，与方案实现后的运行时效果完全相同**（方案额外带来的只是「显式化 + 冻结 + 按原运行持久化」）。
⇒ 方案未实现**不构成**测不了档位对照的障碍。

## 2. 装置（唯一变量 = effort 档）

| 项 | 取值 |
|---|---|
| 输入 | 平台产出的**真实** author prompt（`/tmp/aria-verify/author_prompt.txt`，**52,355 字符**，含 `[issue]` / `[confirmed_context]` / `[markdown_grammar]` / 写范围 / LC 多仓合同） |
| 工作目录 | `/tmp/aria-verify/agg`（LC 多仓 fixture；成员仓 memberA + cadence；目标真实大仓 `/home/michaelche/workspace/test-folder/test-center-java`） |
| 通道 | `magpie` 代理 `127.0.0.1:3425`；模型 `tydic-anthropic/MiniMax-M3.1-Flash[1m]`（与原现场同源） |
| 执行 | `claude -p <prompt> --effort <档> --dangerously-skip-permissions --output-format json` |
| 档位 | **xhigh** = 宿主 `~/.claude/settings.json:31` 的 `effortLevel`（= 现状）；**high** = 方案默认值 |
| 顺序 | 交错 **xhigh → high → high → xhigh**，对冲时间漂移 |
| 授权面 | `--dangerously-skip-permissions` 仅作用于该 headless 会话（为绕开 fixture 的 trust 门），**未改全局配置** |

## 3. 判据（复刻平台编译器，不是启发式）

`src/product/work_item_contract/validation.rs:245-312` 的 plan 域成员校验：

- `task.done_when_refs` ∉ 本 contract 的 `criterion_id` 集合 ⇒ **`unknown_done_when_ref`**（平台真拒收）
- `criterion_id` 在 Handoff Schema 无配对 `reviewer_check_refs` ⇒ **`acceptance_criterion_without_reviewer_check`**
- 另加一项**闭环**检查（非 IR 错误，但直接反映可执行性）：criterion 是否被至少一个 task 的 `done_when_refs` 引用

## 4. 实测结果（n=2/档 · 2026-11-09 18:37~19:00）

| run | 档位 | 墙钟 | turns | output | 其中 thinking | input | cache_read | 成本 | **cross-WI 越界** |
|---|---|---|---|---|---|---|---|---|---|
| high-1 | high | 205.4s | 15 | 33,299 | 26,817 | 61,409 | 320,741 | $1.300 | **0** / 18 |
| high-2 | high | 405.4s | 20 | 60,196 | 51,851 | 66,931 | 573,275 | $2.126 | **2** / 20 |
| xhigh-1 | xhigh | 418.7s | 25 | 67,285 | 57,718 | 83,859 | 887,201 | $2.545 | **15** / 20 |
| xhigh-2 | xhigh | 329.7s | 18 | 54,885 | 46,565 | 80,268 | 554,453 | $2.051 | **13** / 23 |

| 档位 | n | 墙钟 | output | input | cache_read | turns | 成本 | **越界率** |
|---|---|---|---|---|---|---|---|---|
| **xhigh（现状）** | 2 | 374.2s | 61,085 | 82,064 | 720,827 | 21.5 | $2.298 | **28/43 = 65%** |
| **high（方案默认）** | 2 | 305.4s | 46,748 | 64,170 | 447,008 | 17.5 | $1.713 | **2/38 = 5%** |

**两条结论，强度不同：**

**① 质量方向 —— 稳健（2/2 同向，差异大到不像偶然）**
xhigh 两次分别产生 **15 处**与 **13 处** `unknown_done_when_ref`（越界率 **65%**）；high 两次分别 **0 处**与 **2 处**（**5%**）。
⇒ **降档到 `high` 在本样本上同时更快、且 IR 拒收风险低一个数量级** —— 这与「effort 降档会损害质量」的担忧**相反**。
（两次 xhigh 都复现了原现场那条 **15**，说明该缺陷是系统性的、可复现的，不是偶发。）

**② 性能方向 —— 方向一致但被方差主导，量级不可信**
均值：墙钟 **-18.4%**、output **-23.5%**、input **-21.8%**、cache_read **-38%**、成本 **-25.5%**。
但**同档内方差极大**：high 两次 **205s vs 405s（2 倍）**，xhigh 两次 **419s vs 330s（±12%**）。
⇒ **单次或两次运行不足以给性能量级**；n=10/档（owner 已授权）正在补跑。

**③ 机制：档位本身是驱动因素，thinking 总量不是**
初看「越界数随 thinking token 单调上升」，但 n=2 打破了这个相关：
**xhigh-2 的 thinking（46,565）少于 high-2（51,851），越界却多 6.5 倍（13 vs 2）**。
⇒ 起作用的是**档位所改变的推理方式**（更深的逐 token 推理 → 更倾向建立「`AC-` 是全 plan 全局编号」的错误心智模型），
而**不是**思考总量。⚠️ 该判断仍基于 n=4，须由 n=10/档 复核。

## 5. 结论要点

（待复现组完成后填写）

## 6. 局限（必须随结论一起引用）

1. **n=2/档**（owner 授权的是 10/档，本次先跑 2/档 试点）；**同档内方差很大**（见 §5），
   n=2 只能给**方向**，不能给结论量级。
2. **装置级 ≠ 平台全流程**：绝对量级显著小于原现场（原现场 author_run 2679s，本次 xhigh ~419s）——
   探索更少、上下文更小。**但缺陷模式等价**：xhigh 独立复现出与原现场**同一条 15** 的 `unknown_done_when_ref`。
3. **【装置保真度缺口】本次运行 `mcp__codegraph__` 工具未被授权** —— fixture 的
   `.claude/settings.json` 里有 `permissions.allow: ["mcp__codegraph__*"]` 与
   `UserPromptSubmit: codegraph prompt-hook`，但因 `/tmp/aria-verify/agg` **未被 trust**
   （stderr 每次都有 `Ignoring 1 permissions.allow entry … has not been trusted`），
   该授权被忽略 ⇒ **本次 4 次运行的工具轨迹只有 Bash / Read，没有 codegraph**。
   而**原现场有 3 次 codegraph_explore、输出 36,589 字符（占工具输出 38.5%）**。
   ⇒ 本次装置在「探索工具面」上**比原现场贫乏**，绝对耗时/token 不可与原现场直接比。
   （`--dangerously-skip-permissions` 绕开的是权限检查，**绕不开** fixture 的 trust 门。）
4. **方案本身未实现**：本对照测的是「档位」这一个变量，**不覆盖** A/B/C/E/G 包的效果。
5. **单仓单需求**：一个 issue、一个 fixture；不能外推到其它需求形态。
6. **未测 reviewer 侧**：`codex` reviewer 的投影裁剪（`rework-prompt-input-surface`）不在本实验范围。

## 7. 本实验给方案带来的新问题（须登记）

**Q-1：档位对 IR 拒收风险的影响远大于对耗时的影响 —— 且方向与直觉相反。**
本样本 n=2/档：xhigh 越界率 **65%**，high 越界率 **5%**；而耗时只差 **18%**（且方差主导）。
⇒ **effort 档位首先是一个「质量杠杆」，其次才是「性能杠杆」。**
这**改变了方案的收益排序**：方案 §9 把 effort 的收益表述为「时长/token 降幅」，而实测显示它对
**IR 首次通过率**的影响更大（IR 错误直接触发重驱整轮，成本是 623.7 秒 + 一次压缩 + 一次 resume 上下文重建）。
⇒ 建议在方案 §9 的收益表里，把 G-1 的首要收益改写为「**降低 IR 拒收率 ⇒ 消除重驱整轮**」，
时长/token 降幅作为次要收益且**待 n=10/档 复核**。

**Q-2：G-5（上下文分级直供）可能反向 —— 未测，须专门复测。**
直供更多事实可能诱发更多思考 ⇒ 提高 cross-WI 风险。本实验**未测** G-5，故该风险**尚未排除**。
⇒ G-5 实施后**必须**复测 cross-WI 越界率，**不得**默认「直供越多越好」。

**Q-3：装置保真度缺口使绝对数字不可与原现场比。**
本次 4 次运行**没有 codegraph 工具**（fixture 的 trust 门挡掉了 `mcp__codegraph__*` 授权），
而原现场有 3 次 codegraph_explore、输出 36,589 字符（占工具输出 38.5%）。
⇒ 本装置的探索工具面**比原现场贫乏**，绝对耗时/token 不可与原现场直接比；
**可比的只有「同装置、同输入、跨档位」的相对对照**。

**Q-4：n=2 不足以支撑任何决策。**
owner 已授权 10 次/档；本次先跑 2/档 试点，补跑的 16 次正在进行。
**在 n=10/档 出结果前，不得据本次数据修改契约或默认档位。**
