# Bug 报告: code_review 中断门 resolved 后 status 未收口(永久 open)

> **勘误(2026-10-16 晚, design-task-sol 接地复核)**: 本报告根因线索「中断门重试不回写」
> 与当前代码不符: 通用 blocked_gate.inc.rs:207-216 与中断专用 failed_review_recovery.rs:235-245
> 均有 resolve; 现存可证缺陷为 store/recovery.rs:577 调无 action resolve(resolved_action=null)。
> 现场 run_0003 trigger=initial/prior=null 且仍 open, 疑似旧二进制路径(4317 服务自 10-13
> 启动未重启, 部署压住中, 运行版本落后 worktree HEAD)。修复方案与现场归因方法见
> cadence/notes/2026-10-16_修复方案-中断门收口.md。

- 报告人: 测试会话 (2026-10-16)
- 现场: test-java/project_0002 issue_0001, coding_attempt_6aedad1c7b7440b58ce95e862ee51665 (gateway WI-002)
- 严重级: 状态机收口缺陷(数据与流程实际不符)

## 现象

`blocked-gates/coding_blocked_gate_0001.json`:

```json
"kind": "blocked", "title": "代码审查中断",
"description": "provider_empty_output: Pi agent settled without assistant output after one bounded retry",
"reason_code": "code_review_provider_interrupted",
"available_actions": [{"action_id": "retry_review", "label": "重试代码审查"}],
"status": "open", "resolved_action": null,
"created_at": "2026-10-16T10:24:40Z"
```

但该门对应的流程**实际已完成推进**:
- code_reviewer 第二轮 role-run 于 12:37Z 成功执行(937 事件, 含宿主 podman 验证)
- 下游 frontend WI-003 coder 于 12:42Z 启动并交付, WI-004 亦跑完
- plan 走到了「全部完成」确认阶段

即: 重试实际发生了(疑为 owner 在页面点「重试代码审查」), 但门的 status/resolved_action
从未回写, 永久停留在 open。

## 影响

1. 数据面: attempt 状态与真实进度不一致, 状态查询/投影会误判存在待处理门
2. UI 面(待 owner 确认): 该门若仍投影为待处理卡片/通知, 会残留不消失, 干扰判断
3. 幂等性: 若流程恢复逻辑扫描 open 门, 可能重复触发重试

## 复现

1. 触发 reviewer provider 空输出中断(Pi 空输出, 一次有界重试后仍空)→ 产生 blocked 门
2. 页面点「重试代码审查」→ 重试 run 执行
3. 读 blocked-gates JSON: status 仍 open, resolved_action 仍 null

## 根因定位线索

重试路径(`retry_review` action)只发起新 role-run, 未回写门状态;
对照: busi attempt(dd5984)的验证门有 `resolved/` 子目录归档机制, 本门未走该路径——
两种门的 resolve 通路不一致(验证门走「说明情况提交」resolve, 中断门的重试 action 可能缺 resolve 落盘)。

## 建议

1. 所有 gate action(重试/终止/说明)执行成功后统一回写 status=closed + resolved_action + resolved_at
2. 建议 run 启动时带 gate_id 关联, run Completed 时幂等收口对应门
3. 补一个对账: 流程推进到下一 stage 时扫描本级残留 open 门并告警

## 待确认

- owner 页面上该门卡片是否残留(本次未截图); 若 UI 已不显示, 则仅数据面问题
