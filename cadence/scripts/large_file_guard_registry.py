#!/usr/bin/env python3
"""行数门禁登记表收口助手：算出当前可从 GRANDFATHERED 移除的条目。

规则（见 tests/it_core/large_file_guard.rs 顶部注释）：
- 硬线 1200 行；登记表条目是「祖父条款」豁免，上限锁定为登记日行数，任何增长都报红
- 登记表只允许删项：文件拆分达标（≤1200）或文件已不存在 → 该条目可移除
- 仍 >1200 的条目必须保留；若其实测行数 > 登记上限 → 报「增长违规」，必须修

用法：python3 cadence/scripts/large_file_guard_registry.py [--apply]
不带 --apply 只报告；带 --apply 会就地改写 large_file_guard.rs 的 GRANDFATHERED 数组。
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUARD = ROOT / "tests/it_core/large_file_guard.rs"
HARD_LINE = 1200


def parse_registry(text: str):
    """返回 [(path, limit)]，兼容单行与多行两种条目写法。"""
    return [
        (m.group(1), int(m.group(2)))
        for m in re.finditer(r'\(\s*"([^"]+)"\s*,\s*(\d+)\s*,?\s*\)', text)
    ]


def line_count(rel: str):
    p = ROOT / rel
    if not p.is_file():
        return None
    return len(p.read_text(errors="ignore").splitlines())


def main() -> int:
    apply = "--apply" in sys.argv
    text = GUARD.read_text()
    entries = parse_registry(text)

    removable, keep, growth_violation = [], [], []
    for rel, limit in entries:
        n = line_count(rel)
        if n is None:
            removable.append((rel, limit, "文件已不存在"))
        elif n <= HARD_LINE:
            removable.append((rel, limit, f"实测 {n} ≤ 硬线 {HARD_LINE}"))
        elif n > limit:
            growth_violation.append((rel, limit, n))
            keep.append((rel, limit, f"实测 {n} > 登记上限 {limit}（增长违规，必须修）"))
        else:
            keep.append((rel, limit, f"实测 {n} ≤ 登记上限 {limit}"))

    print(f"登记表条目总数：{len(entries)}")
    print(f"\n=== 可移除（{len(removable)}）===")
    for rel, limit, why in removable:
        print(f"  {rel}（登记 {limit}）—— {why}")
    print(f"\n=== 必须保留（{len(keep)}）===")
    for rel, limit, why in keep:
        print(f"  {rel}（登记 {limit}）—— {why}")

    if growth_violation:
        print("\n=== ⚠ 增长违规（必须修，不得自行上调登记上限）===")
        for rel, limit, n in growth_violation:
            print(f"  {rel}：登记 {limit} → 实测 {n}")
        return 1

    if apply and removable:
        drop = {rel for rel, _, _ in removable}
        lines = text.splitlines()
        out = []
        i = 0
        removed = 0
        single = re.compile(r'^\s*\(\s*"([^"]+)"\s*,\s*(\d+)\s*,?\s*\),?\s*$')
        while i < len(lines):
            line = lines[i]
            m = single.match(line)
            if m:
                if m.group(1) in drop:
                    removed += 1
                    i += 1
                    continue
                out.append(line)
                i += 1
                continue
            # 多行写法：`(` / 路径 / 数字 / `),`
            if line.strip() == "(":
                span = []
                j = i + 1
                while j < len(lines) and lines[j].strip() != "),":
                    span.append(lines[j])
                    j += 1
                span.append(lines[j]) if j < len(lines) else None
                joined = "\n".join(span)
                mp = re.search(r'"([^"]+)"\s*,\s*\n\s*(\d+)\s*,', joined)
                if mp and mp.group(1) in drop:
                    removed += 1
                    i = j + 1
                    continue
                out.append(line)
                out.extend(span)
                i = j + 1
                continue
            out.append(line)
            i += 1
        GUARD.write_text("\n".join(out) + "\n")
        print(f"\n已从登记表移除 {removed} 项（保留全部注释与既有格式）；请跑 `cargo fmt --all` 复查格式")
    elif removable:
        print("\n（未改动登记表；加 --apply 执行移除）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
