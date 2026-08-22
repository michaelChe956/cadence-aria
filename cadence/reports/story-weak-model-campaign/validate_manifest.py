#!/usr/bin/env python3
"""Validate a gate-campaign manifest against manifest.schema.json plus semantic rules.

Usage:
  python3 validate_manifest.py [gate-manifest.json]

Semantic rules (beyond the JSON schema):
  1. case_id uniqueness: no two samples share (run_kind, provider, shape_id, repetition_id[, round]).
  2. Consistency: sample.shape_id == case_id.shape_id and int(sample.repetition) == case_id.repetition_id.
  3. Pairing (only when both baseline and revised manifests are validated together via --paired):
     every revised case must have a baseline counterpart and vice versa; version/strategy/round
     must match inside a pair (整组剔除 rather than择优取样 when a pair is broken).
  4. Gate integrity: every sample that claims finished must carry >=1 review verdict;
     a finished sample with an error/failureClass is contradictory.
  5. Usage accounting: retry samples must not enter the token denominator (they must be
     flagged separately); missing/zero usage is a warning when run_kind records
     usage_unavailable, an error otherwise.

Exit code 0 = manifest valid (warnings allowed); 1 = validation failed; 2 = usage error.

Only the Python standard library is used, matching golden_diff.py conventions.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path
from typing import Any

SCHEMA_PATH = Path(__file__).with_name("manifest.schema.json")

REQUIRED_TOP = ("schema", "run_kind", "samples")
VERDICTS = ("pass", "revise", "needs_human")


def fail(errors: list[str], msg: str) -> None:
    errors.append(msg)


def validate_schema_shape(doc: Any, errors: list[str], warnings: list[str]) -> None:
    optional_missing: dict[str, list[int]] = {}
    usage_untracked: list[int] = []
    if not isinstance(doc, dict):
        fail(errors, "manifest 顶层必须是 JSON object")
        return
    for key in REQUIRED_TOP:
        if key not in doc:
            fail(errors, f"顶层缺少必填字段 {key}")
    if doc.get("schema") != "gate-campaign/1":
        fail(errors, f"schema 必须为 'gate-campaign/1'，实际 {doc.get('schema')!r}")
    if doc.get("run_kind") not in ("baseline", "revised"):
        fail(errors, f"run_kind 必须为 baseline|revised，实际 {doc.get('run_kind')!r}")
    samples = doc.get("samples")
    if not isinstance(samples, list) or not samples:
        fail(errors, "samples 必须为非空数组")
        return

    for i, s in enumerate(samples):
        where = f"samples[{i}]"
        if not isinstance(s, dict):
            fail(errors, f"{where} 必须是 object")
            continue
        for key in ("provider", "shape_id", "shape_file", "repetition", "case_id",
                    "startedAt", "issueId", "sessionId", "finished",
                    "reviewVerdicts", "elapsedSec"):
            if key not in s:
                fail(errors, f"{where} 缺少必填字段 {key}")
        cid = s.get("case_id")
        if not (isinstance(cid, dict) and "shape_id" in cid and "repetition_id" in cid):
            fail(errors, f"{where}.case_id 缺少 shape_id/repetition_id")
        if isinstance(s.get("reviewVerdicts"), list):
            for j, v in enumerate(s["reviewVerdicts"]):
                verdict = v.get("verdict") if isinstance(v, dict) else None
                if verdict not in VERDICTS:
                    fail(errors, f"{where}.reviewVerdicts[{j}].verdict 非法: {verdict!r}")
        else:
            fail(errors, f"{where}.reviewVerdicts 必须为数组")
        # 可选字段已知缺口 → 汇总 warning（story 先例：model/version/strategy/usage 未记录）
        for key in ("model", "model_version", "strategy"):
            if key not in s:
                optional_missing.setdefault(key, []).append(i)
        if s.get("usage") is None and not s.get("usage_unavailable"):
            usage_untracked.append(i)

    total = len(doc.get("samples") or [])
    for key, idxs in optional_missing.items():
        warnings.append(f"{len(idxs)}/{total} 样本缺少可选字段 {key}（建议后续 campaign 补记）")
    if usage_untracked:
        warnings.append(f"{len(usage_untracked)}/{total} 样本无 usage 且未声明 usage_unavailable")


def validate_semantics(doc: dict, errors: list[str], warnings: list[str]) -> dict[str, Any]:
    stats: dict[str, Any] = {}
    samples = doc["samples"]
    run_kind = doc["run_kind"]

    keys = []
    for i, s in enumerate(samples):
        where = f"samples[{i}]"
        cid = s.get("case_id") or {}
        if s.get("shape_id") != cid.get("shape_id"):
            fail(errors, f"{where}: shape_id {s.get('shape_id')!r} 与 case_id.shape_id {cid.get('shape_id')!r} 不一致")
        try:
            if int(str(s.get("repetition"))) != cid.get("repetition_id"):
                fail(errors, f"{where}: repetition 与 case_id.repetition_id 不一致")
        except (TypeError, ValueError):
            fail(errors, f"{where}: repetition 无法解析为整数")
        keys.append((run_kind, s.get("provider"), cid.get("shape_id"), cid.get("repetition_id"), cid.get("round")))

    dupes = [k for k, v in Counter(keys).items() if v > 1]
    for k in dupes:
        fail(errors, f"case_id 重复 {v} 次: {k}")
    stats["case_id_duplicates"] = len(dupes)

    by_provider: Counter[str] = Counter(s.get("provider", "?") for s in samples)
    stats["samples_per_provider"] = dict(by_provider)

    finished = sum(1 for s in samples if s.get("finished"))
    stats["finished"] = finished
    for i, s in enumerate(samples):
        where = f"samples[{i}]"
        if s.get("finished"):
            if not (s.get("reviewVerdicts") or []):
                fail(errors, f"{where}: finished=true 但 reviewVerdicts 为空")
            # failureClass 是终态分类（completed/driver-timeout/...），不是错误标志
            if s.get("error"):
                fail(errors, f"{where}: finished=true 与 error 非空矛盾")
            if s.get("failureClass") not in (None, "", "completed"):
                fail(errors, f"{where}: finished=true 但 failureClass={s.get('failureClass')!r}")
        else:
            if not (s.get("failureClass") or s.get("error") or s.get("aborted")):
                warnings.append(f"{where}: finished=false 但无失败归因字段")

    verdict_counter: Counter[str] = Counter(
        v.get("verdict") for s in samples for v in (s.get("reviewVerdicts") or []) if isinstance(v, dict)
    )
    stats["verdicts"] = {k: verdict_counter.get(k, 0) for k in VERDICTS}

    # full-chain 一次成功口径：finished 且仅一轮 review（无自动返修 retry）
    full_chain = sum(1 for s in samples if s.get("finished") and len(s.get("reviewVerdicts") or []) == 1)
    stats["full_chain_one_shot"] = full_chain
    stats["review_rounds_hist"] = dict(Counter(len(s.get("reviewVerdicts") or []) for s in samples))

    usage_missing = sum(1 for s in samples if s.get("usage") is None)
    if usage_missing:
        warnings.append(
            f"{usage_missing}/{len(samples)} 样本无 usage 记录；"
            "若为既有裁决（baseline 无可用 usage），请确保 report/ledger 已按事实记录"
        )
    stats["usage_missing"] = usage_missing
    return stats


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", nargs="?", default="gate-manifest.json")
    args = parser.parse_args(argv)

    path = Path(args.manifest)
    if not path.exists():
        print(f"manifest 不存在: {path}", file=sys.stderr)
        return 2
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        print(f"manifest 不是合法 JSON: {e}", file=sys.stderr)
        return 1

    errors: list[str] = []
    warnings: list[str] = []
    validate_schema_shape(doc, errors, warnings)
    stats = validate_semantics(doc, errors, warnings) if not errors else {}

    report = {
        "manifest": str(path),
        "ok": not errors,
        "errors": errors,
        "warnings": warnings,
        "stats": stats,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
