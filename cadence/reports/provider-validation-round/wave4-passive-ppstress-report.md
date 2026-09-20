# wave4-passive-ppstress 报告：pre-provider 死因压力测试（后台，10 连跑）

- 工作树：`.worktrees/feat-b-0808-add-monorepo`
- 服务器：v30 PID **3305925**（全程未重启；压测前后 `/api/health` ok，etime 17:32→32:03）
- 执行窗口：2026-09-20 10:13:49Z → 10:15:0xZ（约 80s，含 setup）
- 驱动与证据：`wave4-ppstress/ppstress-drive.cjs`、`wave4-ppstress/ppstress-20260920101349418.jsonl`（逐帧 WS 日志）、`wave4-ppstress/ppstress-summary-20260920101349418.json`（10 跑汇总）

## 1. 结论（10/10 死亡，死因串已捕获）

**10 次全部复现 runner pre-provider 死亡，10/10 经 F-16 双通道落死因；F-14 fail-closed（转 AwaitingManualRecovery）10/10 生效，无一次静默死循环、无一次挂在 Running。**

- 死因稳定码（attempt `manual_recovery_reason`）：`coding_runner_failed_while_running`（10/10）
- 死因原始串（F-16 durable 尾帧 `coding_manual_recovery_diagnostic_0001.json` 的 `metadata.failure_detail`）：

  ```
  product_store_not_found: coding author provider Fake
  ```

  （10/10 同串。来源：gate 过期 auto-continue 后 runner 走 `provider_for(state, Fake, "coding author provider")` → 生产 registry（claude_code/codex/pi/kimi_code，无 Fake）→ `ProductStoreError::NotFound` 上抛 → task.rs F-14 分支。）

- eprintln 通道（同源同串，`error.to_string()` 一份两投）：v30 stdout 指向 controller 终端 `/dev/pts/1`，本轮不可回收读取（不作终端输入窃取）；按 task.rs:200 格式重建为
  `[aria-runner-death] coding runner failed while running trigger=pre_provider_failure project_id=project_0001 issue_id=issue_0296 attempt_id=<id> reason=coding_runner_failed_while_running error=<上串>`，
  供 controller 在其终端 scrollback 对照（durable 尾帧已逐字节捕获同值）。

## 2. 方法（复用 coding-drive 模式，触发概率最大化）

每跑（issue_0296 / work_item_0001，串行 10 跑）：

1. HTTP `POST .../work-items/work_item_0001/coding-attempts` 建 attempt（`created`/`prepare_context`）；
2. WS attach（scoped coding-attempts 端点）+ `coding_hello` + `start_coding` → runner 拉起（`running`/`worktree_prepare`）；
3. runner 开 coding stage gate（`coding_stage_gate_0001`，kind=stage_gate，5s 倒计时）→ **不确认，等自然过期**（auto-continue）；
4. gate 倒计时期内 `provider_select {role:"coder", provider:"fake"}`（WS 转发 runner，期内应用并持久化，ack=`coding_provider_config_updated` 10/10）——provider=fake 保证 runner 必然穿过「gate 过期 → provider 启动」窗口且零真实编码、零 token 成本（任务授权：pi 或 fake 即可）；
5. gate 过期 → `provider_for(Fake)` 失败 → F-14：eprintln + durable 尾帧 + 转 `awaiting_manual_recovery`；
6. 观察 0.5s 内 AMR 落定 → HTTP abort 收尾释放 work item → 下一跑。

种子：`POST /api/projects/project_0001/issues` 建 issue_0296（repo=repository_0001/naruto，基线 main@954efab）+ 手工 seed `work-items/work_item_0001.json`（`plan_status=confirmed`、`require_execution_plan_confirm=false`，legacy 单仓路由）——无 LLM 参与，attempt 创建走产品 HTTP 面。

## 3. 10 跑明细

| run | attempt_id | gate 观察 | gate 过期 | provider_select ack | 过期→AMR | 终态（abort 后） | 死因串 |
|---|---|---|---|---|---|---|---|
| 1 | coding_attempt_8a9ef5d7e13… | Y | 5.03s | Y | 0.5s | aborted | product_store_not_found: coding author provider Fake |
| 2 | coding_attempt_4fa9da0b6aa1… | Y | 5.02s | Y | 0.5s | aborted | 同上 |
| 3 | coding_attempt_e77301a0a542… | Y | 5.02s | Y | 0.5s | aborted | 同上 |
| 4 | coding_attempt_58e682216afb… | Y | 5.04s | Y | 0.5s | aborted | 同上 |
| 5 | coding_attempt_389d87a85102… | Y | 5.06s | Y | 0.5s | aborted | 同上 |
| 6 | coding_attempt_04e03ec6cbbe… | Y | 5.03s | Y | 0.5s | aborted | 同上 |
| 7 | coding_attempt_21476d4a060f… | Y | 5.05s | Y | 0.5s | aborted | 同上 |
| 8 | coding_attempt_f37fadc14644… | Y | 5.04s | Y | 0.5s | aborted | 同上 |
| 9 | coding_attempt_041b8fb84c01… | Y | 5.02s | Y | 0.5s | aborted | 同上 |
| 10 | coding_attempt_1f2e3c29f60b… | Y | 5.03s | Y | 0.5s | aborted | 同上 |

Durable 复核（独立于驱动，直读数据面）：

- `issue_0296/coding-attempts/coding_attempt_*/chat-entries/coding_manual_recovery_diagnostic_0001.json` ×10，`failure_detail`/`manual_recovery_reason` 与 WS 观测一致；
- `stage-gates/*.json` ×10 全部 `status=expired`（自然过期，非确认放行）；
- attempt JSON ×10 终态 `aborted` 且保留 `manual_recovery_reason=coding_runner_failed_while_running`；
- `work-item-attempt-locks/` 清空（abort 释放干净，10 跑串行零阻塞）。

状态时间线（10/10 同形）：`created/prepare_context → running/worktree_prepare → awaiting_manual_recovery/worktree_prepare`（死于 provider 启动前，stage 未推进到 coding 落盘——与「pre-provider 窗口」定义一致）。

## 4. 定性：本次捕获 vs 历史「pre-provider 间歇死」

- **本次 10/10 是「构造性死亡」**（provider=fake 必然 NotFound）：它**最大化触发概率**地遍历了历史死亡窗口（attach→gate 过期→provider_for），并证明 **F-14 fail-closed 与 F-16 死因双通道在生产 v30 上端到端工作**（转换 0.5s、死因串 durable 可考、eprintln 同串上屏）。
- **历史上 v25/v27 的「间歇死」原始形态（真实 provider 下偶发）本轮未单独复现**——本压测的任务口径为触发+捕获（fake 即可），间歇形态的死因是否与 `provider_for` 同族（如 registry/gate/host 就绪竞态）仍待真实 provider 长跑观察；F-16 通道已在场，后续自然触发即可直接取证。
- 未观察到第二种死因串（无 `coding_event_channel_closed`、无 worktree materialization 失败、无 attach 重启死循环）；attach 分叉/连接关闭形态本轮未注入（属 F-18 面验证，非本线范围）。

## 5. 环境与副作用登记

- 服务器 v30（PID 3305925）未重启；压测后 health ok；并发在场的 F-24 验证线未受影响（本线独立 issue_0296，数据面零交集）。
- 数据面新增：issue_0296（压测 issue，10 个 aborted attempt + durable 死因证据，保留作证据链）；naruto 新增 worktree `.worktrees/aria-issues/issue_0296`（分支 `aria/issues/issue_0296`@954efab，与既有 aria-issues worktree 同款产品足迹；worktree 无未提交改动——runner 死于任何写路径之前）。
- 零 provider 进程启动（fake 在 registry 即 NotFound，未到达 spawn），零 token 成本。

## 6. Commit 文件清单

- `cadence/reports/provider-validation-round/wave4-ppstress/ppstress-drive.cjs`（驱动）
- `cadence/reports/provider-validation-round/wave4-ppstress/ppstress-20260920101349418.jsonl`（逐帧 WS 日志）
- `cadence/reports/provider-validation-round/wave4-ppstress/ppstress-summary-20260920101349418.json`（10 跑汇总）
- `cadence/reports/provider-validation-round/wave4-passive-ppstress-report.md`（本报告）
