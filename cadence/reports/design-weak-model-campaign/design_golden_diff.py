#!/usr/bin/env python3
"""Compare Design constraint semantics with a frozen normalized golden JSON.

Usage:
  python3 design_golden_diff.py --normalize path/to/design.md-or-json
  python3 design_golden_diff.py path/to/design.md-or-json golden/design_0001.golden.json

The comparison intentionally ignores prose, ordering, and newly-added Design IDs.
It fails on removed required headings, existing Design IDs, upstream references, source
coverage, DEC-to-upstream links, and frozen user-decision answers or bindings.
"""

import argparse
import json
import re
import sys
from pathlib import Path


DESIGN_HEADINGS = ["设计范围", "设计决策", "公共组件", "API 契约", "数据模型", "风险", "追踪关系"]
DEC_RE = re.compile(r"\[(DEC)-([A-Za-z0-9_-]+)\]")
CMP_RE = re.compile(r"\[(CMP)-([A-Za-z0-9_-]+)\]")
API_RE = re.compile(r"\[(API)-([A-Za-z0-9_-]+)\]")
LINK_RE = re.compile(r"\[(DEC-[A-Za-z0-9_-]+)\][^\n]*?\[((?:REQ|AC|DEC)-[A-Za-z0-9_-]+)\]")
HEADING_RE = re.compile(r"^##\s+(.+?)\s*$", re.MULTILINE)
REFERENCE_RE = re.compile(r"\[(REQ|AC)-([A-Za-z0-9_-]+)\]")
ITEM_RE = re.compile(r"^\s*[-*]\s+(?:\*\*)?\[(DEC|CMP|API)-([A-Za-z0-9_-]+)\](?:\*\*)?", re.MULTILINE)
SOURCE_RE = re.compile(r"source\s+id\s*:\s*([^\)）\n]+)", re.IGNORECASE)
AUTHOR_DECISION_RE = re.compile(
    r"^\s*[-*]\s+(?:\*\*)?(author-decision-[A-Za-z0-9_-]+)(?:\*\*)?\s*：?\s*"
    r"(.*?)(?=^\s*[-*]\s+(?:(?:\*\*)?author-decision-[A-Za-z0-9_-]+(?:\*\*)?|(?:\*\*)?\[(?:DEC|CMP|API)-[A-Za-z0-9_-]+\](?:\*\*)?)|^##\s|\Z)",
    re.MULTILINE | re.DOTALL,
)


def fail(message):
    raise ValueError(message)


def read_markdown(path):
    """Read markdown directly or from the Design SpecVersionRecord markdown field."""
    content = path.read_text(encoding="utf-8")
    if path.suffix.lower() != ".json":
        return content
    try:
        value = json.loads(content)
    except json.JSONDecodeError as error:
        fail(f"invalid JSON input: {error}")
    if not isinstance(value, dict):
        fail("JSON input must be an object containing SpecVersionRecord.markdown")
    candidate = value.get("markdown")
    if isinstance(candidate, str):
        return candidate
    fail("JSON input contains no SpecVersionRecord.markdown")


def without_fenced_code(markdown):
    """Exclude fenced code blocks before looking for structural Markdown tokens."""
    kept = []
    fence_character = ""
    fence_length = 0
    for line in markdown.splitlines(keepends=True):
        marker = re.match(r"^[ \t]*(`{3,}|~{3,})", line)
        if not fence_character:
            if marker:
                fence_character = marker.group(1)[0]
                fence_length = len(marker.group(1))
            else:
                kept.append(line)
            continue
        closing = re.match(r"^[ \t]*(%s{%d,})[ \t]*(?:\n|$)" % (re.escape(fence_character), fence_length), line)
        if closing:
            fence_character = ""
            fence_length = 0
    return "".join(kept)


def sections(markdown):
    matches = list(HEADING_RE.finditer(markdown))
    result = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(markdown)
        result[match.group(1).strip()] = markdown[match.end() : end]
    return result


def ids_in(text, expression):
    return sorted({f"{kind}-{suffix}" for kind, suffix in expression.findall(text)})


def referenced_ids(text, kind):
    return sorted({f"{prefix}-{suffix}" for prefix, suffix in REFERENCE_RE.findall(text) if prefix == kind})


def source_ids(text):
    found = set()
    for source_match in SOURCE_RE.finditer(text):
        for item in re.split(r"[,，;；、]", source_match.group(1)):
            item = item.strip().strip("。.")
            if item:
                found.add(item)
    return sorted(found)


def blocks_for_ids(text):
    """Group list-item blocks by their Design ID without mistaking prose for an item."""
    occurrences = list(ITEM_RE.finditer(text))
    blocks = {}
    for index, occurrence in enumerate(occurrences):
        kind, suffix = occurrence.groups()
        stable_id = f"{kind}-{suffix}"
        next_item = occurrences[index + 1].start() if index + 1 < len(occurrences) else len(text)
        next_heading = re.search(r"^##\s+", text[occurrence.end() : next_item], re.MULTILINE)
        end = occurrence.end() + next_heading.start() if next_heading else next_item
        blocks.setdefault(stable_id, []).append(text[occurrence.start() : end])
    return blocks


def decision_answer(block):
    match = re.search(r"\*\*用户选择\*\*\s*：\s*(.*?)(?=\n\s*-\s+|\Z)", block, re.DOTALL)
    if not match:
        return ""
    return " ".join(match.group(1).split())


def decision_bindings(block):
    return {
        "dec_ids": ids_in(block, DEC_RE),
        "req_ids": referenced_ids(block, "REQ"),
        "ac_ids": referenced_ids(block, "AC"),
    }


def decision_sections(by_heading):
    return "\n".join(by_heading.get(heading, "") for heading in ("设计决策", "追踪关系"))


def user_decisions(by_heading):
    """Use author-decision keys first, then DEC keys for bare choice records.

    A bare DEC key is deliberately retained so compare() can map it to a frozen
    author-decision record by its immutable ``dec_id``.
    """
    relevant = decision_sections(by_heading)
    decisions = {}
    author_dec_ids = set()
    for match in AUTHOR_DECISION_RE.finditer(relevant):
        key, detail = match.groups()
        bindings = decision_bindings(detail)
        dec_id = bindings["dec_ids"][0] if bindings["dec_ids"] else ""
        decisions[key] = {
            "answer": decision_answer(detail),
            "dec_id": dec_id,
            "bindings": bindings,
        }
        if dec_id:
            author_dec_ids.add(dec_id)

    for dec_id, blocks in sorted(blocks_for_ids(relevant).items()):
        if not dec_id.startswith("DEC-") or dec_id in author_dec_ids:
            continue
        detail = "\n".join(blocks)
        decisions[dec_id] = {
            "answer": decision_answer(detail),
            "dec_id": dec_id,
            "bindings": decision_bindings(detail),
        }
    return decisions


def normalize(markdown):
    markdown = without_fenced_code(markdown)
    by_heading = sections(markdown)
    item_blocks = blocks_for_ids(markdown)
    dec_req_links = {}
    for dec_id, target_id in LINK_RE.findall(markdown):
        dec_req_links.setdefault(dec_id, set()).add(target_id)
    return {
        "schema_version": 1,
        "workspace_type": "design",
        "heading_set": sorted(by_heading),
        "dec_ids": ids_in(markdown, DEC_RE),
        "cmp_ids": ids_in(markdown, CMP_RE),
        "api_ids": ids_in(markdown, API_RE),
        "referenced_req_ids": referenced_ids(markdown, "REQ"),
        "referenced_ac_ids": referenced_ids(markdown, "AC"),
        "dec_req_links": {stable_id: sorted(targets) for stable_id, targets in sorted(dec_req_links.items())},
        "source_id_coverage": {
            stable_id: source_ids("\n".join(blocks))
            for stable_id, blocks in sorted(item_blocks.items())
        },
        "user_decisions": user_decisions(by_heading),
    }


def difference(field, kind, expected, actual):
    return {"field": field, "kind": kind, "expected": expected, "actual": actual}


def compare(actual, golden):
    differences = []
    for field in (
        "heading_set",
        "dec_ids",
        "cmp_ids",
        "api_ids",
        "referenced_req_ids",
        "referenced_ac_ids",
    ):
        expected = set(golden.get(field, []))
        observed = set(actual.get(field, []))
        if missing := sorted(expected - observed):
            differences.append(difference(field, "forbidden_missing", missing, sorted(observed)))

    required_headings = set(DESIGN_HEADINGS)
    unexpected_headings = sorted(set(actual.get("heading_set", [])) - required_headings)
    if unexpected_headings:
        differences.append(
            difference("heading_set", "forbidden_added", sorted(required_headings), unexpected_headings)
        )

    for dec_id, expected_links in sorted(golden.get("dec_req_links", {}).items()):
        observed_links = set(actual.get("dec_req_links", {}).get(dec_id, []))
        missing = sorted(set(expected_links) - observed_links)
        if missing:
            differences.append(
                difference(f"dec_req_links.{dec_id}", "forbidden_link_lost", missing, sorted(observed_links))
            )

    for stable_id, expected_sources in sorted(golden.get("source_id_coverage", {}).items()):
        observed_sources = set(actual.get("source_id_coverage", {}).get(stable_id, []))
        missing = sorted(set(expected_sources) - observed_sources)
        if missing:
            differences.append(
                difference(
                    f"source_id_coverage.{stable_id}",
                    "forbidden_source_lost",
                    missing,
                    sorted(observed_sources),
                )
            )

    expected_decisions = golden.get("user_decisions", {})
    actual_decisions = actual.get("user_decisions", {})
    for decision_key, expected in sorted(expected_decisions.items()):
        observed = actual_decisions.get(decision_key)
        if observed is None and expected.get("dec_id"):
            observed = next(
                (
                    candidate
                    for candidate in actual_decisions.values()
                    if candidate.get("dec_id") == expected.get("dec_id")
                ),
                None,
            )
        if observed is None:
            differences.append(
                difference(f"user_decisions.{decision_key}", "forbidden_decision_lost", expected, None)
            )
            continue
        if observed.get("answer", "") != expected.get("answer", ""):
            differences.append(
                difference(
                    f"user_decisions.{decision_key}.answer",
                    "forbidden_decision_reversed",
                    expected.get("answer", ""),
                    observed.get("answer", ""),
                )
            )
        if observed.get("bindings") != expected.get("bindings"):
            differences.append(
                difference(
                    f"user_decisions.{decision_key}.bindings",
                    "forbidden_decision_binding_changed",
                    expected.get("bindings"),
                    observed.get("bindings"),
                )
            )
    return differences


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--normalize", metavar="SPEC", help="print normalized JSON for a markdown or JSON Design Spec")
    parser.add_argument("spec", nargs="?", help="candidate Design Spec markdown or JSON")
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
    if golden.get("schema_version") != 1 or golden.get("workspace_type") != "design":
        fail("unsupported Design golden schema")
    differences = compare(actual, golden)
    print(json.dumps({"pass": not differences, "differences": differences}, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if not differences else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(json.dumps({"pass": False, "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        raise SystemExit(2)
