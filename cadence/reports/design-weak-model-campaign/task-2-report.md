# Task 2 报告：Design golden normalizer、回归测试与首个 golden

## 交付范围

仅在 `cadence/reports/design-weak-model-campaign/` 新建以下 Task 2 产物：

- `design_golden_diff.py`：Design Spec 规范化与 golden 语义比对 CLI。
- `test_design_golden_diff.py`：7 个需求书指定的 `unittest` 回归用例。
- `golden/design_0001.golden.json`：D03（choice → DEC 绑定）首个冻结结构化 golden。
- `golden/digests.txt`：golden 的 SHA-256 摘要。

未修改 story campaign 文件或生产代码；实现与测试只使用 Python 标准库。

## 实现摘要

`design_golden_diff.py` 复用了 story 版的 `read_markdown()`、`compare()`、CLI 主流程风格，并替换为 Design schema：

- `SpecVersionRecord.markdown` 是 JSON 输入的唯一 Markdown 字段；普通 `.md` 直接读取。
- 规范化输出固定 `schema_version: 1` 与 `workspace_type: "design"`，并输出 `heading_set`、DEC/CMP/API ID、上游 REQ/AC 引用、`dec_req_links`、每个 Design ID 的 `source_id_coverage` 与 `user_decisions`。
- 按需求书给定的 `DEC_RE`、`CMP_RE`、`API_RE` 与 `LINK_RE` 抽取稳定 ID 和追踪关系。
- fenced code block 在解析前整体排除，因此其中的 heading 或 ID 不影响验收。
- 用户决策仅从 `## 设计决策` 与 `## 追踪关系` 读取：先以 `author-decision-*` 为键；当 candidate 仅有 `[DEC-*]` 时，`compare()` 按 frozen decision 的 `dec_id` 回退映射。
- `compare()` 对既有 heading、ID、REQ/AC 引用和 source 覆盖缺失报 `forbidden_missing` / `forbidden_source_lost`；对 DEC 上游链接丢失报 `forbidden_link_lost`；对冻结决策报 `forbidden_decision_lost`、`forbidden_decision_reversed` 或 `forbidden_decision_binding_changed`。新增 DEC/CMP/API 不会报错。额外非 Design heading 报 `forbidden_added`。

## TDD 证据

先创建 `test_design_golden_diff.py`，在尚未创建实现模块时运行指定命令；随后实现 normalizer 并以同一命令复跑。

### RED（实现前）

命令：

```sh
cd cadence/reports/design-weak-model-campaign && python3 -m unittest test_design_golden_diff -v
```

输出：

```text
test_design_golden_diff (unittest.loader._FailedTest.test_design_golden_diff) ... ERROR

======================================================================
ERROR: test_design_golden_diff (unittest.loader._FailedTest.test_design_golden_diff)
----------------------------------------------------------------------
ImportError: Failed to import test module: test_design_golden_diff
Traceback (most recent call last):
  File "/usr/lib/python3.14/unittest/loader.py", line 137, in loadTestsFromName
    module = __import__(module_name)
  File ".../cadence/reports/design-weak-model-campaign/test_design_golden_diff.py", line 6, in <module>
    import design_golden_diff as golden_diff
ModuleNotFoundError: No module named 'design_golden_diff'

----------------------------------------------------------------------
Ran 1 test in 0.000s

FAILED (errors=1)
```

退出码：`1`（预期 RED）。

### GREEN（实现后）

命令：

```sh
cd cadence/reports/design-weak-model-campaign && python3 -m py_compile design_golden_diff.py && python3 -m unittest test_design_golden_diff -v
```

输出：

```text
test_added_dec_is_allowed (test_design_golden_diff.DesignGoldenDiffTests.test_added_dec_is_allowed) ... ok
test_changed_user_decision_answer_fails (test_design_golden_diff.DesignGoldenDiffTests.test_changed_user_decision_answer_fails) ... ok
test_compliant_candidate_passes (test_design_golden_diff.DesignGoldenDiffTests.test_compliant_candidate_passes) ... ok
test_dec_id_key_without_author_decision_maps_to_golden (test_design_golden_diff.DesignGoldenDiffTests.test_dec_id_key_without_author_decision_maps_to_golden) ... ok
test_ids_and_headings_in_fenced_code_are_ignored (test_design_golden_diff.DesignGoldenDiffTests.test_ids_and_headings_in_fenced_code_are_ignored) ... ok
test_lost_dec_req_link_fails (test_design_golden_diff.DesignGoldenDiffTests.test_lost_dec_req_link_fails) ... ok
test_missing_required_heading_fails (test_design_golden_diff.DesignGoldenDiffTests.test_missing_required_heading_fails) ... ok

----------------------------------------------------------------------
Ran 7 tests in 0.002s

OK
```

退出码：`0`。

## 7 用例结果表

| # | 用例 | 断言的语义 | 结果 |
|---:|---|---|---|
| 1 | 合规 candidate | 结构与冻结 golden 完全兼容 | PASS |
| 2 | 删除一个 required heading | 存在 `forbidden_missing` | PASS |
| 3 | 删除一条 `dec_req_links` | 存在 `forbidden_link_lost` | PASS |
| 4 | 改写用户决策答案 | 存在 `forbidden_decision_reversed` | PASS |
| 5 | 新增一个 DEC | 不产生差异（Design 合法多样） | PASS |
| 6 | fenced code 内示例 ID/heading | 不写入 normalized 字段，比较仍通过 | PASS |
| 7 | 仅 DEC ID 作为决策键 | 通过 golden `dec_id` 映射匹配原 `author-decision-*` | PASS |

## 首个 golden 的构造与冻结

首个样本锚定 D03 的 choice → DEC 绑定形态。上游素材为：

- `corpus/03-choice-to-dec.md`
- `corpus/03-story-fixture.md`

用于生成的合规 Design candidate（在 `/tmp` 临时构造，未纳入仓库）为：

```markdown
# 缓存层设计

## 设计范围

- 覆盖缓存读写、序列化选型与版本化兼容策略；不变更缓存淘汰策略。

## 设计决策

- [DEC-001] 缓存序列化采用 MessagePack；解码失败按缓存未命中处理。
  - source id: issue_design_0003#选型
- **author-decision-001**：序列化方案选型。
  - **用户选择**：MessagePack
  - 绑定决策：[DEC-001]
  - 绑定来源：[REQ-002] [REQ-003] [REQ-004] [AC-001] [AC-002]

## 公共组件

- [CMP-001] `CacheCodec` 统一封装编码、解码和版本前缀。
  - source id: issue_design_0003#选型

## API 契约

- [API-001] `CacheCodec.encode(value)` 返回带版本前缀的字节序列；`decode(bytes)` 在不兼容或解码失败时返回缓存未命中。
  - source id: issue_design_0003#兼容性

## 数据模型

- 缓存值由 codec 版本前缀与 MessagePack payload 组成；读取时按版本分派兼容解码器。

## 风险

- 切换期间旧 JSON 缓存可能仍存在；保留旧版本解码器至过渡期结束。

## 追踪关系

- [DEC-001] -> [REQ-002]
- [DEC-001] -> [REQ-003]
- [DEC-001] -> [REQ-004]
- [DEC-001] -> [AC-001]
- [DEC-001] -> [AC-002]
```

生成与自检命令：

```sh
python3 design_golden_diff.py --normalize /tmp/design_0001_candidate.md > golden/design_0001.golden.json
sha256sum golden/design_0001.golden.json > golden/digests.txt
python3 design_golden_diff.py /tmp/design_0001_candidate.md golden/design_0001.golden.json
```

自检输出：

```json
{
  "differences": [],
  "pass": true
}
```

### 冻结 golden 内容

```json
{
  "api_ids": [
    "API-001"
  ],
  "cmp_ids": [
    "CMP-001"
  ],
  "dec_ids": [
    "DEC-001"
  ],
  "dec_req_links": {
    "DEC-001": [
      "AC-001",
      "AC-002",
      "REQ-002",
      "REQ-003",
      "REQ-004"
    ]
  },
  "heading_set": [
    "API 契约",
    "公共组件",
    "数据模型",
    "设计决策",
    "设计范围",
    "追踪关系",
    "风险"
  ],
  "referenced_ac_ids": [
    "AC-001",
    "AC-002"
  ],
  "referenced_req_ids": [
    "REQ-002",
    "REQ-003",
    "REQ-004"
  ],
  "schema_version": 1,
  "source_id_coverage": {
    "API-001": [
      "issue_design_0003#兼容性"
    ],
    "CMP-001": [
      "issue_design_0003#选型"
    ],
    "DEC-001": [
      "issue_design_0003#选型"
    ]
  },
  "user_decisions": {
    "author-decision-001": {
      "answer": "MessagePack",
      "bindings": {
        "ac_ids": [
          "AC-001",
          "AC-002"
        ],
        "dec_ids": [
          "DEC-001"
        ],
        "req_ids": [
          "REQ-002",
          "REQ-003",
          "REQ-004"
        ]
      },
      "dec_id": "DEC-001"
    }
  },
  "workspace_type": "design"
}
```

Digest：

```text
f5b920cafbc18cdc43d2def5fa97efcfad591492ebf9445959c5a667753759cb  golden/design_0001.golden.json
```

## 验证清单

| 命令 | 结果 |
|---|---|
| `python3 -m py_compile design_golden_diff.py` | PASS |
| `python3 -m unittest test_design_golden_diff -v` | PASS，7/7 |
| `python3 design_golden_diff.py /tmp/design_0001_candidate.md golden/design_0001.golden.json` | PASS，`differences: []` |
| `sha256sum -c golden/digests.txt` | PASS，`golden/design_0001.golden.json: OK` |
| `git diff --check` | PASS，无输出 |

## 风险与边界

- 本版依据需求书的 Markdown 约定，要求 DEC→REQ/AC 追踪关系逐条写为 `- [DEC-*] ... [REQ-*|AC-*]`；若未来允许单行携带多个目标或不同关系语法，应先扩展 `LINK_RE` 并增加 golden 回归用例。
- `forbidden_added` 只用于非 `DESIGN_HEADINGS` 的二级 heading；新增 DEC/CMP/API、REQ/AC 引用与 source ID 不会造成 added 错误，以保留设计方案的合法多样性。
