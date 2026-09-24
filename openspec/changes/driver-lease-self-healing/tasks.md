# Tasks: driver-lease-self-healing

**工作包性质**：只登记高层工作包与验收口径；精确步骤由 `superpowers:writing-plans` 展开。

**全局边界**：不放宽 observer 写拒绝；不引入连接间争抢（holder=Some(活跃) 仍拒）；打点不参与仲裁状态机；B 方向按 oracle 裁决增补或缓行。

## 1. 写时自愈（后端）

- [ ] 1.1 仲裁层对 holder==None 的 Driver 首写放行并原子重授租约（幂等）；红绿用例：悬空自愈/活跃偷窃者仍拒/observer 写拒不变（REQ-DLS-01）。
- [ ] 1.2 既有 lease 测试族（lease_takeover_and_stale_write_rejection 等）语义复核与零回归。

## 2. 前端单次自动重试

- [ ] 2.1 STALE_DRIVER_LEASE 后自动重发 hello+单次重放原写命令；恰一次护栏（二次失败回退手动面）；红绿用例：无感恢复/不循环（REQ-DLS-02）。

## 3. 租约可观测（后端）

- [ ] 3.1 lease-diagnostics.jsonl append-only 打点（六事件枚举+滚动上限）+只读诊断端点（当前持有者+最近转移序列）；打点失败零影响；红绿用例：偷窃可定案/打点失败不影响仲裁（REQ-DLS-03）。
- [ ] 3.2 STALE 错误面引用最近转移事件（前端消费诊断端点数据）。

## 4. B 方向与门禁收口

- [ ] 4.1 oracle 裁决落地：并入则增补 delta（observer attach 不偷/回滚恢复 driver/测试语义登记）并实施；缓则登记观察项。
- [ ] 4.2 change strict 校验+lib/it_core（lease 族）/it_web 按触及面全量；F-50-1 现场复现路径回归（多 tab 轮询偷窃→自愈无感）。
