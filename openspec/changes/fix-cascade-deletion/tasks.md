# Tasks

本文件只列高层工作包。精确实施步骤、独占文件、测试命令及串行写面窗口见 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/plans/2026-10-13_实施计划_级联删除_v1.1.md`（保留v1.0原稿）。本轮吸收接地与一致性审查，不实施、不commit；复选框待真实实施/独立审查/验证后勾。

## 1. R：报告身份先行移交（外部依赖，不重复实现）

- [ ] 1.1 R 消费 FixReportId 的同 payload 幂等、异报告新身份、两 writer 同规则、旧 immutable 严格校验的提交/独立审查与 Main 验证证据；明确 SG3a.0/source store/两 writer/fixture 串行移交窗口（保护 REQ-GRP-01、REQ-DEL-06，R 本身不属删除新功能）。

## 2. D1：拥有闭包、耐久事实与存储准入

- [ ] 2.1 D1.0 单条无内容purging/deleted记录、非阻塞try-shared write/try-exclusive delete、稳定repo resource准入、身份list-by-owner；pending manifest支持重入且成功移除，真实错误不吞（REQ-DEL-02/06/07）。
- [ ] 2.2 D1.1 实现真实 WI/session/review/run/专属配置 manifest 与子树先删/根最后的 store 接口，纳入全局session split-run/tool-policy审计分区，精确识别 incoming refs/共享归属；source整Plan purge在D1.2a落地（REQ-GRP-01、REQ-DEL-01、REQ-DEL-03、REQ-DEL-07）。
  - 进度注记（2026-10-14，ImplD11）：store 原语已实现并红→绿→回归（`cascade_d1_1_real_owner_manifest`/`cascade_d1_1_audit_dependency_order`，`lifecycle_store::tests` 109/0，全量 4610/0）；manifest 已消费 D1.0 owned-index 与 D1.2c list/purge_plan_artifacts；repository_profile 归属按 D1.3a 裁定入 manifest；D1.3b 关切②（save_work_item_split_provider_run 无归属字段）按 §0.5 refs 归属处置并登记。source/revision/engine-checkpoint purge 分别留待 D1.2a/D1.2b/D2.1d+D2.2 接线（报告 `cadence/reports/D1.1-report.md`）。复选框按本文件规约待独立审查后勾。
- [ ] 2.3 D1.2a/b/c 接入source/draft/compile/revision/publication/advance owner准入及完整purge；advance record/journal/retry精确plan归属，删除身份优先于旧幂等与Ready放行，不变未删对象hash/join/CAS（REQ-GRP-01、REQ-DEL-06/07）。
- [ ] 2.4 D1.3a/b/c 生命周期/session/版本/review/run/门操作/checkpoint/审计准入与shared-worktree中央writer guard；按attempt/WI/journal反推Plan，不改原确认CAS/lock/helper，接线遵scope先于旧session/object锁（REQ-GRP-01、REQ-DEL-03/06/07）。
- [ ] 2.5 D1.4 单调保存 ever_confirmed，生产/测试构造闭包同提交迁移；新路径可证未确认零提示，旧历史未知保守一次确认，不改变确认 CAS（REQ-DEL-04）。
  - 进度注记（2026-10-14，ImplD14）：三类 record `ever_confirmed:Option<bool>`（serde default/skip_none）＋`LifecycleStore::classify_entity_deletion_confirmation` 三分类（NotRequired／Required{PreviouslyConfirmed}〔current Confirmed｜ever=true｜Plan 发布日志在档｜Story/Design confirmed_by 版本〕／Required{LegacyHistoryUnknown}）已红→绿→回归（`cascade_d1_4_confirmation_history_classification`；`lifecycle_store` 140/0、`conversational_gate` 146/0、全量 lib 4616/1/7i——唯一失败 `campaign_stage3_turn_reservation_crash_recovers_exactly_once` 归因 SG6a.1 契约迁移消费方回归〔基线 9596b68e 单跑 PASS、主树 FAIL，对方已认领登记并直报 Main〕；it_product 209/1 唯一失败 ambiguous_push 为基线既有）。确认/commit/request_change/restore/append_version(confirmed_by) 原写单调置 true，确认 CAS 零改动；16 个构造点机械闭包同提交（含 Task Files 清单外 `src/web/handlers/workspace_human_action.rs` 与 `tests/it_product` 三 fixture——Step 3 机械 literal 闭包要求，报告注明）；候选 parser（parse/revision/validate）内存 record 保持 None。模型/分类交 D2.2，原 SG fixture 回交。报告 `cadence/reports/D1.4-report.md`。复选框按本文件规约待独立审查后勾。

## 3. D2：协调、运行隔离与一次确认

- [ ] 3.1 D2.1a/b/c/d manager/REST/provider/compile/advance/retry恢复真实入口持共享run lease；coding创建与删除互斥，Deleted advance/retry拒绝且旧Ready false，旧Arc/晚写无权；写面点名双向SG移交（REQ-GRP-01、REQ-DEL-05/06/07）。
- [ ] 3.2 D2.2 三类DELETE统一门禁/manifest及服务端分级确认：coding/ref/busy/未确认拒删零副作用，两Plan路径含advance完整purge、target专属清理只读消费现worktree helper；失败重入与单条事实（REQ-GRP-01、REQ-DEL-01/02/03/04/05/07）。
- [ ] 3.3 D2.3 所有既有 Story/Design/Plan 删除入口消费服务端确认分类，Draft 零弹窗、Confirmed/未知旧历史恰一次，取消零删除、错误提示先删后置对象（REQ-DEL-04、REQ-DEL-05）。
- [ ] 3.4 D2.4 Main 统一执行端到端与无误伤回归、独立终审、owner 审阅；仅在全部契约满足后更新主规格/归档，不把 strict 格式通过当实现已完成（REQ-GRP-01、REQ-DEL-01 至 REQ-DEL-07）。
