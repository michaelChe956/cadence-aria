# Tasks

## 1. Capability 与 gateway 合同

- [x] 1.1 扩展四家 provider 的显式 ProviderRefType、ProviderDialect、wire dialect、registry adapter 与未知/Fake 拒绝路径；统一 LC 同步和流式入口使用 validated gateway policy（REQ-LCG-01、REQ-LCG-02）。
- [x] 1.2 版本化 capability durable 记录，加入 action×resume×write_boundary 三态、exact version、证据引用、provenance、projection digest 和旧记录 Unknown 迁移语义（REQ-LCG-01）。
- [x] 1.3 将 canonical root cwd、独立 target、policy projection、trust、availability、D4 freshness 纳入 admission 与 spawn 前复验，并关闭 LC 裸 input/legacy fallback（REQ-LCG-02、REQ-LCG-03）。

## 2. Provider 权限与安全投影

- [x] 2.1 为 Claude Code、Pi、Kimi Code 建立真实 CLI/RPC/ACP projection：角色工具策略、permission、MCP/config、trust、cwd、target 与 writable_roots；Kimi 保持 ClientServicePolicy 独立控制面（REQ-LCG-03、REQ-LCG-06）。
- [x] 2.2 实施 Codex 受限安全模型：永久阻断 danger-full-access；验证 read-only/on-request 规划评审与 workspace-write、protocol cwd=target、trust、D4、target-only boundary 的 Coding 组合；证据不足保持阻断并修订 REQ-ENV-05（REQ-LCG-04）。
- [x] 2.3 完成四家 boundary probe 与 projection 审计，覆盖 target 正向写、root/非 target/.git/.aria 负向写及只读物理边界；未证明项保持 Unknown/Denied（REQ-LCG-03、REQ-LCG-07）。

## 3. Tool policy、role-chain 与 resume

- [x] 3.1 对齐 Claude/Codex/Pi 的 spawn 前双向 tool-policy guard 及 Kimi 通用策略拒绝，保持作者/评审 deny 与 Executor/Coder 无通用 deny 的 D1-D4 语义（REQ-LCG-06）。
- [x] 3.2 让 automation_gateway_preflight 与 gateway 使用同一 role/provider/action/capability/trust/projection/boundary 判定，聚合全部违规项并禁止 gateway_required=false 旁路（REQ-LCG-06）。
- [x] 3.3 实现 Claude、Codex、Pi、Kimi 原生 resume 适配与 action 级 resume capability；扩展 policy/cwd/target/tool/projection/trust/MCP/evidence fingerprint，漂移时 supersede、零 spawn、不得静默 fresh（REQ-LCG-05）。

## 4. LC 端到端验收与交付门

- [x] 4.1 建立四家 provider × Story/Design/Plan/split/Coding/Review × fresh/resume 的真实 LC E2E 矩阵，记录 argv/RPC/ACP、cwd/target、角色、审批、工具调用与 capability/projection/trust digest（REQ-LCG-07）。
- [x] 4.2 为 Coding 与只读 action 采集 D4、target 正向写、root/非 target/.git/.aria 越界负向写、pre/post snapshot；每个失败场景证明 provider 零启动，并将缺证据格保持 Unknown/Denied（REQ-LCG-07）。
- [x] 4.3 在 policy authority-root 正文发布通道（缺陷 #8）未交付时维持 admission fail-closed；完成四家单仓 direct 对照、现有 Claude-only recipe 不回归和最终 capability/审计报告（REQ-LCG-02、REQ-LCG-07）。

## 5. 兼容性与收口

- [x] 5.1 校验单仓 direct topology、参数、cwd/target 映射、GitFinalize、既有输出契约和多 target 人工 StartCoding 红线零回归（REQ-LCG-02）。
- [x] 5.2 以当前 HEAD 已落地的 cwd-target、trust、D1-D4、预算门为基线，更新能力迁移/等待面与运维证据；不将多版本 CLI、OS launcher 或大 LC 成本假设为默认支持（REQ-LCG-01、REQ-LCG-03、REQ-LCG-04）。
