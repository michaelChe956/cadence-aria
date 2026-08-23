#!/usr/bin/env python3
"""TDD regression tests for the strict Design campaign manifest validator."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import validate_manifest as validator


class DesignManifestValidatorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.campaign_root = Path(self.temp_dir.name)
        self._write_digest_fixture()

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _write_digest_fixture(self) -> None:
        files = {
            "corpus/fixture.md": "corpus fixture\n",
            "golden/fixture.json": "{\"fixture\": true}\n",
        }
        digest_lines: dict[str, list[str]] = {"corpus": ["# generated fixture digest"], "golden": ["# generated fixture digest"]}
        for relative_path, content in files.items():
            path = self.campaign_root / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            directory, filename = relative_path.split("/", 1)
            digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
            digest_lines[directory].append(f"{digest}  {filename}")
        for directory, lines in digest_lines.items():
            (self.campaign_root / directory / "digests.txt").write_text(
                "\n".join(lines) + "\n", encoding="utf-8"
            )

    def sample(
        self,
        *,
        provider: str = "provider-a",
        shape_id: str = "01",
        repetition_id: int = 1,
        strategy: str = "fresh",
        boundary_kind: str | None = None,
        finished: bool = True,
    ) -> dict:
        return {
            "provider": provider,
            "model": "model-a",
            "model_version": "2026-08",
            "role_provider": {
                "author": {
                    "provider": "author-provider",
                    "model": "author-model",
                    "model_version": "author-v1",
                },
                "reviewer": {
                    "provider": "reviewer-provider",
                    "model": "reviewer-model",
                    "model_version": "reviewer-v1",
                },
            },
            "strategy": strategy,
            "resume_available": strategy == "resume",
            "shape_id": shape_id,
            "shape_file": f"{shape_id}-fixture.md",
            "repetition": str(repetition_id),
            "case_id": {"shape_id": shape_id, "repetition_id": repetition_id},
            "startedAt": "2026-08-22T00:00:00Z",
            "finishedAt": "2026-08-22T00:00:01Z",
            "issueId": "issue-1",
            "sessionId": f"session-{provider}-{shape_id}-{repetition_id}",
            "storySpecId": "story-1",
            "designSpecId": "design-1",
            "finished": finished,
            "reviewVerdicts": [{"verdict": "pass", "el": 1.0}],
            "failureClass": "completed" if finished else "driver-timeout",
            "elapsedSec": 1.0,
            "boundary_kind": boundary_kind,
            "usage": {"input_tokens": 123, "cache_read_tokens": 45},
        }

    def manifest(self, *samples: dict, run_kind: str = "baseline") -> dict:
        return {"schema": "gate-campaign/1", "run_kind": run_kind, "samples": list(samples)}

    def validate(self, manifest: dict, paired: dict | None = None) -> dict:
        return validator.validate_manifests(
            manifest,
            paired_doc=paired,
            campaign_root=self.campaign_root,
        )

    def assert_error_contains(self, report: dict, needle: str) -> None:
        self.assertFalse(report["ok"], report)
        self.assertTrue(any(needle in error for error in report["errors"]), report["errors"])

    def test_valid_manifest_passes(self) -> None:
        report = self.validate(self.manifest(self.sample()))
        self.assertTrue(report["ok"], report)
        self.assertEqual(report["stats"]["case_id_duplicates"], 0)

    def test_missing_usage_is_rejected(self) -> None:
        sample = self.sample()
        sample.pop("usage")
        report = self.validate(self.manifest(sample))
        self.assert_error_contains(report, "usage 或 usage_unavailable")

    def test_duplicate_case_id_is_rejected(self) -> None:
        first = self.sample()
        second = self.sample()
        second["sessionId"] = "different-session"
        report = self.validate(self.manifest(first, second))
        self.assert_error_contains(report, "case_id 重复")

    def test_paired_manifest_missing_counterpart_is_rejected(self) -> None:
        baseline = self.manifest(self.sample(shape_id="01"), run_kind="baseline")
        revised = self.manifest(self.sample(shape_id="02"), run_kind="revised")
        report = self.validate(baseline, revised)
        self.assert_error_contains(report, "配对缺边")
        self.assertEqual(report["stats"]["pairing"]["accepted_groups"], 0)
        self.assertEqual(report["stats"]["pairing"]["excluded_groups"], 2)

    def test_paired_manifest_strategy_mismatch_rejects_whole_group(self) -> None:
        baseline = self.manifest(self.sample(strategy="fresh"), run_kind="baseline")
        revised = self.manifest(self.sample(strategy="resume"), run_kind="revised")
        report = self.validate(baseline, revised)
        self.assert_error_contains(report, "strategy 不一致")
        self.assertEqual(report["stats"]["pairing"]["accepted_groups"], 0)
        self.assertEqual(report["stats"]["pairing"]["excluded_groups"], 1)

    def test_digest_mismatch_is_rejected(self) -> None:
        digest_file = self.campaign_root / "corpus" / "digests.txt"
        digest_file.write_text(
            "# comments and blank lines are ignored\n\n"
            + "0" * 64
            + "  fixture.md\n",
            encoding="utf-8",
        )
        report = self.validate(self.manifest(self.sample()))
        self.assert_error_contains(report, "digest 不匹配")

    def test_boundary_zero_failure_upper_bound_is_rule_of_three(self) -> None:
        samples = [
            self.sample(repetition_id=repetition_id, boundary_kind="abstract-positive")
            for repetition_id in range(1, 16)
        ]
        report = self.validate(self.manifest(*samples))
        self.assertTrue(report["ok"], report)
        boundary = report["stats"]["boundary"]["abstract-positive"]
        self.assertEqual(boundary["observations"], 15)
        self.assertEqual(boundary["false_negatives"], 0)
        self.assertEqual(boundary["false_negative_upper_95"], 0.2)
        self.assertEqual(validator.rule_of_three_upper_bound(0, 15), 0.2)
        self.assertIsNone(validator.rule_of_three_upper_bound(1, 15))
        self.assertIsNone(validator.rule_of_three_upper_bound(0, 0))


if __name__ == "__main__":
    unittest.main()
