#!/usr/bin/env python3
"""Regression tests for the Design golden normalizer and semantic diff."""

import unittest

import design_golden_diff as golden_diff


CANDIDATE = """# 缓存层设计

## 设计范围

- 缓存读写、序列化选型与版本化兼容策略。

## 设计决策

- [DEC-001] 缓存序列化采用 MessagePack，解码失败按缓存未命中处理。
  - source id: issue_design_0003#选型
- **author-decision-001**：序列化方案选型。
  - **用户选择**：MessagePack
  - 绑定决策：[DEC-001]
  - 绑定来源：[REQ-002] [REQ-003] [REQ-004] [AC-001] [AC-002]

## 公共组件

- [CMP-001] `CacheCodec` 统一封装编码、解码和版本前缀。
  - source id: issue_design_0003#选型

## API 契约

- [API-001] `CacheCodec.encode(value)` 返回带版本前缀的字节序列。
  - source id: issue_design_0003#兼容性

## 数据模型

- 缓存值由版本前缀与 MessagePack payload 组成。

## 风险

- 旧 JSON 缓存数据须在过渡期兼容读取。

## 追踪关系

- [DEC-001] -> [REQ-002]
- [DEC-001] -> [REQ-003]
- [DEC-001] -> [REQ-004]
- [DEC-001] -> [AC-001]
- [DEC-001] -> [AC-002]
"""


class DesignGoldenDiffTests(unittest.TestCase):
    def setUp(self):
        self.golden = golden_diff.normalize(CANDIDATE)

    def assert_difference_kind(self, differences, kind):
        self.assertIn(kind, [difference["kind"] for difference in differences])

    def test_compliant_candidate_passes(self):
        self.assertEqual(golden_diff.compare(golden_diff.normalize(CANDIDATE), self.golden), [])

    def test_missing_required_heading_fails(self):
        candidate = CANDIDATE.replace("## 风险\n\n- 旧 JSON 缓存数据须在过渡期兼容读取。\n\n", "")
        self.assert_difference_kind(
            golden_diff.compare(golden_diff.normalize(candidate), self.golden),
            "forbidden_missing",
        )

    def test_lost_dec_req_link_fails(self):
        candidate = CANDIDATE.replace("- [DEC-001] -> [AC-002]\n", "")
        self.assert_difference_kind(
            golden_diff.compare(golden_diff.normalize(candidate), self.golden),
            "forbidden_link_lost",
        )

    def test_changed_user_decision_answer_fails(self):
        candidate = CANDIDATE.replace("**用户选择**：MessagePack", "**用户选择**：JSON")
        self.assert_difference_kind(
            golden_diff.compare(golden_diff.normalize(candidate), self.golden),
            "forbidden_decision_reversed",
        )

    def test_added_dec_is_allowed(self):
        candidate = CANDIDATE.replace(
            "## 公共组件",
            "- [DEC-002] 缓存 key 增加 codec 版本命名空间。\n"
            "  - source id: issue_design_0003#兼容性\n\n"
            "## 公共组件",
        )
        self.assertEqual(golden_diff.compare(golden_diff.normalize(candidate), self.golden), [])

    def test_ids_and_headings_in_fenced_code_are_ignored(self):
        candidate = CANDIDATE + """
```markdown
## 不应计入的示例
- [DEC-999] 仅为文档示例。
- [CMP-999] 仅为文档示例。
- [API-999] 仅为文档示例。
```
"""
        normalized = golden_diff.normalize(candidate)
        self.assertNotIn("不应计入的示例", normalized["heading_set"])
        self.assertNotIn("DEC-999", normalized["dec_ids"])
        self.assertNotIn("CMP-999", normalized["cmp_ids"])
        self.assertNotIn("API-999", normalized["api_ids"])
        self.assertEqual(golden_diff.compare(normalized, self.golden), [])

    def test_dec_id_key_without_author_decision_maps_to_golden(self):
        candidate = CANDIDATE.replace(
            "- [DEC-001] 缓存序列化采用 MessagePack，解码失败按缓存未命中处理。\n"
            "  - source id: issue_design_0003#选型\n"
            "- **author-decision-001**：序列化方案选型。\n"
            "  - **用户选择**：MessagePack\n"
            "  - 绑定决策：[DEC-001]\n"
            "  - 绑定来源：[REQ-002] [REQ-003] [REQ-004] [AC-001] [AC-002]\n",
            "- [DEC-001] 缓存序列化采用 MessagePack，解码失败按缓存未命中处理。\n"
            "  - source id: issue_design_0003#选型\n"
            "  - **用户选择**：MessagePack\n"
            "  - 绑定来源：[REQ-002] [REQ-003] [REQ-004] [AC-001] [AC-002]\n",
        )
        normalized = golden_diff.normalize(candidate)
        self.assertIn("DEC-001", normalized["user_decisions"])
        self.assertEqual(golden_diff.compare(normalized, self.golden), [])


if __name__ == "__main__":
    unittest.main()
