# Bug 报告: 跨仓验收 WI 的证据文件触发 work_item_diff_scope_violation,「全部完成」确认死锁

> **勘误(2026-10-16 晚, design-task-sol 接地复核)**: 本报告「跨仓证据死锁」定性有误。
> 真实根因是**中文路径转义 bug**: gates 侧 GitWorkspaceService::git_commit_changed_files
> 用 `git show --name-only` 无 `-z`(git_workspace_service.rs:555-579), 中文文件名被
> core.quotepath 转成 `"cadence/verify/2026-10-16_\351..."` 带引号八进制串, glob 匹配
> 必失败——owner 收到的报错原文即转义字面量, 为直接铁证。WI-004 实有
> target_repository_id=frontend(worktree 12116fbd), 归属合法; 当前代码为 per-unit
> scope 校验(gates.rs:574-617), 并非「整仓 diff 套单仓 WI scope」。修复方案见
> cadence/designs/2026-10-16_技术方案_跨仓验收证据确认scope误拒修复_v1.0.md。死锁三建议中仅「确认层归属标注」
> 仍有价值, 其余由转义修复取代。

- 报告人: 测试会话 (2026-10-16)
- 现场: test-java/project_0002 issue_0001, plan issue_work_item_plan_0001, 4 WI 全部交付后点总览页「全部完成」
- 严重级: 流程死锁(需人工 git 干预才能解锁确认)

## 现象

「全部完成」逐仓确认在 frontend 仓失败并停止:

```
12116fbd-4358-4806-a317-6c61a2aad323：失败（work_item_diff_scope_violation）
work_item_diff_scope_violation: "cadence/verify/2026-10-16_验证报告_公告管理升级端到端验收_v1.0.md"
```

## 复现链(结构性死锁,非偶发)

1. WI-004(三仓联调验收) `exclusive_write_scopes: ['cadence/verify/**']`,kind 为跨仓验收 WI,**无独立仓库/worktree**
2. coder 只能借用某个成员仓的 worktree 落盘证据——本次选了 frontend,产生两笔提交:
   - `754d197 docs(verify): ...端到端验收验证证据`(v1.0, BLOCKED 结论+阻塞证据)
   - `fbac52d docs(verify): ...v2.0`(按人工补充的替代验收安排修订结论)
   - 路径与内容均符合 WI-004 自己的 scope 定义
3. 「全部完成」确认校验按**仓→绑定该仓的 WI** 查 scope: frontend 仓 diff 含 `cadence/**`,
   违反 WI-003 的 `exclusive_write_scopes: ['frontend/src/**']` → violation,确认中断
4. 死锁: 跨仓验收 WI 按自身 scope 交付的证据,落在**任何一个**成员仓都必然违反该仓所辖 WI 的 scope

## 关键数据

- work-items/WI-003: exclusive=['frontend/src/**'], forbidden=['busi/**','gateway/**']
- work-items/WI-004: exclusive=['cadence/verify/**'], forbidden=['busi/src/**','gateway/src/**','frontend/src/**']
  (注意: WI-004 的 forbidden 明确排除写 frontend/src,但没排除在 frontend 仓**新建** cadence/ 目录;
   而它又没有自己的 worktree——落盘位置从一开始就无处合法)
- 验证计划: CHECK-008/009/010 全为运行期取证(经网关分页/数据库核对/页面交互)
- coder 报告本身合规且质量合格(阻塞证据可复现,v2.0 按人工说明修订)

## 根因

两处模型错配叠加:
1. **跨仓 WI 的证据落位机制缺失**: plan 引擎允许定义跨仓验收 WI(scope 指向 cadence/verify/**),
   但没有为其提供独立落盘位置(独立 worktree / issue 级目录 / .aria 侧投影)
2. **逐仓确认校验只看单仓绑定 WI 的 scope**,不感知「该 diff 来自 plan 内其他 WI 的合法交付」

## 关联

上游诱因是《2026-10-16_观察-coder沙箱无运行时-CHECK验证系统性不可执行》:
正因沙箱跑不了运行时验证,跨仓 WI 才以「文件型验证报告」作为交付物,从而踩中本死锁。
若批2 落地受控验证 runner(证据回填 attempt 而非落盘文件),本 bug 的触发面同步收窄。

## 建议

1. 确认校验改为取 **plan 内全部 WI 的 scope 并集**(或对标记为跨仓/aggregate 的 WI 的产物放行),
   同时保持单仓 WI 之间的越权拦截
2. 或给跨仓 WI 独立证据落位(推荐): plan 创建时为 aggregate WI 指定 issue 级证据目录,
   不进任何成员仓 diff
3. 短期缓解: WI 拆分器对跨仓验收 WI 的 scope 生成加约束——要么绑定具体仓并在该仓 scope 内,
   要么明确「证据走 attempt 回填、不落盘」

## 处置记录

- owner 选择按 bug 上报,不 revert 两笔 docs 提交;确认按钮停留在失败态,作为批2 修复后的回归现场
- frontend worktree HEAD 保持 fbac52d(含证据文件),三仓功能代码均已交付
  (busi 204ca26/6880023, gateway 1555f83, frontend 57c0ae5)
