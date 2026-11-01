# Task 3 报告：Design manifest schema 与强化校验器

## 交付范围

本任务仅在 `cadence/reports/design-weak-model-campaign/` 新建以下文件，未修改 story campaign、生产 Rust/前端代码或既有 campaign 产物：

- `manifest.schema.json`：Design campaign 的严格 JSON Schema（Draft 7）。
- `validate_manifest.py`：stdlib-only manifest 校验 CLI，输出机器可读 JSON，并以 0/1/2 表示成功、校验失败、命令/输入错误。
- `test_validate_manifest.py`：需求书指定的 7 个 `unittest` 回归测试。
- `task-3-report.md`：本实施与 TDD 证据报告。

CLI 接口：

```sh
python3 validate_manifest.py gate-manifest.json [--paired baseline-manifest.json]
```

输出固定包含 `ok`、`errors`、`warnings`、`stats`。`--paired` 不依赖文件顺序：只要两份 manifest 的 `run_kind` 正好组成 `baseline` 与 `revised`，即执行真实配对校验。

## Schema 全文要点

`manifest.schema.json` 的完整约束覆盖如下字段与分支：

| 区域 | 严格约束 |
|---|---|
| 顶层 | `schema` 固定为 `gate-campaign/1`，`run_kind` 只能为 `baseline`/`revised`，`samples` 必须为非空数组。 |
| 主模型身份 | 每个 sample 强制 `provider`、`model`、`model_version`。 |
| Author / Reviewer 记录 | `role_provider.author` 与 `role_provider.reviewer` 均独立要求 `provider`、`model`、`model_version`，避免将两个角色折叠为单一模型记录。 |
| 运行策略 | `strategy` 只能为 `fresh`/`resume`；`resume_available` 必须是 bool。运行时额外校验 `strategy=resume` 时该字段为 `true`。 |
| 样本身份 | `shape_id`、`shape_file`、数字字符串 `repetition`，以及 `case_id.shape_id`、正整数 `case_id.repetition_id` 是必填项；可选 `case_id.round` 必须为正整数。 |
| 时间与产物关联 | `startedAt`、`finishedAt`、`issueId`、`sessionId`、`storySpecId`、`designSpecId` 是必填字段；允许未产生的关联 ID/未完成时间使用 `null`。完成样本在语义校验中还要求非空 `finishedAt`。 |
| 终态与审核 | `finished`、`reviewVerdicts[{verdict,el}]`、`failureClass`、`elapsedSec` 是必填；verdict 限定为 `pass`、`revise`、`needs_human`，`el` 和 `elapsedSec` 非负。 |
| 边界样本 | `boundary_kind` 必填，枚举为 `abstract-positive`、`violation-negative` 或 `null`。 |
| Usage 二选一 | `usage` 要求非负整数 `input_tokens` 与 `cache_read_tokens`；或 `usage_unavailable: true`。Schema 的 `oneOf` 与运行时校验均禁止两者同时存在或两者都缺失。此处是相对 story 弱模型 manifest 的强化点：usage 缺失为 **error**，不是 warning。 |

Schema 是机器可消费契约；校验器使用标准库做对应的 JSON Schema 等价检查，并补充跨字段、跨 manifest、digest 与统计语义，因而运行不需要额外安装 `jsonschema`。

## 校验器实现要点

### 继承并强化的单 manifest 规则

- 以 `(run_kind, provider, case_id.shape_id, case_id.repetition_id, case_id.round)` 检查 `case_id` 去重。
- 检查 `shape_id == case_id.shape_id`，且 `int(repetition) == case_id.repetition_id`。
- 完成样本必须有 review verdict；`finished=true` 与非空 `error` 或非 `completed` 的 `failureClass` 冲突时为 error。非完成样本没有失败归因则产出 warning。
- 汇总 provider 样本数、完成数、verdict、审核轮次直方图、full-chain one-shot 数量与 token 使用量。
- `usage_unavailable` 样本以 warning 标示并不进入 token 聚合；有 `case_id.round > 1` 的 retry token 单独统计为 `*_excluded`，不进入主 token 分母；两个 token 值都为零时警告可能的采集缺失。

### `--paired` 的真实整组剔除

- baseline 与 revised 以 `(provider, shape_id, repetition_id)` 分组。每个组必须一边恰好一个 sample。
- 缺 baseline 或 revised、任何一侧多于一个（`重复 pair/round`）、`case_id.round` 不同、`model_version` 不同、或 `strategy` 不同，均记 error。
- 任一上述问题会让整个 group 计入 `stats.pairing.excluded_groups`，不会保留单边/任选一个样本；仅完整兼容的组进入 `accepted_groups`。

### Corpus / golden digest 复核

- 每次校验均读取 campaign 根目录下的 `corpus/digests.txt` 与 `golden/digests.txt`。
- 用 `hashlib.sha256` 逐项复算。兼容 corpus digest ledger 中的空行、`#` 注释和既有未加 `#` 的说明行，仅校验合法 SHA-256 条目；支持相对 campaign 根目录或 digest 文件目录的条目路径。
- 缺文件、非法相对路径或摘要不一致均为 error。当前仓库 corpus 13 项与 golden 1 项均被复核，共 14 项。

### 边界样本统计

- `abstract-positive` 预期通过；未接受视为 false negative。
- `violation-negative` 预期拒绝；被接受视为 false positive。
- `stats.boundary` 分别报告 observations、false negative/positive 数以及对应上界。
- `rule_of_three_upper_bound(failures, observations)` 在 `failures == 0` 且 `observations > 0` 时返回 `3 / observations`，否则返回 `null`。因此 0 失败、15 次观测的 95% 上界为 `3/15 = 0.2`。

## TDD 证据

先创建 `test_validate_manifest.py`，此时 `validate_manifest.py` 尚不存在；随后实现 schema 与校验器并以同一命令复跑。

### RED（实现前）

命令：

```sh
cd cadence/reports/design-weak-model-campaign && python3 -m unittest test_validate_manifest -v
```

输出：

```text
test_validate_manifest (unittest.loader._FailedTest.test_validate_manifest) ... ERROR

======================================================================
ERROR: test_validate_manifest (unittest.loader._FailedTest.test_validate_manifest)
----------------------------------------------------------------------
ImportError: Failed to import test module: test_validate_manifest
Traceback (most recent call last):
  File "/usr/lib/python3.14/unittest/loader.py", line 137, in loadTestsFromName
    module = __import__(module_name)
  File ".../cadence/reports/design-weak-model-campaign/test_validate_manifest.py", line 12, in <module>
    import validate_manifest as validator
ModuleNotFoundError: No module named 'validate_manifest'

----------------------------------------------------------------------
Ran 1 test in 0.000s

FAILED (errors=1)
```

退出码：`1`（预期 RED）。

### GREEN（实现后）

命令：

```sh
cd cadence/reports/design-weak-model-campaign && python3 -m py_compile validate_manifest.py test_validate_manifest.py && python3 -m unittest test_validate_manifest -v
```

输出：

```text
test_boundary_zero_failure_upper_bound_is_rule_of_three (test_validate_manifest.DesignManifestValidatorTests.test_boundary_zero_failure_upper_bound_is_rule_of_three) ... ok
test_digest_mismatch_is_rejected (test_validate_manifest.DesignManifestValidatorTests.test_digest_mismatch_is_rejected) ... ok
test_duplicate_case_id_is_rejected (test_validate_manifest.DesignManifestValidatorTests.test_duplicate_case_id_is_rejected) ... ok
test_missing_usage_is_rejected (test_validate_manifest.DesignManifestValidatorTests.test_missing_usage_is_rejected) ... ok
test_paired_manifest_missing_counterpart_is_rejected (test_validate_manifest.DesignManifestValidatorTests.test_paired_manifest_missing_counterpart_is_rejected) ... ok
test_paired_manifest_strategy_mismatch_rejects_whole_group (test_validate_manifest.DesignManifestValidatorTests.test_paired_manifest_strategy_mismatch_rejects_whole_group) ... ok
test_valid_manifest_passes (test_validate_manifest.DesignManifestValidatorTests.test_valid_manifest_passes) ... ok

----------------------------------------------------------------------
Ran 7 tests in 0.003s

OK
```

退出码：`0`。

## 7 类用例结果表

| # | 用例 | 验收语义 | 结果 |
|---:|---|---|---|
| 1 | 合法 manifest | 完整必填字段、usage 与 digest fixture 均正确时通过 | PASS |
| 2 | usage 缺失 | 无 `usage` 且无 `usage_unavailable:true` 必须拒绝 | PASS |
| 3 | `case_id` 重复 | 相同 run/provider/shape/repetition/round 不得出现两次 | PASS |
| 4 | paired 缺边 | baseline/revised 任一 side 缺失，相关组为 excluded 且整体拒绝 | PASS |
| 5 | paired strategy 不一致 | 错误且整组剔除，`accepted_groups=0`、`excluded_groups=1` | PASS |
| 6 | digest 不匹配 | corpus checksum 被替换为错误值时拒绝 | PASS |
| 7 | 边界置信上界 | 15 个 abstract-positive 成功样本产生 `false_negatives=0` 与 `0.2` 上界 | PASS |

## 额外验证

| 命令 | 结果 |
|---|---|
| `python3 -m json.tool manifest.schema.json >/dev/null` | PASS，schema 是合法 JSON。 |
| `python3 -m py_compile validate_manifest.py test_validate_manifest.py` | PASS。 |
| `python3 -m unittest test_validate_manifest -v` | PASS，7/7。 |
| `python3 validate_manifest.py /tmp/design-baseline-manifest.json --paired /tmp/design-revised-manifest.json` | PASS，输出 `ok: true`、digest 复核 14 项、pairing `1 accepted / 0 excluded`。临时 manifest 未写入仓库。 |
| `git diff --check` | PASS，无空白错误。 |

## 风险与边界

- 当前没有实际 campaign gate manifest，因此 CLI 的正向 end-to-end 验证使用 `/tmp` 中生成的一对最小合法 baseline/revised fixture；真实 campaign 生成后应在其产出目录重新运行同一 CLI。
- `digests.txt` 的既有 ledger 包含一行不以 `#` 开头的说明文本；校验器按需求容错跳过非 SHA-256 条目，同时仍严格复核所有格式正确的摘要记录。
- 本任务不新增第三方依赖；schema 的运行时执行使用等价的标准库检查，避免验证工具本身成为 campaign 环境前置条件。
