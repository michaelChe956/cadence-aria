## 1. M0 范围冻结与基线准备

- [ ] 1.1 冻结 non-goals 与范围声明进 proposal/design（已完成于契约阶段，实施前复核一次）；确认 campaign 判定口径：3 组合 × 12 样本、fresh full-chain 全数通过 gate、边界假阳/假阴独立零门槛。验证：口径写入 campaign README。
- [ ] 1.2 冻结 Design corpus 6 形态 + 每形态上游 Story Spec fixture + SHA-256 digest + README（runner 不得改写语料）。验证：digest 文件与语料一一对应。

## 2. M1 基础设施与 baseline（prompt 改造前）

- [ ] 2.1 design golden normalizer + 回归单测（heading 集/DEC-CMP-API/REQ-AC 引用/dec_req_links/source/双章节 decision 抽取；不比 ID 集合相等）+ 首个 golden。验证：normalizer 单测全绿，golden 带 digest。
- [ ] 2.2 design manifest schema + 强化校验器（author/reviewer 分离字段必填、strategy/usage/retry/超时分类、--paired 配对校验、digest 校验、去重）+ Python 单测。验证：对构造的合法/非法 manifest 分别通过/拒绝。
- [ ] 2.3 baseline sanity（3 组合 × 1 样本真机驱动，两阶段种入 Story→Design 生成）→ baseline campaign（12 样本/组合采集）。验证：baseline manifest 通过校验器并落盘；此阶段只记录不设 gate。

## 3. M2 结构契约回归矩阵

- [ ] 3.1 表驱动 Design 负例矩阵（合法基准每次单点变动：缺 heading/缺 ID 族/缺 source/禁止 heading/禁止 token → 对应 finding 字段断言）。验证：全部走 `validate_workspace_artifact_constraints` 真实路径。
- [ ] 3.2 Design 多轮滑窗 fixture（四轮 artifact + choice audit + 未关闭 must_fix；三入口断言保留/摘要行为）。验证：part_32 等价 Design 用例全绿。
- [ ] 3.3 skeleton 防照抄提示语修正为 DEC/CMP/API + source id（同步重构 strip_prefix 式既有测试）。验证：相关 prompt 测试全绿。
- [ ] 3.4 单仓确认红线测试（pass→AuthorConfirm 不自动 Completed；AcceptFinalize→Confirmed；author/revision input 无 aggregate contract）。验证：基于真实 LifecycleStore 的用例通过。

## 4. M3 Reviewer 判例与 repair 纵深

- [ ] 4.1 repair fail-closed 先行：载荷指纹判据测试（JsonNonceMismatch/MissingJsonNonce × 示例指纹/正常载荷 四象限）、NonceMismatch 死分支移除及锁定测试、repair prompt nonce 排除与 readable 回灌。验证：先 RED 后 GREEN，既有 envelope repair 回归不破坏。
- [ ] 4.2 Design 边界判例模块（三判例纯文本，无 sentinel/nonce/完整 JSON）+ build_review_input 单点注入。验证：Design prompt 含判例恰 1 次；Story/WorkItem/WorkItemPlan prompt 不含；判例串不含 ARIA_STRUCTURED_OUTPUT 与 nonce=（防回归锁）。
- [ ] 4.3 candidate→finding contract-driven 测试（抽象追踪→suggestion/pass、测试越界→must_fix×2、风险提及→pass 空 findings；含 B-1 照抄负例）。验证：新测试文件全绿。

## 5. M4 用户反馈修订入口补全

- [ ] 5.1 Design 分支注入 schema/fence 契约/skeleton/missing notes + 输入围栏四反引号化（调用方传 fresh/resume 标记）。验证：Design feedback prompt 含全部注入项且产物内嵌代码块时输入边界完整。
- [ ] 5.2 范围锁负例：Story/WorkItem 反馈入口 prompt 逐字节不变。验证：负例测试全绿。

## 6. M5 revised campaign 与验收

- [ ] 6.1 四件套全量验证 + `openspec validate --strict`。验证：fmt/clippy/check/test 与 web pnpm test/tsc -b 全绿。
- [ ] 6.2 revised campaign（12 样本/组合 × 3 组合真机）：full-chain gate、边界假阳/假阴零门槛判定、usage fresh/resume 分列（不可用如实记录）、golden 规范化 diff、retry/超时分类。验证：manifest 通过强化校验器；gate-report 达标或如实记录失败归因。
- [ ] 6.3 改造收益对比报告（baseline vs revised）+ compact_history 启用决策记录（依据 usage 数据）。验证：报告落盘 cadence/reports/design-weak-model-campaign/。
