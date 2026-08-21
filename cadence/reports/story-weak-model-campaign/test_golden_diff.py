#!/usr/bin/env python3
"""Regression coverage for golden_diff.py's forbidden-difference checks."""
from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = Path(__file__).with_name("golden_diff.py")
SOURCE = ROOT / ".aria/projects/project_0001/issues/issue_0001/versions/story_spec_0001/version_0001.json"
GOLDEN = Path(__file__).with_name("golden") / "issue_0001.golden.json"

spec = importlib.util.spec_from_file_location("golden_diff", SCRIPT)
assert spec and spec.loader
golden_diff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(golden_diff)


class GoldenDiffTests(unittest.TestCase):
    def setUp(self) -> None:
        self.markdown = json.loads(SOURCE.read_text(encoding="utf-8"))["markdown"]
        self.golden = json.loads(GOLDEN.read_text(encoding="utf-8"))

    def test_issue_0001_version_matches_frozen_golden(self) -> None:
        self.assertEqual(golden_diff.compare(golden_diff.normalize(self.markdown), self.golden), [])

    def test_lost_source_id_is_forbidden(self) -> None:
        modified = self.markdown.replace(
            "source id: issue_0001#背景", "source id: intentionally-removed", 1
        )
        differences = golden_diff.compare(golden_diff.normalize(modified), self.golden)
        self.assertIn(
            "forbidden_source_lost",
            {difference["kind"] for difference in differences},
        )


if __name__ == "__main__":
    unittest.main()
