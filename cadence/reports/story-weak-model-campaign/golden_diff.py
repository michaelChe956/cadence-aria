#!/usr/bin/env python3
"""Compare Story Spec constraint semantics with a frozen normalized golden JSON.

Usage:
  python3 golden_diff.py --normalize path/to/story-spec.md-or-json
  python3 golden_diff.py path/to/story-spec.md-or-json golden/issue_0001.golden.json

The comparison is intentionally structural. It ignores prose and ordering but fails
on removed/added REQ, AC, or NFR IDs, required-heading changes, lost source IDs,
removed AC-to-REQ links, missing decisions, and changed decision answers/bindings.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

HEADING_RE = re.compile(r"^##\s+(.+?)\s*$", re.MULTILINE)
STABLE_ID_RE = re.compile(r"\[(REQ|AC|NFR)-([A-Za-z0-9_-]+)\]")
SOURCE_RE = re.compile(r"source\s+id\s*:\s*([^\)）\n]+)", re.IGNORECASE)
ITEM_RE = re.compile(r"^\s*[-*]\s+\*\*\[(REQ|AC|NFR)-[A-Za-z0-9_-]+\]", re.MULTILINE)
DECISION_RE = re.compile(
    r"^\s*-?\s*\*\*(author-decision-[A-Za-z0-9_-]+)\*\*\s*：?\s*(.*?)(?=^\s*-?\s*\*\*author-decision-|^##\s|\Z)",
    re.MULTILINE | re.DOTALL,
)


def fail(message: str) -> None:
    raise ValueError(message)


def read_markdown(path: Path) -> str:
    content = path.read_text(encoding="utf-8")
    if path.suffix.lower() != ".json":
        return content
    try:
        value = json.loads(content)
    except json.JSONDecodeError as error:
        fail(f"invalid JSON input: {error}")
    if not isinstance(value, dict):
        fail("JSON input must be an object containing markdown")
    for key in ("markdown", "current_markdown_preview", "story_spec_markdown"):
        candidate = value.get(key)
        if isinstance(candidate, str):
            return candidate
    fail("JSON input contains none of: markdown, current_markdown_preview, story_spec_markdown")


def sections(markdown: str) -> dict[str, str]:
    matches = list(HEADING_RE.finditer(markdown))
    result: dict[str, str] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(markdown)
        heading = match.group(1).strip()
        result[heading] = markdown[match.end() : end]
    return result


def ids_in(text: str, prefix: str) -> list[str]:
    return sorted({f"{kind}-{suffix}" for kind, suffix in STABLE_ID_RE.findall(text) if kind == prefix})


def source_ids(text: str) -> list[str]:
    found: set[str] = set()
    for source_match in SOURCE_RE.finditer(text):
        for item in re.split(r"[,，;；、]", source_match.group(1)):
            item = item.strip().strip("。.")
            if item:
                found.add(item)
    return sorted(found)


def blocks_for_ids(text: str, prefix: str) -> dict[str, str]:
    """Return each bullet-item block, stopping at any next stable-ID item or heading."""
    occurrences = list(ITEM_RE.finditer(text))
    blocks: dict[str, str] = {}
    for index, occurrence in enumerate(occurrences):
        stable_match = STABLE_ID_RE.search(occurrence.group(0))
        if stable_match is None or stable_match.group(1) != prefix:
            continue
        stable_id = f"{stable_match.group(1)}-{stable_match.group(2)}"
        next_item = occurrences[index + 1].start() if index + 1 < len(occurrences) else len(text)
        next_heading = re.search(r"^##\s+", text[occurrence.end() : next_item], re.MULTILINE)
        end = occurrence.end() + next_heading.start() if next_heading else next_item
        blocks[stable_id] = text[occurrence.start() : end]
    return blocks

def decision_answer(block: str) -> str:
    match = re.search(r"\*\*用户选择\*\*\s*：\s*(.*?)(?=\n\s*-\s*\*\*|\Z)", block, re.DOTALL)
    if not match:
        return ""
    return " ".join(match.group(1).split())


def normalize(markdown: str) -> dict[str, Any]:
    by_heading = sections(markdown)
    req_blocks = blocks_for_ids(markdown, "REQ")
    nfr_blocks = blocks_for_ids(markdown, "NFR")
    ac_blocks = blocks_for_ids(by_heading.get("成功标准", ""), "AC")
    decisions: dict[str, Any] = {}
    decision_section = by_heading.get("用户确认决策", "")
    for match in DECISION_RE.finditer(decision_section):
        decision_id, detail = match.groups()
        decisions[decision_id] = {
            "answer": decision_answer(detail),
            "bindings": {
                "req_ids": ids_in(detail, "REQ"),
                "ac_ids": ids_in(detail, "AC"),
            },
        }
    return {
        "schema_version": 1,
        "heading_set": sorted(by_heading),
        "req_ids": ids_in(markdown, "REQ"),
        "ac_ids": ids_in(markdown, "AC"),
        "nfr_ids": ids_in(markdown, "NFR"),
        "ac_req_links": {stable_id: ids_in(block, "REQ") for stable_id, block in sorted(ac_blocks.items())},
        "source_id_coverage": {
            stable_id: source_ids(block)
            for stable_id, block in sorted({**req_blocks, **nfr_blocks}.items())
        },
        "user_decisions": decisions,
    }


def difference(field: str, kind: str, expected: Any, actual: Any) -> dict[str, Any]:
    return {"field": field, "kind": kind, "expected": expected, "actual": actual}


def compare(actual: dict[str, Any], golden: dict[str, Any]) -> list[dict[str, Any]]:
    differences: list[dict[str, Any]] = []
    for field in ("heading_set", "req_ids", "ac_ids", "nfr_ids"):
        expected = set(golden.get(field, []))
        observed = set(actual.get(field, []))
        if missing := sorted(expected - observed):
            differences.append(difference(field, "forbidden_missing", missing, sorted(observed)))
        if added := sorted(observed - expected):
            differences.append(difference(field, "forbidden_added", sorted(expected), added))

    for ac_id, expected_links in sorted(golden.get("ac_req_links", {}).items()):
        observed_links = set(actual.get("ac_req_links", {}).get(ac_id, []))
        missing = sorted(set(expected_links) - observed_links)
        if missing:
            differences.append(difference(f"ac_req_links.{ac_id}", "forbidden_link_lost", missing, sorted(observed_links)))

    for stable_id, expected_sources in sorted(golden.get("source_id_coverage", {}).items()):
        observed_sources = set(actual.get("source_id_coverage", {}).get(stable_id, []))
        missing = sorted(set(expected_sources) - observed_sources)
        if missing:
            differences.append(difference(f"source_id_coverage.{stable_id}", "forbidden_source_lost", missing, sorted(observed_sources)))

    expected_decisions = golden.get("user_decisions", {})
    actual_decisions = actual.get("user_decisions", {})
    for decision_id, expected in sorted(expected_decisions.items()):
        observed = actual_decisions.get(decision_id)
        if observed is None:
            differences.append(difference(f"user_decisions.{decision_id}", "forbidden_decision_lost", expected, None))
            continue
        if observed.get("answer", "") != expected.get("answer", ""):
            differences.append(difference(f"user_decisions.{decision_id}.answer", "forbidden_decision_reversed", expected.get("answer", ""), observed.get("answer", "")))
        if observed.get("bindings") != expected.get("bindings"):
            differences.append(difference(f"user_decisions.{decision_id}.bindings", "forbidden_decision_binding_changed", expected.get("bindings"), observed.get("bindings")))
    return differences


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--normalize", metavar="SPEC", help="print normalized JSON for a markdown or JSON Story Spec")
    parser.add_argument("spec", nargs="?", help="candidate Story Spec markdown or JSON")
    parser.add_argument("golden", nargs="?", help="frozen normalized golden JSON")
    args = parser.parse_args()
    if args.normalize:
        if args.spec or args.golden:
            parser.error("--normalize accepts exactly one input")
        print(json.dumps(normalize(read_markdown(Path(args.normalize))), ensure_ascii=False, indent=2, sort_keys=True))
        return 0
    if not args.spec or not args.golden:
        parser.error("provide SPEC and GOLDEN, or use --normalize SPEC")
    actual = normalize(read_markdown(Path(args.spec)))
    try:
        golden = json.loads(Path(args.golden).read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        fail(f"invalid golden JSON: {error}")
    if golden.get("schema_version") != 1:
        fail("unsupported golden schema_version")
    differences = compare(actual, golden)
    print(json.dumps({"pass": not differences, "differences": differences}, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if not differences else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(json.dumps({"pass": False, "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        raise SystemExit(2)
