# pi×轻 格证据（C1 轻 3/3 全过,2026-09-08）

- 口径:provider=pi(**glm-5.3-flash 弱模型基线**,用户裁决)|语料=minimal(08-minimal-hello-api.md,digest ca554062)|脚本=request-change:请在契约定义中补充错误响应的示例;confirm;advance(单 request-change val 口径,用户裁决⑤)|flow=single_candidate+interactive|workitem 预算 30min
- 前置:naruto 目标仓「产物自动提交」开关 关闭→开启(naruto main 23457ba,用户裁决——修复 flash 字面服从「禁止 commit」导致的空提交链,零产品代码改动)
- 3 连跑(rep1m2/rep2m/rep3m=issue_0142/0143/0144)全部:workitem 段全绿(confirmed+advance send/completed+admission_kind=sc_advance+同 attempt+ledger 零新增)→coding 段 completed(同 attempt,agent 自行 commit)→push 远端实测一致→worktree 干净
- 通过线=oracle Q5 十四条 AND(含 sc_advance/同 attempt 两条 durable 回读),逐跑字段在各自 result.json/coding-result.json
- 计时:workitem 434.8/582.0/642.8s;coding 1006.4/398.4/424.4s(rep1m2 含 1 轮 review 返修 commit)
- 同格历史:rep1m(fail,开关关闭时空提交链,证据在 pi-light-rep1e-exploratory 同级待移入/见 /tmp)——不计入;rep1/1b/1c/1e=语料口径失配探索跑,另档
- deferred 伴随登记:产物自动提交开关用户分歧→后续做可配置功能(用户指示);sc_advance 预置 head_commit 废掉 runner 兜底 commit 的代码级观察(oracle 中断未完成,供未来功能参考)
