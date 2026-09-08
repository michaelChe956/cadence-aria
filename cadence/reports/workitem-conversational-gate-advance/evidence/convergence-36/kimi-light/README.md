# kimi×轻 格证据（C1 轻 3/3 全过,2026-09-09）
- 口径:provider=kimi_code(经用户中继 glm-5.3-flash)|语料=minimal|脚本=单 request-change val 口径|interactive+single_candidate|30min 预算
- 3 连跑(rep1v9/rep2v10/rep3v10=issue_0175/0182/0183)全部全链:workitem 全绿→coding completed→push 远端一致→树净
- 特征:kimi coder 不自行 commit(开关不服从)——**runner 兜底 commit 实战开火**(rep1v9 "feat: implement work item");rep1(v9 前置)曾撞 terminal cwd 绝对路径拒(aria 适配器缺陷,935665f3 修复)与 InvalidRepairTarget 结构化角落(重驱后重犯,分诊门正确拦)
- 前置修复链:runner 兜底 commit(18a85d94)+kimi terminal cwd 容忍(935665f3)
