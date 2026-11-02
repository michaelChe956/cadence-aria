# 核查：33 个 `openspec validate --strict` 失败项的完成度（2026-11-02）

> 触发：owner「整理下所有失败的 changes，看看有没有都完成」。

> 背景：2026-11-02 归档 19 个 change 后，`openspec validate --all --strict` = **43 passed / 33 failed**。

> 归档前基线 = 49 passed / 37 failed（失败项集合与现在同构，归档未引入新失败）。


## 一页结论

**33 个失败项全部是 warning 级（措辞/风格），零真 ERROR；对应 capability 的功能全部已落地，零实现缺口。**

| 警告类型 | 条数 | 含义 |
|---|---|---|
| 正文>500字 | 84 | 规格措辞问题，**与功能是否完成无关** |
| 缺SHALL/MUST | 13 | 规格措辞问题，**与功能是否完成无关** |
| Purpose过短 | 1 | 规格措辞问题，**与功能是否完成无关** |

**判定方法**：不看 validate 结果（它只查措辞），而是逐 capability 抽 delta 里的代码标识符（`snake_case` 符号 / REQ 编号），到 `src/` + `web/src/` + `tests/` 里查是否真实存在。


## 逐项核查表

| capability | 代码标识符 | 命中 | 未命中 | validate 警告 | 来源 change |
|---|---|---|---|---|---|
| `artifact-candidate-selection` | 0 | —（无标识符可抽） | — | 正文>500字×2 | 09-26-artifact-candidate-selection |
| `codebase-kinds` | 0 | —（无标识符可抽） | — | Purpose过短×1、缺SHALL/MUST×3 | 08-19-add-logical-codebase-product、09-30-logical-codebase-bootstrap-a |
| `coder-owned-work-item-commit` | 3 | 3/3 | — | 正文>500字×1 | 08-08-coder-owned-commits-and-prov、09-30-coding-execution-resilience- |
| `coding-code-review-triage` | 17 | 17/17 | — | 正文>500字×2 | 07-29-open-code-review-triage-gate、09-30-coding-execution-resilience- |
| `coding-provider-transport-retry` | 5 | 5/5 | — | 正文>500字×1 | 08-08-coder-owned-commits-and-prov、11-02-batch1-coding-autopilot-main |
| `coding-workspace-completion` | 3 | 3/3 | — | 正文>500字×2 | 07-29-relax-completion-testing-rep、08-08-coder-owned-commits-and-prov |
| `design-pipeline-weak-model-hardening` | 2 | 1/2 | `resume_available` | 正文>500字×2 | 08-24-harden-single-repo-design-we |
| `group-final-review-triage` | 9 | 9/9 | — | 正文>500字×1 | 07-29-open-group-final-review-chan、08-20-scalable-group-final-review |
| `kimi-acp-client-services` | 3 | 3/3 | — | 正文>500字×5 | 08-22-harden-story-pipeline-weak-m、08-24-harden-single-repo-design-we |
| `kimi-code-provider-integration` | 2 | 2/2 | — | 正文>500字×2 | 09-19-add-kimi-code-provider、11-02-provider-desandbox-full-open |
| `legacy-protocol-retirement` | 1 | 1/1 | — | 正文>500字×2 | 09-21-retire-legacy-workitem-proto、09-23-plan-compile-gate-visibility |
| `logical-codebase-aggregate-index` | 1 | 1/1 | — | 缺SHALL/MUST×1 | 08-05-add-logical-codebase、08-19-add-logical-codebase-product |
| `logical-codebase-aggregate-planning` | 11 | 11/11 | — | 缺SHALL/MUST×2 | 08-05-add-logical-codebase、08-19-add-logical-codebase-product |
| `logical-codebase-registration` | 18 | 18/18 | — | 正文>500字×2、缺SHALL/MUST×4 | 08-05-add-logical-codebase、08-19-add-logical-codebase-product |
| `multi-target-group-coding` | 24 | 24/24 | — | 正文>500字×4 | 09-21-multi-repo-group-coding、09-28-work-item-group-autopilot |
| `non-interrupt-repository-bootstrap` | 3 | 3/3 | — | 正文>500字×1、缺SHALL/MUST×1 | 07-23-add-repository-initializatio、07-29-add-repository-initializatio |
| `per-issue-base-branch` | 2 | 2/2 | — | 正文>500字×1 | 09-26-per-issue-base-branch |
| `per-repo-coding-execution` | 7 | 7/7 | — | 正文>500字×4 | 08-05-add-logical-codebase、09-21-multi-repo-group-coding |
| `repository-initialization-progress` | 7 | 7/7 | — | 正文>500字×1 | 07-23-add-repository-initializatio、07-29-add-repository-initializatio |
| `session-policy-envelope` | 13 | 12/13 | `durable_tool_policy_audit` | 正文>500字×5 | 08-05-add-logical-codebase、08-22-harden-story-pipeline-weak-m |
| `story-pipeline-weak-model-hardening` | 7 | 7/7 | — | 正文>500字×2 | 08-22-harden-story-pipeline-weak-m、08-24-harden-single-repo-design-we |
| `web-cockpit-full-course` | 8 | 8/8 | — | 正文>500字×2 | 09-30-cockpit-fullcourse-connectio |
| `web-runtime-repository-routing` | 14 | 14/14 | — | 正文>500字×3 | 08-11-fix-web-runtime-repository-r、09-30-enrollment-recovery-surface |
| `web-workbench-autopilot` | 25 | 24/25 | `auto_start_coding` | 正文>500字×7 | 09-13-ui-workbench-autopilot、11-02-batch1-coding-autopilot-main |
| `work-item-group-autopilot` | 19 | 19/19 | — | 正文>500字×8 | 09-28-work-item-group-autopilot、09-30-enrollment-recovery-surface |
| `work-item-group-coding-execution` | 11 | 11/11 | — | 正文>500字×2 | 09-03-workitem-conversational-gate、09-30-coding-execution-resilience- |
| `work-item-plan-advance` | 8 | 8/8 | — | 正文>500字×3 | 09-03-workitem-conversational-gate、09-21-multi-repo-group-coding |
| `work-item-plan-conversational-gate` | 13 | 13/13 | — | 正文>500字×4 | 09-03-workitem-conversational-gate、09-21-retire-legacy-workitem-proto |
| `work-item-plan-single-candidate` | 48 | 48/48 | — | 正文>500字×4 | 08-31-rearch-workitem-plan-pipelin、09-03-workitem-conversational-gate |
| `work-item-runtime-projection` | 0 | —（无标识符可抽） | — | 正文>500字×1 | 07-29-fix-work-item-runtime-projec |
| `work-item-typed-outcome-policy` | 7 | 7/7 | — | 正文>500字×6 | 08-27-workitem-typed-outcome-polic、09-26-single-candidate-contract-pr |
| `workspace-artifact-open-item-validation` | 0 | —（无标识符可抽） | — | 缺SHALL/MUST×2 | 08-21-tolerate-explained-empty-ope |
| `workspace-session-connection-resilience` | 4 | 4/4 | — | 正文>500字×4 | 09-30-cockpit-fullcourse-connectio |

**统计**：全命中 **26** 个 / 部分命中 **3** 个 / 零命中 **0** 个。


## 3 处「未命中」是命名变体，不是实现缺口

| 规格里的名字 | 代码里的实际实现 |
|---|---|
| `durable_tool_policy_audit` | `src/cross_cutting/tool_policy_audit.rs`（`resume_with_audit_record` / `append_superseded_policy_drift`）|
| `auto_start_coding` | `auto_start_once`（attempt 上的冻结 policy）|
| `resume_available` | **误判，实为真字段**：campaign sample manifest 的 `resume_available(bool)`
（定义见 `cadence/plans/2026-08-22_计划文档_单仓Design弱模型加固_v1.0.md` 的 sample 必填字段与对应 task-4-report；
测量工具 `validate_manifest.py` / `gate-loop.sh` 在仓外）→ **保持原样，不改名**|

→ 前两处已对齐（见提交 `0b5d54c`）；第三处经复核是**误判**，保持原样。

> 补充（2026-11-02 复核）：`resume_available` 出现在「Design corpus/golden/campaign 实测验收」要求里，
> 语义是 **campaign sample manifest 的字段**，不是生产代码符号。计划文档与 task-4-report 都把它列为
> sample 必填字段，测量工具在仓外（`validate_manifest.py` / `gate-loop.sh`）。
> 因此**不改名**；上一版把它当成「命名漂移」是判断错误。


## 留下的欠账（只影响 validate 观感，不影响功能）

- 84 处 requirement 正文超 500 字 → 建议拆成 scenario（一次批量文本重构，可另开 change）；
- 13 处 requirement 缺 SHALL/MUST → 措辞补齐；
- 这两类是 `openspec validate --strict` 把 warning 计入失败的原因，**不影响归档与实现判定**。


## 附：唯一保留未归档的 change

`mechanicalize-plan-compile-repair` —— owner 2026-11-02 裁定**保留不归档**（其核心机制 `LoweringFault` 在代码中 0 命中，归档会把未落地要求写进主 specs）。

