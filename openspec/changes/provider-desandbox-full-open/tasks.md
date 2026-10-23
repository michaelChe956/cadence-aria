# 高层工作包：Bug4+5 合并实施

本文件记录未来 apply 的高层工作包；本次仅创建提案，不执行、不勾选实现任务。精确文件、命令、测试与提交切分须由后续经审阅的 `cadence/plans/` Plan 展开，不以本清单替代实施 Plan。各组随实现交付本组定向测试、相应现有说明与 changelog 更新，共享文件由单一集成人控制。

## 1. 公共安全前置与受信执行合同

- [ ] 1.1 完成 REST 状态变更及 alias、WS upgrade、敏感 GET 的 Host/Origin 来源校验（控制面不要求入站令牌或会话），并交付对应拒绝/正常调用测试及安全边界说明；验收伪造/缺失 Host、非法/缺失 Origin 均在副作用前 403（受信来源下无会话即放行），localhost 无豁免，入站令牌不进 URL、日志或 provider 环境（REQ-API-AUTH-01/02）。
- [ ] 1.2 统一受信执行装配与宿主继承环境，覆盖各角色 Direct/LC、同步/流式及 ACP/MCP 子执行，交付跨入口装配测试及环境政策说明；验收自报 role/prompt/env 不可升级、HOME=用户宿主 HOME（子进程继承宿主环境，平台不构造不覆盖）、无 bwrap 仍可启动、审计明确非隔离（REQ-ENV-HOST-01、REQ-ENV-01/02、网关 projection）。
- [ ] 1.3 完成运行资源登记、退出/启动补偿及平台受信 Git 启动点加固，交付资源归属与 hooks/fsmonitor 探针测试、清理说明；验收先登记后执行、未知 writer 停等、仅清本轮登记私有资源、不按 HOME/XDG 递归清理，平台不会执行仓库 hooks/fsmonitor，`git_finalize` 子进程继承宿主环境并只叠加 `LC_ALL`/`HOME`/`SSH_AUTH_SOCK` 显式覆盖层，正常 Coder 提交保持可用（REQ-ENV-HOST-01、REQ-ENV-REVIEW-B）。

## 2. 全开能力记录、角色岗位与两阶段准入

- [ ] 2.1 迁移 action 能力证据为有版本标识的宿主全开语义，交付旧/新记录与重验证测试及迁移说明；验收旧只读/target-only Confirmed 不自动升级，新证据可追溯 exact version/投影，缺失维持 Unknown/Denied，历史记录不篡改（网关显式映射、projection、E2E；REQ-ENV-05）。
- [ ] 2.2 同步更新 gateway validate 与 spawn 复验、resume 指纹及 role-chain preflight，交付双阶段漂移零会话启动与显式 fresh 测试；验收新 write_boundary 两处同源消费、native identity/canonical cwd/trust/完整 fingerprint 保留、旧物理拒写门不复活、RootRecipe 独立凭据合同不误套（网关 admission/resume/版本观测、REQ-ENV-01/02/03）。
- [ ] 2.3 四家落实 author-only 岗位纪律与 Reviewer/Coder 全工具面，交付岗位、审批、持久审计测试与角色说明；Pi/Claude/Kimi 的工具限制与 Codex 检测型 fileChange 纪律分别验收，Codex 事件出现后须中断 turn、失败关闭并 durable 审计，不承诺 danger 下预防，不禁 terminal/MCP/skills（REQ-ENV-09、REQ-PLN-01/06、网关角色守卫）。
- [ ] 2.4 完成 Codex LC 全相位 danger-full-access；Direct 同步按“诉求1的推论”接入，并保留独立审批。固定真实 CLI 版本分别运行 danger+apply_patch 的 on-request/never 探针，保存审批出现/未出现、fileChange、实际变化、中断与审计原始证据；不能观察或中断即岗位机制验收不通过，不写 Decline 强制（REQ-LCG-04、REQ-ENV-09）。
- [ ] 2.5 删除 require_write_boundary 的 Unknown+旧 supported_actions 放行臂并适配测试；验收 validate、spawn 复验与其它直接消费仅当前语义 Confirmed 放行，legacy 必须真实重探针铸证，无迁移默认 allow，Denied 保留诊断（网关显式映射/admission/projection，已知偏差 DEV-01）。

## 3. Kimi 全角色执行通道

- [ ] 3.1 统一 Kimi 主会话与 terminal 全角色 Host 单路径，删除 bwrap/只读 grammar/固定解释器及角色整体拒绝旧行为，交付普通 argv/legacy shell/规划角色/无 bwrap 测试与用户说明；验收 Auto/Supervised 仅差审批，author 岗位工具与 Reviewer 窗口独立（Kimi OS 隔离、grammar、权限与 fs 三条；集成 ACP/权限/逐工具审批三条）。
- [ ] 3.2 完成 Kimi fs API 路径与 author 岗位配套，交付 no-follow/父目录竞态/授权根越界和 Reviewer FsWrite 正向测试及边界说明；验收 fs API 仍限授权根，但不冒称 terminal 物理不可越界，Splitter/Handoff 不整体拒绝（Kimi 权限与路径沙箱、REQ-ENV-09、REQ-PLN-06）。
- [ ] 3.3 解除独立 120 秒终端上限并将真实活动终端接入 ACP stall 与产品 idle 生命周期，交付跨 120 秒静默 CHECK、总 deadline/取消、并发及确定性双流预算测试与超时说明；验收真实活动不误杀、无工作 stall 保留、4 终端/1 MiB/幂等清理不缩水（Kimi 终端生命周期与资源边界、REQ-ENV-CHECK-01）。

## 4. 四家 Reviewer B 档闭环

- [ ] 4.1 交付可恢复基线与平台通道互斥的 durable 审查窗口，覆盖四家受支持的 Reviewer 入口并提供快照矩阵、并发通道测试与窗口说明；验收先锁并停 writer 再持久化基线/采材料，基线范围恰为 tracked 与 untracked 非 ignored（ignored 产物不采集、不进基线），无有效窗口或未接线 provider 不启动（REQ-ENV-REVIEW-B、REQ-ENV-09、网关 admission）。
- [ ] 4.2 完成恢复前违规落盘、三域对账与报告独立采信，交付对账域内构建残留、ignored 产物不触发违规且不被恢复删除、改写 `.gitignore` 不得缩小对账域、自提交/.git/.aria 越权、保留 Coder dirty、恢复成功仍作废的测试与风险说明；验收对账域内违规整轮不可用于交付、不可由恢复洗白，ignored 产物改动不入基线与违规判定且报告/审计面显式披露该边界，对账域内临时验证变化恢复且只在完整无违规时采信（REQ-ENV-REVIEW-B）。
- [ ] 4.3 接通取消/超时/崩溃补偿、人工恢复与重试新轮，交付启动补偿、未知 writer、journal 不一致及技术重试测试与运维说明；验收恢复未完成不释放冲突写通道、不采污染基线，违规必须新轮，预算不因新窗口增加，不复活旧只读重试（REQ-ENV-REVIEW-B、coding-provider-transport-retry）。

## 5. Coder CHECK 主路径与真实集成验收

- [ ] 5.1 完成 HostCoding 自跑 CHECK 的 prompt/证据/失败分类与 A′ 补充说明，随交付定向验证流测试及现有使用说明/changelog；验收计划与实际命令/cwd/退出码/数量可核对，环境声明非授权，重跑仍唤回 Coder，不新增服务端通用 runner（REQ-ENV-CHECK-01）。
- [ ] 5.2 执行四家×角色、Direct/LC、fresh/resume/retry 集成验收，记录当前 CLI/工具链版本与真实证据，覆盖 Kimi Maven/npm/Podman、静默长 CHECK、Codex author 检测型真机探针、Reviewer 自提交作废/崩溃恢复和鉴权拒绝；前提缺失如实记未完成，不 skip 后宣称全通（网关四家 E2E、全部新增需求）。
- [ ] 5.3 完成契约与实现一致性审查并复跑本 change 的 strict 校验；验收旧限制测试按对应行为反转/替换而非放宽保护断言，所有五点 delta、B 档三要素、HostCoding CHECK 和无条件鉴权均有证据，未实现工作包不勾选（全部 requirements）。
- [ ] 5.4 【归档硬门】在本 change 归档前，必须完成 anchor-recovery 的依赖门修订：该 change 排在去沙箱③之后，Codex read-only/workspace-write、旧角色 Reviewer deny 与物理 write_boundary 前提按本全开契约修订，并核对同 capability 的 requirement/header；交付修订审阅记录与两个 change 的 strict 结果，否则禁止归档本 change，不得以 supersedes 注释代替（design.md 同 capability 冲突面、REQ-LCG-04/06、REQ-PLN-01/06）。
- [ ] 5.5 【归档前后措辞门】对主规范执行失效措辞扫描并人工判读：不能残留规范性“永久禁止 Codex danger-full-access”“宿主写成功即能力失败”“Reviewer 必带 deny”等旧结论；核对 PLN 与 anchor-recovery 的 header 无覆盖冲突，交付扫描结果及归档后主规范 strict 证据（五处契约变更）。

## 后续工作流

- 契约审阅通过后才展开实施 Plan 和 apply；本次工件创建不代表任何源码行为已实现。
- 原网关 change 的基线归档由 controller 执行；本 change 网关仅 MODIFIED。anchor-recovery 的冲突先修订、后按去沙箱③之后的依赖排期实施；归档前必须通过 5.4/5.5 硬门，不能把未开工 anchor 合同提前搬入本 change 或写成已完成。
- 二期 dev-sandbox 的启动由 owner 自行安排，不属于本 change 必做实施项。
