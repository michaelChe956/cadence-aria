# Tasks: add-provider-run-boundary

技术方案：`cadence/designs/2026-10-27_技术方案_Provider运行边界与跨run隔离_v1.0.md`。本表只列高层工作包，精确文件、命令、测试与提交步骤由后续 `cadence/plans/` 实施 Plan 展开；所有任务均未实施。三个步骤各自独立提交、可单独验收与回滚，顺序为 ① → ② → ③。

## 1. 第一步：边界块无条件注入

- [x] 1.1 在全部作者侧 prompt 构造路径（初次、续轮、修订、自动续写重试）无条件注入「本轮运行边界」块，覆盖聚合/LC 无基线会话，内存态 engine 省略，reviewer 不注入；块文案为事实来源白名单、禁读清单与默认不落盘，体积受固定预算断言约束，不披露状态路径。此步块内暂不含 scratch 条目。（REQ-PRB-01、REQ-PRB-02、REQ-PRB-03）
- [ ] 1.2 提供本步验收：块在四种作者类型与修订、重试路径均出现且至多一次；既有基线教学块与既有 prompt 预算、内存态 engine 测试保持不变；真机核对事故同形态会话的 prompt 已含边界块（需 owner 另行授权 provider 调用）。（REQ-PRB-01、REQ-PRB-03、REQ-PRB-12）

## 2. 第二步：per-session scratch 与生命周期

- [x] 2.1 新增状态根下与现有 session 分区并列的 scratch 分区及其路径方法，宿主存储层提供带存活准入的准备、幂等清理与孤儿回收三项能力，全部删除带护栏（只删自建分区内合法子目录、路径漂移不删、不跟随符号链接）。（REQ-PRB-04、REQ-PRB-05、REQ-PRB-08）
- [x] 2.2 把准备接到 run 登记（supersede 与 claim-if-idle 两条路径、全部 run 种类），把条件清理接到 run 唯一收口点（仅在已无活动 run 时删除，旧 token 迟到收口不误删），把一次孤儿回收接到服务启动 bind 之后；孤儿 TTL 6 小时以具名常量表示。（REQ-PRB-05、REQ-PRB-06、REQ-PRB-07）
- [x] 2.3 边界块在目录存在时加入 scratch 绝对路径与「运行结束系统会删除」的说明，不存在时退化为省略。（REQ-PRB-02、REQ-PRB-04）
- [ ] 2.4 提供本步验收：创建/清空/权限、同 run 保留、已删除 session 不复活、supersede 竞态、取消与失败路径、符号链接与漂移护栏、三类孤儿回收均可观测；真机核对 run 结束后 scratch 被清理（需 owner 另行授权）。（REQ-PRB-04 – REQ-PRB-08）

## 3. 第三步：宿主侧越界检测告警

- [x] 3.1 新增作者侧 tool-call 的纯函数分类能力，覆盖四个稳定码和有界扫描，不引入新依赖，provider 历史目录来自可配置表。（REQ-PRB-09）
- [x] 3.2 把分类结果接到 provider 执行事件漏斗：原事件先持久化，告警以独立事件追加到同一 node detail，按 id 去重、每 node 有上界、告警不再触发扫描，范围限作者侧节点，reviewer 不扫。（REQ-PRB-10）
- [x] 3.3 保持只告警不阻断：检测与接线的任何失败不向 run 传播，不改变 turn 成败、审批与产物确认。（REQ-PRB-11）
- [ ] 3.4 提供本步验收：事故四类越界向量在表驱动用例中全部命中且合法访问不误报；引擎级用例证明原事件不变、告警追加；真机核对诱饵文件存在时 node detail 出现稳定码告警（需 owner 另行授权）。（REQ-PRB-09 – REQ-PRB-11）

## 4. 统一验收与残余风险确认

- [ ] 4.1 Controller：三步全部落地后统一运行各包定向用例与完整回归，核对 provider 侧、envelope、指纹、`StreamingProviderInput` 与 T13 语义零改动，并确认未触碰任何已到行数红线的文件；完成独立审查。（REQ-PRB-12）
- [x] 4.2 在交付说明中明示残余风险：提示词是软约束，检测层是必须兜底；resume 场景禁令无效；codex 原生 FileChange 写 scratch 仍被 T13 判失败；事故现场既有 `/tmp` 残留不由本 change 清理。（REQ-PRB-11、REQ-PRB-12）

未取得以上各步证据，不得宣称跨 run 污染已被治理；本 change 只交付软边界、宿主托管暂存与可发现性，不交付物理隔离。