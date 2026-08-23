#!/usr/bin/env python3
"""Validate strict Design weak-model campaign manifests.

Usage:
  python3 validate_manifest.py gate-manifest.json [--paired baseline-manifest.json]

The validator uses only the Python standard library.  It prints one JSON object with
``ok``, ``errors``, ``warnings``, and ``stats`` and returns 0 for a valid manifest,
1 for validation failures, and 2 for command-line/input errors.

Semantic rules in addition to ``manifest.schema.json``:
* samples carry complete Design-campaign metadata and exactly one usage disclosure;
* case identities are unique and internally consistent;
* terminal status, review verdicts, and failure metadata cannot contradict each other;
* paired baseline/revised runs have one compatible sample per
  (provider, shape_id, repetition_id) group; invalid groups are excluded as a whole;
* checked-in corpus and golden SHA-256 digests still describe their files; and
* boundary samples report false-positive/false-negative counts and rule-of-three
  95% upper bounds when no relevant failure is observed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable

SCHEMA = "gate-campaign/1"
RUN_KINDS = ("baseline", "revised")
STRATEGIES = ("fresh", "resume")
VERDICTS = ("pass", "revise", "needs_human")
BOUNDARY_KINDS = ("abstract-positive", "violation-negative")
DIGEST_LINE = re.compile(r"^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$")


def fail(errors: list[str], message: str) -> None:
    """Record one user-actionable validation failure."""
    errors.append(message)


def is_integer(value: Any) -> bool:
    """Return whether value is a JSON integer (booleans are not integers here)."""
    return isinstance(value, int) and not isinstance(value, bool)


def is_number(value: Any) -> bool:
    """Return whether value is a JSON number (booleans are not numbers here)."""
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def non_empty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def rule_of_three_upper_bound(failures: int, observations: int) -> float | None:
    """Return the 95% rule-of-three upper bound for zero failures, otherwise None.

    With zero observed failures in ``n`` observations, the conventional approximate
    one-sided 95% upper bound is 3/n.  A bound is undefined for no observations and
    intentionally not reported after one or more observed failures because the
    requested rule-of-three form applies only to the zero-failure case.
    """
    if failures != 0 or observations <= 0:
        return None
    return 3 / observations


def validate_model_identity(value: Any, where: str, errors: list[str]) -> None:
    if not isinstance(value, dict):
        fail(errors, f"{where} 必须是 object")
        return
    for key in ("provider", "model", "model_version"):
        if not non_empty_string(value.get(key)):
            fail(errors, f"{where}.{key} 必须为非空字符串")


def validate_usage(sample: dict[str, Any], where: str, errors: list[str]) -> None:
    has_usage = "usage" in sample
    has_unavailable = "usage_unavailable" in sample
    unavailable = sample.get("usage_unavailable")
    if not has_usage and not has_unavailable:
        fail(errors, f"{where} 缺少 usage 或 usage_unavailable（Design campaign 不允许未披露 usage）")
        return
    if has_usage and has_unavailable:
        fail(errors, f"{where} 的 usage 与 usage_unavailable 必须二选一")
        return
    if has_unavailable:
        if unavailable is not True:
            fail(errors, f"{where}.usage_unavailable 必须为 true")
        return

    usage = sample.get("usage")
    if not isinstance(usage, dict):
        fail(errors, f"{where}.usage 必须是 object")
        return
    for key in ("input_tokens", "cache_read_tokens"):
        value = usage.get(key)
        if not is_integer(value) or value < 0:
            fail(errors, f"{where}.usage.{key} 必须为非负整数")
    fresh_or_resume = usage.get("fresh_or_resume")
    if fresh_or_resume is not None:
        if fresh_or_resume not in STRATEGIES:
            fail(errors, f"{where}.usage.fresh_or_resume 必须为 fresh|resume")
        elif fresh_or_resume != sample.get("strategy"):
            fail(errors, f"{where}.usage.fresh_or_resume 与 strategy 不一致")


def validate_schema_shape(doc: Any, errors: list[str], warnings: list[str]) -> None:
    """Perform stdlib-only JSON-schema-equivalent checks for required campaign fields."""
    if not isinstance(doc, dict):
        fail(errors, "manifest 顶层必须是 JSON object")
        return

    for key in ("schema", "run_kind", "samples"):
        if key not in doc:
            fail(errors, f"顶层缺少必填字段 {key}")
    if doc.get("schema") != SCHEMA:
        fail(errors, f"schema 必须为 {SCHEMA!r}，实际 {doc.get('schema')!r}")
    if doc.get("run_kind") not in RUN_KINDS:
        fail(errors, f"run_kind 必须为 baseline|revised，实际 {doc.get('run_kind')!r}")

    samples = doc.get("samples")
    if not isinstance(samples, list) or not samples:
        fail(errors, "samples 必须为非空数组")
        return

    unavailable_indices: list[int] = []
    for index, sample in enumerate(samples):
        where = f"samples[{index}]"
        if not isinstance(sample, dict):
            fail(errors, f"{where} 必须是 object")
            continue

        required = (
            "provider", "model", "model_version", "role_provider", "strategy",
            "resume_available", "shape_id", "shape_file", "repetition", "case_id",
            "startedAt", "finishedAt", "issueId", "sessionId", "storySpecId",
            "designSpecId", "finished", "reviewVerdicts", "failureClass", "elapsedSec",
            "boundary_kind",
        )
        for key in required:
            if key not in sample:
                fail(errors, f"{where} 缺少必填字段 {key}")

        for key in ("provider", "model", "model_version", "shape_id", "shape_file", "repetition", "startedAt"):
            if not non_empty_string(sample.get(key)):
                fail(errors, f"{where}.{key} 必须为非空字符串")
        if non_empty_string(sample.get("shape_id")) and not re.fullmatch(r"[0-9]{2}", sample["shape_id"]):
            fail(errors, f"{where}.shape_id 必须匹配两位数字")
        if non_empty_string(sample.get("repetition")) and not sample["repetition"].isdigit():
            fail(errors, f"{where}.repetition 必须为数字字符串")
        if sample.get("strategy") not in STRATEGIES:
            fail(errors, f"{where}.strategy 必须为 fresh|resume")
        if not isinstance(sample.get("resume_available"), bool):
            fail(errors, f"{where}.resume_available 必须为 boolean")
        if sample.get("strategy") == "resume" and sample.get("resume_available") is not True:
            fail(errors, f"{where}: strategy=resume 但 resume_available 不是 true")
        if not isinstance(sample.get("finished"), bool):
            fail(errors, f"{where}.finished 必须为 boolean")
        if not is_number(sample.get("elapsedSec")) or sample.get("elapsedSec", -1) < 0:
            fail(errors, f"{where}.elapsedSec 必须为非负 number")

        for key in ("finishedAt", "issueId", "sessionId", "storySpecId", "designSpecId", "failureClass"):
            if sample.get(key) is not None and not isinstance(sample.get(key), str):
                fail(errors, f"{where}.{key} 必须为 string 或 null")
        if sample.get("finished") is True and not non_empty_string(sample.get("finishedAt")):
            fail(errors, f"{where}: finished=true 但 finishedAt 为空")

        role_provider = sample.get("role_provider")
        if not isinstance(role_provider, dict):
            fail(errors, f"{where}.role_provider 必须是 object")
        else:
            validate_model_identity(role_provider.get("author"), f"{where}.role_provider.author", errors)
            validate_model_identity(role_provider.get("reviewer"), f"{where}.role_provider.reviewer", errors)

        case_id = sample.get("case_id")
        if not isinstance(case_id, dict):
            fail(errors, f"{where}.case_id 必须是 object")
        else:
            if not non_empty_string(case_id.get("shape_id")):
                fail(errors, f"{where}.case_id.shape_id 必须为非空字符串")
            elif not re.fullmatch(r"[0-9]{2}", case_id["shape_id"]):
                fail(errors, f"{where}.case_id.shape_id 必须匹配两位数字")
            if not is_integer(case_id.get("repetition_id")) or case_id.get("repetition_id", 0) < 1:
                fail(errors, f"{where}.case_id.repetition_id 必须为正整数")
            if "round" in case_id and (
                not is_integer(case_id["round"]) or case_id["round"] < 1
            ):
                fail(errors, f"{where}.case_id.round 必须为正整数")

        review_verdicts = sample.get("reviewVerdicts")
        if not isinstance(review_verdicts, list):
            fail(errors, f"{where}.reviewVerdicts 必须为数组")
        else:
            for verdict_index, verdict in enumerate(review_verdicts):
                verdict_where = f"{where}.reviewVerdicts[{verdict_index}]"
                if not isinstance(verdict, dict):
                    fail(errors, f"{verdict_where} 必须是 object")
                    continue
                if verdict.get("verdict") not in VERDICTS:
                    fail(errors, f"{verdict_where}.verdict 非法: {verdict.get('verdict')!r}")
                if not is_number(verdict.get("el")) or verdict.get("el", -1) < 0:
                    fail(errors, f"{verdict_where}.el 必须为非负 number")

        if sample.get("boundary_kind") not in (*BOUNDARY_KINDS, None):
            fail(errors, f"{where}.boundary_kind 必须为 abstract-positive|violation-negative|null")
        validate_usage(sample, where, errors)
        if sample.get("usage_unavailable") is True:
            unavailable_indices.append(index)

    if unavailable_indices:
        warnings.append(
            f"{len(unavailable_indices)}/{len(samples)} 样本声明 usage_unavailable；这些样本不进入 token 聚合"
        )


def sample_case_key(sample: dict[str, Any], run_kind: str) -> tuple[Any, ...]:
    case_id = sample.get("case_id") if isinstance(sample.get("case_id"), dict) else {}
    return (
        run_kind,
        sample.get("provider"),
        case_id.get("shape_id"),
        case_id.get("repetition_id"),
        case_id.get("round"),
    )


def pair_key(sample: dict[str, Any]) -> tuple[Any, Any, Any]:
    case_id = sample.get("case_id") if isinstance(sample.get("case_id"), dict) else {}
    return (
        sample.get("provider"),
        case_id.get("shape_id"),
        case_id.get("repetition_id"),
    )


def terminal_is_accepted(sample: dict[str, Any]) -> bool:
    """Return whether the final terminal state accepted the Design candidate."""
    verdicts = sample.get("reviewVerdicts")
    return bool(
        sample.get("finished") is True
        and sample.get("failureClass") in (None, "", "completed")
        and not sample.get("error")
        and isinstance(verdicts, list)
        and verdicts
        and isinstance(verdicts[-1], dict)
        and verdicts[-1].get("verdict") == "pass"
    )


def validate_semantics(doc: dict[str, Any], errors: list[str], warnings: list[str]) -> dict[str, Any]:
    """Check per-manifest identity, terminal, usage, and boundary semantics."""
    samples = doc.get("samples")
    if not isinstance(samples, list):
        return {}
    run_kind = doc.get("run_kind")
    stats: dict[str, Any] = {}

    keys: list[tuple[Any, ...]] = []
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict):
            continue
        where = f"samples[{index}]"
        case_id = sample.get("case_id") if isinstance(sample.get("case_id"), dict) else {}
        if sample.get("shape_id") != case_id.get("shape_id"):
            fail(
                errors,
                f"{where}: shape_id {sample.get('shape_id')!r} 与 "
                f"case_id.shape_id {case_id.get('shape_id')!r} 不一致",
            )
        try:
            if int(str(sample.get("repetition"))) != case_id.get("repetition_id"):
                fail(errors, f"{where}: repetition 与 case_id.repetition_id 不一致")
        except (TypeError, ValueError):
            fail(errors, f"{where}: repetition 无法解析为整数")
        keys.append(sample_case_key(sample, run_kind))

    duplicate_keys = [key for key, count in Counter(keys).items() if count > 1]
    for key in duplicate_keys:
        count = keys.count(key)
        fail(errors, f"case_id 重复 {count} 次: {key}")
    stats["case_id_duplicates"] = len(duplicate_keys)
    stats["samples_per_provider"] = dict(Counter(
        sample.get("provider", "?") for sample in samples if isinstance(sample, dict)
    ))

    finished_count = 0
    unavailable_count = 0
    reported_samples = 0
    retry_samples = 0
    input_tokens = 0
    cache_read_tokens = 0
    retry_input_tokens = 0
    retry_cache_read_tokens = 0
    verdict_counter: Counter[str] = Counter()
    review_histogram: Counter[int] = Counter()
    one_shot = 0
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict):
            continue
        where = f"samples[{index}]"
        verdicts = sample.get("reviewVerdicts") if isinstance(sample.get("reviewVerdicts"), list) else []
        if sample.get("finished") is True:
            finished_count += 1
            if not verdicts:
                fail(errors, f"{where}: finished=true 但 reviewVerdicts 为空")
            if sample.get("error"):
                fail(errors, f"{where}: finished=true 与 error 非空矛盾")
            if sample.get("failureClass") not in (None, "", "completed"):
                fail(errors, f"{where}: finished=true 但 failureClass={sample.get('failureClass')!r}")
        elif not (sample.get("failureClass") or sample.get("error") or sample.get("aborted")):
            warnings.append(f"{where}: finished=false 但无失败归因字段")

        for verdict in verdicts:
            if isinstance(verdict, dict) and verdict.get("verdict") in VERDICTS:
                verdict_counter[verdict["verdict"]] += 1
        review_histogram[len(verdicts)] += 1
        if sample.get("finished") is True and len(verdicts) == 1:
            one_shot += 1

        if sample.get("usage_unavailable") is True:
            unavailable_count += 1
        elif isinstance(sample.get("usage"), dict):
            usage = sample["usage"]
            sample_input_tokens = usage.get("input_tokens", 0)
            sample_cache_read_tokens = usage.get("cache_read_tokens", 0)
            if sample_input_tokens == 0 and sample_cache_read_tokens == 0:
                warnings.append(f"{where}: usage 记录为 0 token，请确认不是采集缺失")
            case_id = sample.get("case_id") if isinstance(sample.get("case_id"), dict) else {}
            is_retry = case_id.get("round", 1) > 1
            if is_retry:
                retry_samples += 1
                retry_input_tokens += sample_input_tokens
                retry_cache_read_tokens += sample_cache_read_tokens
            else:
                reported_samples += 1
                input_tokens += sample_input_tokens
                cache_read_tokens += sample_cache_read_tokens

    stats["finished"] = finished_count
    stats["verdicts"] = {verdict: verdict_counter.get(verdict, 0) for verdict in VERDICTS}
    stats["full_chain_one_shot"] = one_shot
    stats["review_rounds_hist"] = dict(review_histogram)
    stats["usage"] = {
        "reported_samples": reported_samples,
        "unavailable_samples": unavailable_count,
        "input_tokens": input_tokens,
        "cache_read_tokens": cache_read_tokens,
        "retry_samples_excluded": retry_samples,
        "retry_input_tokens_excluded": retry_input_tokens,
        "retry_cache_read_tokens_excluded": retry_cache_read_tokens,
    }
    stats["boundary"] = boundary_stats(samples)
    return stats


def boundary_stats(samples: Iterable[Any]) -> dict[str, dict[str, Any]]:
    """Summarize expected-positive and expected-negative boundary behavior.

    ``abstract-positive`` samples should be accepted, so a non-acceptance is a false
    negative.  ``violation-negative`` samples should be rejected, so an acceptance is
    a false positive.  The rule-of-three value is emitted only for zero observed
    relevant failures.
    """
    grouped: dict[str, list[dict[str, Any]]] = {kind: [] for kind in BOUNDARY_KINDS}
    for sample in samples:
        if isinstance(sample, dict) and sample.get("boundary_kind") in grouped:
            grouped[sample["boundary_kind"]].append(sample)

    positives = grouped["abstract-positive"]
    negatives = grouped["violation-negative"]
    false_negatives = sum(1 for sample in positives if not terminal_is_accepted(sample))
    false_positives = sum(1 for sample in negatives if terminal_is_accepted(sample))
    return {
        "abstract-positive": {
            "observations": len(positives),
            "false_negatives": false_negatives,
            "false_negative_upper_95": rule_of_three_upper_bound(false_negatives, len(positives)),
        },
        "violation-negative": {
            "observations": len(negatives),
            "false_positives": false_positives,
            "false_positive_upper_95": rule_of_three_upper_bound(false_positives, len(negatives)),
        },
    }


def validate_pairing(
    first_doc: dict[str, Any], second_doc: dict[str, Any], errors: list[str]
) -> dict[str, Any]:
    """Validate and account for complete baseline/revised pairing groups.

    A group is accepted only when exactly one baseline and one revised sample exist
    under its (provider, shape_id, repetition_id) key and their optional ``round``
    plus required ``strategy`` values match.  Any malformed group is counted as an
    excluded whole group; no partial pair is retained in the pairing statistics.
    """
    documents = {first_doc.get("run_kind"): first_doc, second_doc.get("run_kind"): second_doc}
    if set(documents) != set(RUN_KINDS):
        fail(errors, "--paired 必须提供一份 baseline 与一份 revised manifest")
        return {"accepted_groups": 0, "excluded_groups": 0, "groups": 0}

    grouped: dict[str, defaultdict[tuple[Any, Any, Any], list[dict[str, Any]]]] = {
        kind: defaultdict(list) for kind in RUN_KINDS
    }
    for kind, doc in documents.items():
        samples = doc.get("samples")
        if not isinstance(samples, list):
            continue
        for sample in samples:
            if isinstance(sample, dict):
                grouped[kind][pair_key(sample)].append(sample)

    all_keys = sorted(
        set(grouped["baseline"]) | set(grouped["revised"]),
        key=lambda key: tuple(str(part) for part in key),
    )
    accepted_groups = 0
    excluded_groups = 0
    for key in all_keys:
        baseline_samples = grouped["baseline"].get(key, [])
        revised_samples = grouped["revised"].get(key, [])
        group_valid = True
        if not baseline_samples or not revised_samples:
            missing = "baseline" if not baseline_samples else "revised"
            fail(errors, f"配对缺边 {key}: 缺少 {missing}")
            group_valid = False
        if len(baseline_samples) != 1 or len(revised_samples) != 1:
            fail(
                errors,
                f"重复 pair/round {key}: baseline={len(baseline_samples)}, revised={len(revised_samples)}；整组剔除",
            )
            group_valid = False
        if group_valid:
            baseline = baseline_samples[0]
            revised = revised_samples[0]
            baseline_case = baseline.get("case_id") if isinstance(baseline.get("case_id"), dict) else {}
            revised_case = revised.get("case_id") if isinstance(revised.get("case_id"), dict) else {}
            if baseline_case.get("round") != revised_case.get("round"):
                fail(errors, f"配对 round 不一致 {key}: {baseline_case.get('round')!r} != {revised_case.get('round')!r}；整组剔除")
                group_valid = False
            if baseline.get("model_version") != revised.get("model_version"):
                fail(
                    errors,
                    f"配对 model_version 不一致 {key}: "
                    f"{baseline.get('model_version')!r} != {revised.get('model_version')!r}；整组剔除",
                )
                group_valid = False
            if baseline.get("strategy") != revised.get("strategy"):
                fail(errors, f"配对 strategy 不一致 {key}: {baseline.get('strategy')!r} != {revised.get('strategy')!r}；整组剔除")
                group_valid = False
        if group_valid:
            accepted_groups += 1
        else:
            excluded_groups += 1

    return {
        "groups": len(all_keys),
        "accepted_groups": accepted_groups,
        "excluded_groups": excluded_groups,
    }


def parse_digest_file(path: Path, campaign_root: Path, errors: list[str]) -> list[tuple[Path, str]]:
    """Parse one tolerant digest list and resolve paths without shelling out."""
    if not path.is_file():
        fail(errors, f"digest 文件不存在: {path}")
        return []
    entries: list[tuple[Path, str]] = []
    for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        match = DIGEST_LINE.fullmatch(line)
        if not match:
            # Existing digest ledgers include human-readable annotation lines that
            # are not prefixed with '#'.  Only well-formed checksum entries are
            # actionable; tolerate every other line as commentary.
            continue
        expected, raw_name = match.groups()
        name = Path(raw_name)
        if name.is_absolute() or ".." in name.parts:
            fail(errors, f"digest 路径非法: {path}:{line_number}: {raw_name!r}")
            continue
        # Golden lists conventionally use campaign-root-relative entries while corpus
        # lists conventionally use entries relative to corpus/.  Accept both forms.
        candidates = (campaign_root / name, path.parent / name)
        target = next((candidate for candidate in candidates if candidate.is_file()), candidates[0])
        entries.append((target, expected.lower()))
    return entries


def validate_digests(campaign_root: Path, errors: list[str]) -> dict[str, int]:
    """Verify corpus and golden SHA-256 lists, ignoring comments and blank lines."""
    checked = 0
    for relative in (Path("corpus/digests.txt"), Path("golden/digests.txt")):
        digest_file = campaign_root / relative
        for target, expected in parse_digest_file(digest_file, campaign_root, errors):
            if not target.is_file():
                fail(errors, f"digest 对应文件不存在: {target}")
                continue
            actual = hashlib.sha256(target.read_bytes()).hexdigest()
            checked += 1
            if actual != expected:
                fail(errors, f"digest 不匹配: {target}（期望 {expected}，实际 {actual}）")
    return {"checked": checked}


def validate_manifests(
    doc: Any,
    *,
    paired_doc: Any | None = None,
    campaign_root: Path | None = None,
) -> dict[str, Any]:
    """Return a JSON-serializable validation report for one or two manifest objects."""
    errors: list[str] = []
    warnings: list[str] = []
    validate_schema_shape(doc, errors, warnings)
    stats: dict[str, Any] = {}
    if isinstance(doc, dict) and not errors:
        stats = validate_semantics(doc, errors, warnings)

    if paired_doc is not None:
        paired_errors: list[str] = []
        paired_warnings: list[str] = []
        validate_schema_shape(paired_doc, paired_errors, paired_warnings)
        errors.extend(f"paired manifest: {message}" for message in paired_errors)
        warnings.extend(f"paired manifest: {message}" for message in paired_warnings)
        if isinstance(paired_doc, dict) and not paired_errors:
            paired_stats = validate_semantics(paired_doc, errors, warnings)
            stats["paired_manifest"] = paired_stats
        if isinstance(doc, dict) and isinstance(paired_doc, dict) and not paired_errors:
            stats["pairing"] = validate_pairing(doc, paired_doc, errors)

    root = campaign_root if campaign_root is not None else Path(__file__).resolve().parent
    stats["digests"] = validate_digests(root, errors)
    return {"ok": not errors, "errors": errors, "warnings": warnings, "stats": stats}


def load_manifest(path: Path) -> Any | None:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        print(f"manifest 不存在: {path}", file=sys.stderr)
        return None
    except json.JSONDecodeError as error:
        print(f"manifest 不是合法 JSON: {path}: {error}", file=sys.stderr)
        return None
    # Keep a valid JSON report for a top-level schema failure; loading itself still
    # succeeded even when the JSON value is not an object.
    return data


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", nargs="?", default="gate-manifest.json")
    parser.add_argument("--paired", metavar="BASELINE_MANIFEST", help="paired baseline/revised manifest")
    args = parser.parse_args(argv)

    manifest_path = Path(args.manifest)
    doc = load_manifest(manifest_path)
    if doc is None:
        return 2
    paired_doc: Any | None = None
    if args.paired:
        paired_doc = load_manifest(Path(args.paired))
        if paired_doc is None:
            return 2

    report = validate_manifests(doc, paired_doc=paired_doc)
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
