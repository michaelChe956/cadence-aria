# Design weak-model campaign corpus（冻结）

六个冻结的设计链路 Issue 描述（每形态一个 md）+ 六份冻结上游已确认 Story Spec fixture + digest 冻结记录，供 Task 4 baseline 与 Task 16 revised campaign 的 runner 按 shape_id 读取。

## 形态清单

- `01-api-design.md`：单仓 API 设计（订单状态查询接口：字段/错误码/幂等，分页）。考察必需 heading、稳定 API/DEC ID、source 覆盖。
- `02-data-model.md`：数据模型设计（多租户报表：报表定义/生成任务/租户隔离/索引，不做迁移）。考察 CMP/DEC ID 与结构化呈现。
- `03-choice-to-dec.md`：用户 choice 映射 DEC（JSON vs MessagePack 选型须经 AskUserQuestion 确认并写入 DEC 绑定来源）。考察用户决策不反转、dec_req_links 不改绑。
- `04-abstract-traceability.md`：抽象追踪正例（订单结算：策略与舍入，DEC 追溯上游 `[REQ-*]/[AC-*]`）。考察上游引用不丢失、不越界写测试。
- `05-test-boundary-violation.md`：测试越界反例（订单导出：诱导给出测试文件路径与运行命令）。考察边界判定与 must_fix 返修收敛。
- `06-multi-constraint.md`：多约束返修（缓存预热：调度复用/目录限定/失败不取消他租户/公共组件与风险可追踪）。考察约束逐条覆盖与决策追踪。

每个形态 `NN-*.md` 配对一份冻结上游 Story Spec fixture `NN-story-fixture.md`：合规 Story Spec markdown，含一级标题、必需 heading（用户故事/功能需求/非功能需求/成功标准/范围，03 另含待确认项与用户确认决策）、`[REQ-*]/[AC-*]`（及 `[NFR-*]`）稳定 ID、每条需求含 `source id:` 追踪行。04 的 fixture 含可被 DEC 引用的 REQ/AC；03 的 fixture 含用户决策 author-decision-001 及其绑定。

## 冻结规则

`digests.txt` 为冻结记录（SHA-256 over UTF-8 bytes，格式 `<相对路径>  <sha256hex>`）。任何 campaign runner SHALL NOT 改写本目录的语料文件与上游 fixture；runner 以 digest 校验读取一致性。语料自冻结起不变更；若必须变更，须重跑全部 baseline。

## 判定口径（逐字采用）

- 主 gate：3 组合 × 12 样本（6 形态 × 2 重复），fresh full-chain 全数通过。
- D05 特例口径：「成功 = 含一次正确 must_fix 返修的全链走通且首轮边界判定正确」，不按「首轮 pass」计。
- 边界 mini-campaign：D04/D05 各 5 重复 × 3 provider = 各 15 观测，假阳/假阴 = 0 方可通过。
- 所有成功率结论 SHALL 附样本量与置信上界（示例：0/36 通过 → 95% 置信上界 <8.3%）。

## 相关产物

- 基线 campaign 数据：Task 4 产出（任何 prompt 改造前采集）。
- 规范来源：`openspec/changes/harden-single-repo-design-weak-models/specs/design-pipeline-weak-model-hardening/spec.md`；Story Spec 合规结构参照 `openspec/specs/story-pipeline-weak-model-hardening/spec.md`。
- 组织方式先例：`cadence/reports/story-weak-model-campaign/corpus/`。
