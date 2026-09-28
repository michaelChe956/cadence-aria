# Tasks

## 1. 唯一路由与身份事实

- [ ] 1.1 实现并接入单仓/逻辑代码库统一 authority resolver，输出稳定 target、manifest/checkout、selection、policy、member index 与 aggregate index 引用；对旧布局、路径越界、重复候选和来源身份冲突 fail-closed（对应 `logical-codebase-registration` REQ-REG-01、`codebase-kinds`）。
- [ ] 1.2 将 resolver 的 authority、revision/digest 和冲突诊断接入现有查询、登记、规划/策略读取入口，确保各 capability 不再各自猜测路径；保持登记零成员仓 Git 写副作用（对应 `logical-codebase-registration` REQ-REG-01/02、`project-rule-aware-prompts` REQ-PROMPT-02、`session-policy-envelope` REQ-ENV-01）。

## 2. LC 冷启动与聚合索引

- [ ] 2.1 在既有 registration/index operation 上补齐 identity→manifest/checkout→rules/policy→member index→aggregate active 的可重入 checkpoint、步骤状态、失败原因、稳定命令键和 expected revision，解除 active index 首建准入的鸡生蛋（对应 `logical-codebase-registration` REQ-REG-10、`logical-codebase-aggregate-index` 聚合索引生产触发）。
- [ ] 2.2 将只读详情/成员/policy/index 查询收敛为纯投影，并为安全继续、准备、重试和核验提供统一产品动作；失败保留 last-known-good/degraded 或 missing/failed 事实，成功前不得伪造 active（对应 `logical-codebase-registration` REQ-REG-10/13、`logical-codebase-aggregate-index` 构建状态与失败恢复）。
- [ ] 2.3 将成员 index 与 aggregate index 构建的快照、single-writer、范围验证和 PlanningReady 推进接入同一 authority，保证冷启动无手工 seed、已完成 provider turn 不重复且成员仓无 Git 写副作用（对应 `logical-codebase-aggregate-index` 聚合索引生产触发/构建状态）。

## 3. Identity journal repair

- [ ] 3.1 提供独立于普通成员列表的 Failed identity journal 诊断投影，展示 source digest、read mode、completed keys、候选 mapping、冲突和影响范围；保留原失败事实及审计（对应 `logical-codebase-registration` REQ-REG-11）。
- [ ] 3.2 实现安全 prefix 继续与 mapping 提交后再次核验的受限 repair 动作，使用稳定命令键和版本检查；冲突未裁决前不得切换 authority、修改权威 JSON、删除 journal 或启动 provider（对应 `logical-codebase-registration` REQ-REG-11/13、`codebase-kinds`）。

## 4. Rules/policy/capability 准入

- [ ] 4.1 在真实 provider/index/规划准入前校验实际消费的规则材料、聚合 policy revision/digest 与 capability/gateway 条件，产出可审计等待项和准备/重试动作（对应 `logical-codebase-registration` REQ-REG-12、`project-rule-aware-prompts` REQ-PROMPT-01/02）。
- [ ] 4.2 将唯一 resolver 的 authority/policy 引用接入 SessionPolicyEnvelope，缺失或不一致时在 spawn 前 fail-closed；禁止伪造 capability、回落旧路径或绕过 gateway，准备完成后回到原链（对应 `session-policy-envelope` REQ-ENV-01）。

## 5. 通知、恢复与验收

- [ ] 5.1 将步骤失败、repair 冲突、材料缺失和操作结果接入既有 inbox/系统通知与补读投影，展示对象身份、已完成步骤、原因、副作用、按钮语义和下一阶段；REST 与页面动作复用同一应用服务（对应 `logical-codebase-registration` REQ-REG-13）。
- [ ] 5.2 覆盖 A03 LC 冷启动与 A04 identity/rules repair 故障注入及恢复路径，验证无手工铺底、无直接改数据、无服务重启、mapping 未确认不切读、同一命令不重复 provider turn；与 C1 并行交接 resolver/policy 合同。
