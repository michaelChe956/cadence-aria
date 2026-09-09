# claude×轻 格证据（C1 轻 3/3 全过,2026-09-09 深夜）
- 口径:provider=claude_code(claude CLI 2.1.247 经用户中继 glm-5.3-flash)|语料=minimal|脚本=单 request-change val 口径|interactive+single_candidate
- 3 连跑(rep4v12/rep5v12/rep6v12=issue_0190/0191/0192)全部全链:workitem 全绿(含 advance)→coding completed→push 远端一致→树净;claude **零实测首秀格历史首通**
- 计数 3 连前史:rep1-1c(信任+握手 cancelled 谜团)/rep1v9-v11(断连被杀+双 spawn 互杀——三层修复链后打通)/rep1v12b-rep2v12 过但 rep3v12 撞 scope violation 断裂(模型质量族)
- 前置修复链(本格解锁所依赖,全部 TDD+过审):~/.claude.json worktree 信任条目/断连清理不取消 run(2200c521)+idle 守卫扩展/ReviewOnly 接力 drain 去双 spawn(104662b0)/claude stderr 尾部直通+非 JSON 首行容忍(aa397db3+2ae61a80)
- 附件:rep1v12b/rep2v12 为断裂系列参考证据
