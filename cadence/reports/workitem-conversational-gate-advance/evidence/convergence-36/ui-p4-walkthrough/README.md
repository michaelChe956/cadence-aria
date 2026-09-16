# 3.7 Phase 4 真实链代理预检（ui-p4-walkthrough）

> 执行：2026-09-16，controller 代理预检（REQ-UI37-12：自动化替身只做回归；本页为代理预检记录，人工结论仍由用户走查填写 Phase4 关闸报告 §5）。
> 环境：v16e 生产嵌入构建（PID 3648229 / md5 485befa4 proc=disk 一致 / health ok，三对账证据 /tmp/aria-37-v16e/deploy-check.txt）；真实 `pi` provider + `ARIA_FIXTURE_SET=minimal` 轻语料；全程浏览器操作（driver 仅制造前置状态，未代点任何 UI 动作）。
> 种子：workitem driver rep2（issue_0261 / session_0414，request-change;confirm;advance 全链，advance_completed，SC attempt `coding_attempt_402f6dcd570848f985a68ce76d81d999`）；rep1 失败留档（plan EARS 不合规×教学重驱后仍败，3.6 既知模型质量族，产品 fail-closed 正确）。

## 场景结果总表

| ID | 场景 | 结果 | 证据 |
|---|---|---|---|
| A 正链 | UI 启动 Coding → 实时日志增长/贴底/不抢滚 | ✅ 通过 | 日志 mono 行 12→25→33 增长；贴底：内容 1980→2280px 期间 scrollTop 1673→1973 保持 atBottom；手动上滚 scrollTop=100 后 12s 未被抢回；runner 全链到 final_confirm completed；A-before-start/A-start-clicked/A-sticky-bottom/A-final-state.png |
| A SC 准入 | advance durable ready + attempt 匹配 | ✅ 通过（P0+） | advance_cmd-advance-2-86ed848718.json：status=ready，attempt_id 与页面 attempt 全等（A-advance-record.json 已存） |
| A 四拒绝码 | coding_message_not_allowed / SC_CODING_REQUIRES_ADVANCE / coding_runner_already_started / work_item_execution_plan_not_confirmed | ⏳ 未覆盖（P0+） | 四码需特定引擎状态（阶段竞态/非 Ready 窗口/双击窗口/require=true 语料），本次链路未自然产生；不伪造——留给用户走查或后续覆盖 |
| A 多节点拓扑 | levels 补充链 | ⏳ 未覆盖（P0+） | 需另跑 pi+levels 链（20min+），本次 minimal 单节点 WI-001 拓扑正常渲染 |
| B 编码侧快捷键 | coding 阶段三键（confirm/接管/advance）零协议噪音 | ✅ 通过 | 诊断计数 0→0，无错误 toast，无发送；Ctrl+Enter 于 final_confirm 门开态真实发送确认（时序意外=对话侧 confirm 语义正向实证） |
| B 对话侧带门四键 | 开态门上 Ctrl+Enter/F/Shift+T 二击/Ctrl+A | ⏳ 部分留用户 | confirm 语义已实证（同 facade）；完整四键+焦点守卫细节由用户走查（关闸 §2.1） |
| C 批量框架 | checkbox+aria-label+「批量确认 1 项」+如实标注文案+审计行 | ✅ 通过（框架） | checkbox aria「选择 门禁等待（需分诊）」；文案「同会话批量确认；…通常只确认 1 项」生产可见；confirm 审计行含完整 gateId 生产格式（session:snapshot:…|3|true||指纹）；C-bulk-selected/confirmed.png |
| C 正向门关闭 | 批量确认后门关闭恰一次 | ⏳ 未覆盖 | 0017 引擎 stage=completed（见下方发现 1），confirm 被引擎 INVALID_MESSAGE_FOR_STAGE 拒——无活门可正向验证；留用户走查 |
| D 审计 | 本地行+durable 行+操作者+时间+刷新对照 | ✅ 通过 | start_coding 行：本地浏览器+client ISO(04:04:24.806Z)+attempt+completed+「本地命令日志」；final_confirm 行：「REST 快照」+「时间由会话快照提供」；刷新后本地行消失、durable 行保留（精确符合 Ruling-2）；「按目标回查/仅此目标」交互在场 |
| E takeover 返回父级 | arm 二击→子会话→返回父→刷新存活→新标签丢失 | ✅ 全过（含 P0+ 边界） | UI 二击发出 takeover REST（200）+sessionStorage `aria.takeover-parent:<child>` 写入；子会话「返回父会话」→路由回 0017；reload 后按钮+键保留；独立新标签页无按钮无键；E-takeover-armed/E-back-to-parent.png |
| E plan-repair 返回父级 | plan-repair 子会话返回父 | ⏳ 未覆盖 | 工作树 durable 无 plan_repair 会话样本（F6 模型限制，P3 测试均 review-fixture 驱动）；按方案纪律不捏造——留真实使用中验证 |
| F 双轨回滚 | legacy↔cockpit | ✅ 通过 | legacy 旧形态渲染正常（输入栏在/无 cockpit 元素）；切回 cockpit 批量文案+审计入口恢复；F-legacy/F-cockpit-back.png |
| G 归一 | 终止/接管同款 arm+10s 回落 | ✅ 通过 | 终止首击「确认终止」出现（与接管同款 ConfirmTwiceButton），未二击 10.5s 后自动回落；G-terminate-armed.png；静态归一 6 项见关闸报告 §3 |

## 发现（如实）

1. **[观察·1b 族] 引擎 stage=completed 的会话，前端快照门投影仍呈现为可操作**：0017（3.6 遗留 blocked 会话）的「门禁等待（需分诊）」门在 UI 有 checkbox+确认+批量按钮；confirm 发出后被引擎 `INVALID_MESSAGE_FOR_STAGE: message human_confirm not allowed in stage completed` 拒；前端把协议错误正确内联为 hard_error 条目（2.7 分流正常）。定性：与 1b 已知「observer 快照最多 15 秒陈旧」同族（此处为停摆会话的长效陈旧），非 Phase 4 新引入——T3 门守卫按计划以「前端投影存在未关闭门」为判据，已按计划实现。后续可在投影层加引擎 stage 终态守卫（backlog 候选）。
2. **[driver 缺陷] workitem driver stage3 readback bug**：`stage3_group_snapshot_readback_failed: elapsedMs is not a function`——driver 脚本自身缺陷（不影响产品链，advance/workitem 段已全完成，result.json 完整）。登记待修。
3. **[时序备注] 代理预检的一次 UI 接管二击未生效**：首击 arm→（截图调用）→二击后未见请求与 sessionStorage 写入；同参数立即复现则成功（请求+键+跳转全对）。疑似截图调用造成 tab 短暂冻结吞掉点击（§5.4 冻结教训同族）。复现成功证明链路本身健康；用户走查不涉及截图冻结，预期无此现象。

## 用户走查补充指引

- B 对话侧带门四键：任选一个有开态门的真实会话（建议现场用 driver 种子或既有 pending 会话）；Ctrl/Cmd+Shift+T 接管需 stopped 条目在场。
- C 正向门关闭：需要活的引擎侧门（新种子会话 confirm 后门应关闭且审计恰一条 confirm）。
- A 四拒绝码：`coding_runner_already_started` 最易复现（开始按钮双击）；其余三码需特殊状态，无法稳定制造时按方案登记「顶检未覆盖」即可，不影响 P0 验收。
- E plan-repair：如有机会在真实使用中触发 plan repair（coding defect 修订），顺路验证「返回父会话」。

## 截图清单

A-before-start / A-start-clicked / A-sticky-bottom / A-final-state / C-bulk-selected / C-bulk-confirmed / E-takeover-armed / E-back-to-parent / F-legacy / F-cockpit-back / G-terminate-armed（.png，同目录；另 /tmp/aria-37-v16e/ 有完整过程证据含 deploy-check/health/advance-record）
