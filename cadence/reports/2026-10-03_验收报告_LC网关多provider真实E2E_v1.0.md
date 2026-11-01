# 验收报告:LC 网关多 provider 真实 E2E(草稿 v0.1——证据盘点骨架,未定稿)

> 状态:claude/codex/pi 三家已按 Ruling 14 收官;kimi 正式三轮待工作树 WIP 提交后执行(kimi-11 参考轮 10/10)。本文件为 Task 12 汇总面的证据盘点骨架,终稿待四家全收官+Task 11/12 现场证据齐后落。

## 1. 四家矩阵终局(Ruling 14 口径:10 可支持格全 Confirmed×连续 3 轮+2 不可支持格产品级证据)

| Provider | 收官轮 | 形态 | 证据根 |
|---|---|---|---|
| claude-code | r59/r60/r61 | capture 7148s+resume 2274s+3053s | `cadence/reports/lc-gateway-multi-provider/claude-code/matrix/`(snapshots 20261008T130537Z 链) |
| codex | codex-7/8/9 | capture 2960s+resume 447s+529s(后两轮与 kimi 并行) | `.../codex/matrix/` |
| pi | pi-8/9/10 | capture 2927s+resume 857s+610s | `.../pi/matrix/` |
| kimi-code | **待正式三轮**(参考:kimi-11 混合树 10/10) | — | `.../kimi-code/matrix/` |

2 不可支持格(四家一致的产品级证据):
- review resume:`INVALID_MESSAGE_FOR_STAGE: message request_revision not allowed in stage completed`(产品无「评审续开」功能;用户裁 A:未来 change)
- split_sync resume:`sync split 无 native session contract:resume=Unknown,零 spawn(新增 provider_start=0)`(计划冻结语义)

## 2. 过程修复台账(累计 26 项,全部 TDD 钉死;详见 SDD 台账 progress.md r53-kimi-11 段)

**产品缺陷(12 号前含 13 项)**:①supersede 锁不回收 1faa43f9 ②sync 桥 join 冻死 fa39b9b8 ③CLI 死亡不传播 66262588+2bcaeff2 ④SC 修订 compile 缺 target fe55ae03 ⑤同 cycle 二次计费 eb22da67 ⑥policy_invocation None 臂 143e5db1 ⑦组入口 binding 过滤 c6380034 ⑧分诊门不发帧 3e48f689 ⑨allowlist 备份 a1b5b65f ⑩聚合初始化失败不落终态 97f9f607 ⑪死亡路径静默不发帧 6bbe62b7 ⑫多仓 Design 路由歧义 35fa5d16 ⑬kimi Bash cwd 全拒 88f66f99
**AI 方差系统性消除(Ruling 11 系列)**:compile 清单 e8370425/AC 域 54923ecd/feedback 清单 15090d40/worktree 纪律 5d41f438/nonce 归一 aac1bfcc/design 钉定 e78a97c6/plan intent 教学 6a2e1207+39bbe0b0/recipe scope 纪律线 19e784c9
**夹具/harness 族**:bare origin d6cd16c2/spec 恢复 0ab2b72f/深掏 F1-F5 22146ce7/对话形状三修 9cafa49b/重挂判据 e12ace50/codex 判据映射 52ca7607/语义应答挤出 b44a44ae/聚合根复位 9b5aab99/trust 收窄 46cd5465(并行基建)
**静态审计 2 轮**:恢复面 8 发现(local://resume-deep-audit.md)+对话形状 4 发现(local://dialog-shape-audit.md)

## 3. 并行基建(oracle×max 三轮互审定型)

- 分层:capture 串行段→阶段屏障(skills 共享面)→resume 并行段
- 五硬前提全落地:看门狗 v3(scripts/lcg_matrix_watchdog.sh)/两阶段启动器(scripts/lcg_matrix_batch.sh)/批原子/env 隔离/trust selected-only
- 实战验证:codex-8/9+kimi-8 并行 resume 零异常

## 4. 待补(终稿前)

- [ ] kimi 正式三轮(1/3~3/3)
- [ ] Task 11 现场(失败零 spawn/越界写/D4 四家矩阵)
- [ ] Task 12 两类 direct 对照(sync direct 两槽/workspace streaming legacy 四家)
- [ ] #8 交付门校验(canonical_root/policy artifact/receipt 链)
- [ ] 四家 40 主格交叉表(每格 state/reason/evidence_ref)
