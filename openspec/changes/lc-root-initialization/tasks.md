# Tasks

## 1. Phase 1：根 Recipe、自举凭据与 readiness（约 4–5 人日）

- [ ] 1.1 冻结 LC root-cwd 的契约模型与回归锁迁移边界，覆盖 BOOT-01 supersede 半句、REG-12 根规则来源、ENV-06/07/08 原生自发现与 pointer 边界，以及 D1–D4 的单仓不变声明；验证四份 delta requirement 均能映射到设计决策，旧单仓契约快照无语义变化。
- [ ] 1.2 实现仅由 durable Running operation 派生的 bootstrap phase credential、临时最小权限 policy 与 BootstrapExecutor 双向工具策略守卫；验证有效凭据只豁免根规则存在性，失效/伪造/漂移凭据、普通 Executor 带策略和普通 session 缺根规则均在 spawn 前拒绝，且 bootstrap 仍需 authority/capability/gateway 校验（REQ-BOOT-04、REQ-REG-12、D1）。
- [ ] 1.3 接入 Codex projects trust、Kimi workspace-trust 与撤销生命周期，在 AggregatePreflight 后、PreCheck 的首个真实 provider turn 前登记 canonical LC root，提供幂等、多 LC 隔离、用户已有 trust 归属保护、前后摘要和跨工作区副作用审计；验证 Codex 启动含 `--skip-git-repo-check`，trust 写入/撤销失败进入等待且不以降级 provider 继续（REQ-REG-14、REQ-REG-12、REQ-ENV-06）。
- [ ] 1.4 将聚合初始化五步 operation 接入四条 root recipe 命令的顺序、取消、超时、输出摘要、恢复与失败事实；验证临时 policy/bootstrap credential 与所选 provider trust 均有效后，四命令在 canonical LC root 各执行一次、信任或命令失败停止后续步骤，且单仓四命令与 GitFinalize 回归通过（REQ-BOOT-01、REQ-BOOT-03、REQ-BOOT-04、REQ-REG-14）。
- [ ] 1.5 建立 root recipe receipt 与 filesystem auditor 的生产事实边界，记录 root、step/command、allowlist、前后快照、产物摘要、policy/rule identity 与未知变更证据；验证用户冲突、symlink escape、成员 Git/worktree 变化、不可观测写入和越界路径均 fail-closed，且不覆盖或回滚用户文件（REQ-BOOT-03、D2、REQ-REG-09）。
- [ ] 1.6 实现 recipe operation 与 bootstrap projection 的 readiness 闭环，令 RulesPolicy 使用可解析的最终 policy 正文、独立根规则摘要及 receipt，MemberIndex 与 AggregateIndexActive 保持独立 checkpoint；验证 recipe Completed 但上述投影材料缺失、policy/rule digest 不一致或索引未 active 时 `planning_ready` 仍为 false，且只读查询不启动副作用（REQ-BOOT-03、REQ-REG-10）。

## 2. Phase 2：cwd、target、resume 与 provider 入口统一（约 3–4 人日）

- [ ] 2.1 扩展 session policy、validated launch、adapter input 与 resume fingerprint 的独立 `working_directory` 合同；验证 LC cwd 始终是 canonical root、target/worktree 保持独立、cwd 漂移使 resume superseded，单仓两字段仍映射旧目录（REQ-ENV-01、REQ-ENV-03、REQ-ENV-04、REQ-ENV-10、REQ-ENV-11）。
- [ ] 2.2 更新 gateway revalidation 与双工厂 root assertion，保留 policy/capability/config/target/git-dir/worktree/availability 全链，并移除 cwd 必须等于 target 的错误等式；验证 cwd≠target 合法 fixture 放行，authority 越界、非成员 target、Git identity 漂移与 root factory 不一致均 zero spawn（REQ-ENV-01、REQ-ENV-11）。
- [ ] 2.3 迁移 Author、ChoiceFollowup、Revision、ReviewOnly、Story/Design/Plan author 与 plan review builders 到 root cwd + 独立 target；验证首轮、follow-up、revision、legacy review 分支和 delegated review 均经正确 gateway/admission/resume 路径，作者/评审 built-in 写工具策略未放宽（REQ-PLN-01、REQ-PLN-03、REQ-PLN-07、REQ-ENV-09）。
- [ ] 2.4 迁移 split sync、Coder/retry、Group review 与 InternalReviewer 的 cwd/target 输入和 launch rebind；验证 Coder 唯一 writable root 仍是 attempt target worktree、Reviewer/plan 空写根、target resolver 仍校验 Git 身份，单仓 direct 入口不变（REQ-ENV-03、REQ-ENV-10、REQ-ENV-11）。
- [ ] 2.5 保持 planning snapshot 贯穿 context、cwd、prompt、audit、revision、resume 与 WebSocket follow-up；验证成员/checkout/policy/access fingerprint 漂移触发重建，不出现 `issue.repo_id`、first Story 或成员 cwd fallback（REQ-PLN-03、REQ-PLN-07）。
- [ ] 2.6 固化根入口与 skills 零改造结论：以 AGENTS.md 为通用入口、CLAUDE.md 为兼容副本，沿用机器级 managed links，不新增注入、复制或重链；验证四家 provider 的根规则/MCP/skills 发现来源与既有安装链一致（REQ-ENV-06、REQ-ENV-07、REQ-ENV-08）。

## 3. Phase 3：provider 证据、回归锁与终局验收（约 3–5 人日）

- [ ] 3.1 建立 cwd≠target 专用 gateway、resolver、resume 与缺根 admission fixture，保留旧同 cwd fixture 作为兼容边界；验证 root cwd + member target 放行、root/target 指纹变化拒绝、普通缺根 session 零 spawn（REQ-ENV-10、REQ-ENV-11、REQ-BOOT-04）。
- [ ] 3.2 重写 D1/D2/D3 回归锁并补 D4 跨目标基线：覆盖三 adapter tool-policy 双向 guard、root receipt allowlist/auditor、共享命令 executor 不可达单仓 registration/GitFinalize、LC coding/retry/review 前后 member baseline；验证所有未知 tool policy、成员写入、越界变更、单仓调用图和 baseline 漂移均按角色 fail-closed，且单仓行为零变化（REQ-ENV-09、REQ-BOOT-03、D1–D4）。
- [ ] 3.3 运行四家 provider 真实 CLI 矩阵并归档证据：规则/MCP/root-native skills、祖先 Git 污染、Codex anchor/config 限制、Codex/Kimi trust 前提与 AGENTS/CLAUDE 入口；验证已预研通过项不回归，未支持版本或证据不足时明确阻断而不增加注入 shim（REQ-ENV-06、REQ-ENV-08、REQ-REG-14）。
- [ ] 3.4 完成 provider-specific writable evidence gate 与大 LC/多版本验收；验证数十成员的上下文、token、启动时延预算，非 target 成员写尝试和成员主 checkout 快照，以及版本不兼容时的 fail-closed 结论；不得以 MCP 成功或 `git status` 单独宣称 OS 级隔离（REQ-PLN-06、REQ-ENV-03、D2、D4）。
- [ ] 3.5 执行全链 E2E 终局验收：全新非 Git LC 从准入、成员登记、AggregatePreflight 冻结 root、所选 provider trust 登记、四命令 root recipe、bootstrap projection、`planning_ready` 到 Story/Design/Plan/Coding/Review；验证 Author/ChoiceFollowup/Revision/Review 分流、cwd=root、member target、resume fingerprint、工具策略、D4 baseline、root receipt、成员主 checkout 零初始化写入和普通缺根零 spawn（REQ-BOOT-01、REQ-BOOT-03/04、REQ-REG-12/14、REQ-PLN-01/03/06/07、REQ-ENV-01/03/04/06/07/08/10/11）。
- [ ] 3.6 运行单仓对照与最终契约验收，确认单仓初始化命令/cwd/operation/GitFinalize、legacy provider direct 入口、role tool-policy、pointer 原有受控通道和 Kimi Aria bundle 语义均未改变；验证 `openspec validate lc-root-initialization` 与 `openspec status --change lc-root-initialization` 均通过且所有规划工件为 done（proposal、specs、design、tasks）。