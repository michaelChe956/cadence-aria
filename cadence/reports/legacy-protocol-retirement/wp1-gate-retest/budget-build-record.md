# WP1 退役门重测：超时预算+build 版本前置留档（budget-build-record）

> change ③ retire-legacy-workitem-protocol / WP1.1（REQ-RET-01 scenario「超时预算前置」+ 全局约束 7「F-14 修复后 build 前置」）。
> 本文件在**任何重测跑（测试面与真实 campaign）启动之前**落档（Step 1 先于 Step 2/3/4）。

## §0 build 版本与部署记录（v25）

| 项 | 值 | 证据 |
|---|---|---|
| build commit | `4d097c1d21a9`（= `4d097c1d`，**含 F-14 修复**「coding-ws runner pre-provider 死亡 fail-closed 转人工恢复」） | `GET /api/runtime-info` → `git_sha:"4d097c1d21a9"`；`git log -1 4d097c1d` = `2026-09-19 06:49:54 +0800 fix(coding-ws): F-14 …` |
| built_at | 2026-09-19 06:52:21 +0800（unix 1789771941） | `GET /api/runtime-info` → `built_at_unix:"1789771941"`；二进制 mtime `target/release/aria` = 2026-09-19 06:56:33 +0800 |
| package_version | 0.0.9 | `GET /api/runtime-info` |
| 部署代次 | **v25**（progress.md :574 登记；controller 部署） | `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/progress.md` |
| 启动时间 | 2026-09-19 07:23:24 +0800 | `ps -o lstart -p 1005268` |
| PID | **1005268**（本 Task 全程禁重启；T1 与 kimi 升级线共享 v25） | 同上 |
| 启动命令 | `target/release/aria web --workspace <worktree 绝对路径> --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate` | `ps -o cmd -p 1005268` |
| 健康检查 | `GET http://127.0.0.1:4317/api/health` → `{"status":"ok"}`（2026-09-19 ~09:29 复核存活，已运行 ≈2h06m） | 本表各行同刻实测 |
| worktree HEAD（重测时点） | `3b5260c7`（4d097c1d 之后仅 `14c4c734` 风格/日志补丁与 `3b5260c7` docs 提交；`git merge-base --is-ancestor 4d097c1d HEAD` 通过） | `git log 4d097c1d..HEAD` |

F-14 前置结论：v24（01:0x 部署）不含 F-14；现行 v25 二进制 `git_sha=4d097c1d21a9` 直接实证含 F-14——满足全局约束 7（重测 build MUST 构建自含 `4d097c1d` 的 HEAD）。判据零改不受 build 版本影响（约束 7 末句）。

## §1 超时预算（执行参数，非判据）

| 项 | 设定 | 依据 |
|---|---|---|
| 单案例超时上限 | `ARIA_WORKITEM_HARD_TIMEOUT_MS=1800000`（30min；驱动器 mjs:70 读入、:137 校验） | pi 先例全程需 1800s 上限（阶段 2 报告 §9.1）；codex 先例 142.96–627.14s 余量充分 |
| campaign 总预算 | 30min × 2（codex+pi 各 1 案例）+ 30min 缓冲 = **90min** | 本计划 Step 1 公式 |
| 失败重试策略 | 每案例失败先按 RR-3 定性：**首败事实留档 + 定向复跑 1 次 + diff 无交集佐证**；定性后修复重跑不设次数上限但逐次留档；不以复跑覆盖首败事实 | 全局约束 9 |
| 驱动器对齐环境 | `ARIA_EXPECTED_FLOW_KIND=single_candidate`（v25 服务器带 `--work-item-plan-single-candidate` flag，驱动器默认期望 legacy 须显式对齐——PVR 先例 evidence-matrix.md :51 登记）、`ARIA_RUN_POLICY=auto_if_valid`（驱动器默认）、`ARIA_BASE_URL=http://127.0.0.1:4317` | change ① 验证过的双驱动器链口径 |
| 并发环境声明 | v25 为四线并行共享（kimi 升级线 0556a410 收口可能同刻使用服务器）；重测期间本 Task 不主动引入 cargo/构建并发（测试面与 campaign 串行排布），如实记录观测到的第三方活动 | 派工约束「服务器勿重启（T1/kimi 线共享 v25）」 |

## §2 判据零改声明

矩阵 §1 判据列以 `openspec/specs/work-item-plan-single-candidate/spec.md:121-135`（REQ-WSC-07 正文+第二段多仓 preflight 条款）**原文为准，本 Task 不改 spec 任何字符**；不放宽阈值、不改计数口径、不删减子项（全局约束 2）。预算（§1）是执行参数，不与判据零改红线混淆（全局约束 8）。
