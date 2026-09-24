# Tasks: human-gate-convergence

**全局边界**：advisory 不自动升 must_fix；UI 不自行扣预算；不动 abort/terminate 语义（C3）；零新协议消息。

## 1. 预算真值（后端）

- [x] 1.1 gate-local accepted_feedback_turns（与预留同 CAS 原子）；REQ-CG-02 重置边界修订（普通门重建快照 carry-forward）；红绿：三轮修订 3→2→1→0 递减/刷新不回填/accepted 不污染 policy 计数（REQ-HGC-01、CG-02 场景）。
- [x] 1.2 旧会话缺 gate-local 事实保守 unknown（不补预算）。

- [x] 2.1 批次确认门专属 title/why 文案+删除误导相位提示行；两道门可区分用例（REQ-HGC-02 场景 1）。
- [x] 2.2 「距通过」清单投影（must_fix/预检/Verification/approve 可用性）；红绿（REQ-HGC-02 场景 2）。
- [x] 2.3 跨轮 delta（新增/已解决/复现/unknown，消费 C1 结构化 identity）；红绿（REQ-HGC-02 场景 3）。— commit `1b0ccb79`

## 3. 分类透明与解析韧性（前后端）

- [x] 3.1 finding-list 双轨显示（effective class 主显+severity 标注）；红绿（REQ-HGC-03 场景 1）。— commit `a3560fc3`
- [x] 3.2 invalid_json 同 invocation 一次重试（零计数增量）+失败保 diagnostic 进人工；红绿（REQ-HGC-03 场景 2）。— commit `86392ffc`

## 4. 修订教学逃生

- [x] 4.1 REVISION_TEACHING 结构变更逃生条款（受影响闭包联动+影响面声明；禁删字段/绕过 validator 不变）；prompt contract 断言。— commit `86392ffc`

## 5. 门禁收口

- [x] 5.1 lib/it_core/it_web/vitest 全量+strict；F-52 预算假数现场回放（三轮真递减）；既有 CG-02 重置断言按新语义改写登记。
