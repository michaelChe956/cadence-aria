# Proposal

## Why

E2E v1.2 的缺陷 #8 已证实：`AggregatePolicyArtifact` 仍可保存固定的 `BOOTSTRAP_POLICY_TEXT`，真实根 recipe 缺少把生成的规则正文发布为 canonical root 权威政策的产品通道，验收只能靠 fixture 物化补齐。现有 readiness 会把「桩正文 + 有效旧 receipt」当作就绪；必须同时补齐最终正文发布与显式迁移，避免投影已就绪而下游消费仍不完整。

## What Changes

- 将 LC 五步 root recipe 生成的根规则聚合为最终政策正文，在既有 `RuleAndMcpConfig` 产物流的收尾发布正文、原始 UTF-8 字节 SHA-256 digest 与 revision；canonical `provider_context_root` 下 `policy_id` 路径保存同一正文，LC scoped `aggregate-policy.json` 保存同一 artifact，最终 root recipe receipt 冻结该 policy digest 和根 rule digest。
- 退役固定正文作为最终政策的路径：保留既有 `ensure_bootstrap` 为受 bootstrap credential 约束的首次 recipe 提供临时材料，最终收口必须发布真正文；每次实际完成的新 recipe 发布新 revision，旧 operation/receipt 与 revision 正文保留审计。存量桩 LC 先 waiting，再由用户通过现有初始化产品 API 显式重跑（新操作身份）并重冻结；生产 AggregatePreflight 仅凭同 LC/root 的旧四命令 Allowed receipt 与现存根入口摘要证明复用已生成根，首次登记的用户文件冲突仍拒绝。
- **BREAKING**：readiness 唯一新增「artifact 正文等于已知 bootstrap 桩（或其 digest）」识别，命中后 `planning_ready=false`；allowed_actions 保留 `Retry`，detail/notice 指向既有初始化 API。非桩 artifact 的原有 operation、policy/receipt/rule digest、成员索引与聚合索引判定保持不变；其余 resolver、admission、gateway、envelope、bootstrap action dispatcher 与 UI 路由零改动。
- 加入无手工 seed/无 fixture 物化政策的 fresh-LC E2E：产品登记→五步 recipe→权威根正文、artifact 与 receipt 的 digest 一致→索引就绪→`planning_ready=true`；覆盖桩加旧有效 receipt 的等待/重跑、真正文就绪、发布失败与显式重冻结。
- 非目标：不新增第六步、第五条命令、第二套 durable operation、独立政策编辑/发布 API、成员规则复制或单仓行为变更；四家 provider 网关解锁和各自启动行为由 `lc-gateway-multi-provider` 交付，本 change 只提供其共享的政策事实。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `non-interrupt-repository-bootstrap`：在上游 BOOT-03/BOOT-04 五步与自举边界上增加根政策发布、receipt 收口、桩等待与显式重冻结 requirements；上游 change 尚未归档，采用独立 ADDED requirements，避免用不完整 MODIFIED 覆盖既有条款。
- `session-policy-envelope`：增加 canonical root 已发布正文与权威 artifact/frozen policy 引用一致的 requirement；保持 ENV-01 与原有启动、准入、resume 消费契约。

## Impact

涉及 LC 聚合初始化生产收尾、政策 artifact store、canonical root 正文发布、root receipt 冻结、readiness 的单一桩识别及相应测试/E2E。无新增外部依赖；传统单仓初始化、成员仓 Git 状态、LC authority 路由和下游网关消费结构保持现有契约。与并行网关 change 的接口为 `canonical root/policy_id`、`policy_id/revision/digest` 和 receipt 的 `policy_digest`。

### 存量迁移入口

Completed 存量桩 LC 的重跑入口为既有产品 API：`POST /api/projects/{pid}/logical-codebases/{lcid}/initializations`，请求携带新的 `idempotency_key`。现有“启动聚合初始化”UI 按钮仅在 operation 为 null、failed 或 cancelled 时可见，Completed 态没有该按钮；本 change 的等待说明必须如实提供 API 指引，迁移 E2E 实际调用该 API，不声称通用 bootstrap Retry 按钮能够重跑。

该 API 的可执行前提是 receipt 证明的根归属：同一 LC/canonical root 有四命令全 Allowed 的旧完成 receipt，`rule_digest` 与当前 `AGENTS.md` 一致，`CLAUDE.md` 若在场其字节 digest 与同一旧命令审计快照一致。此前无条件根所有权检查会把产品自己生成的入口误判为用户冲突；本 change 只补 recipe 生产侧的已证明重跑路径。没有 receipt、身份/摘要不符或用户文件冲突时仍拒绝，不删文件、不手工改 JSON、不静默迁移。

预计 **1–2 人日（8–16 小时）**，覆盖发布链、桩迁移与回归验收；design/tasks 将按现有文件边界拆成可独立提交的工作包。此处只提供规划产物，实施须另行审阅契约并形成已确认 Plan。
