# claude×重（levels×双脚本）收官证据——3.6 矩阵 6/9

- 收官序列：**rep5 / rep6 / rep7c 三连 workitem 全过（C2 达成，2026-09-11 晨，服务器 v15）**
- 判据口径：C1-重=计划 Confirmed+advance Ready（编码 completed 为加分项）；SOP=levels 语料+双 request-change 脚本+workitem 预算 60min
- 通过线逐项（三跑全绿）：digest 锚 levels（driver fixtures_seeded）+completed+confirmed=1+advance_actions[0]=completed+attempt_id 非空+ledger before==after+handoff confirmed+**durable group-initializations admission_kind=sc_advance**（attempt 匹配）

## 三跑明细

| 跑 | issue | 时长(workitem) | 修订轮 | coding(加分项) | push 远端实测 |
|---|---|---|---|---|---|
| rep5 | issue_0203 | 2062s | 6 | ✅ completed 同 attempt（coding_attempt_a28e17fe） | naruto aria/issues/issue_0203=610f142，worktree 净 |
| rep6 | issue_0204 | 2147s | 7 | ❌ InvalidRepairTarget(route VerificationRetry×CurrentWorkItem)→blocked_gate 分诊正确拦（加分项 miss，不计 C2） | 分支已推进 daae1a2 |
| rep7c | issue_0207 | 1526s | 4 | ✅ completed 同 attempt（coding_attempt_41902716） | naruto aria/issues/issue_0207=7019223，worktree 净 |

## 收官前的完整谱系（v14/v15，如实全录）

| 跑 | 结果 | 形态 |
|---|---|---|
| v14 rep1d | fail | **误燃顶杀**（SC 修订接力 supersede 杀 followups 误燃 reviewer 握手→handshake cancelled；kind 打点取数跑，据此定修法=oracle 候选6 根修 commit 23cdeb55，v15 部署 md5 92c6ee9b） |
| v15 rep1 | fail | 0199 形态：6 轮修订+双脚本后 approval compile 正确拒（3 项 capability 缺口游走） |
| v15 rep2 | fail | 上游波动族（upstream stream idle 3m0s），不计缺陷环 |
| v15 rep2b | fail | 0199 形态（缺口游走+contract_id 跨修订重编号） |
| v15 rep3 | ✅ 全链（issue_0201，7 轮，push 75ccf887）——后被 rep4 断裂 |
| v15 rep4 | fail | 前导语形态（unknown_structured_key 非 section 内容，教学重驱一次仍犯） |
| v15 rep5/rep6/rep7c | ✅ 三连收官（上表） |
| （rep7=上游 reset 不计；rep7b=controller 包装超时误杀作废，非产品因素） |

## 模型/环境

- 模型基线：glm-5.3-flash（用户裁决：四 provider 统一弱模型基线）经 claude CLI 2.1.247 中继
- 服务器：v15（md5 92c6ee9b…，PID 183996，含误燃根修 23cdeb55）
- 两形态闭环：①误燃顶杀=产品缺陷→根修（followups 循环 SC 委托守卫，oracle 候选6，TDD+k3 Approved）②前导语/契约网缺口=模型质量族（产品 fail-closed 正确；概率通过+重跑收敛）
