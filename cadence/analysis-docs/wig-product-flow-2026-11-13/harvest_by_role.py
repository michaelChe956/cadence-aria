#!/usr/bin/env python3
"""按角色拆分收割 WIG 真实链路的每个 run，并输出 before/after 方向性对照。

与旧版的关键差异（纠正既有错误归因，见同目录 README）：
  1. **绑定 manifest 是唯一采样来源**：每个 run 目录必须有 run-manifest.json
     （run_arms.py 写出：tag/arm/binary sha256/issue/plan_session/status/valid/
     输入身份复核）。不再硬编码 issue_0001，也不再用目录猜测归属。
  2. **无效样本显式排除**：manifest.valid=false（超时/异常/输入污染/无真实
     provider 启动账目）的 run 列表打印，绝不混入对照。
  3. **总耗时 = sum(节点时长)**，不是墙钟起止差——timeline 节点可重叠/等待，
     两者不可互换，本装置只有前者有 durable 来源。
  4. **usage 缺值记 None**：不补 0；中位数只对观测到的事件计算，并标注
     「缺 usage 的 run 数」。
  5. **first-token time 标不可测**：durable 产物（timeline node details 的
     execution_events 无时间戳，streaming_content 无 token 时刻）没有首 token
     时间来源，本装置不测、不估。
  6. **置换检验 = 精确枚举 C(8,4)=70 个分组**（n=4/臂），分辨率下限 1/70；
     只作方向性参考，不作显著性判定，不做多重检验校正。
  7. **输入来源 digest 直接取自 manifest**（`issue_input_sha256` /
     `story_markdown_sha256` / `story_version_ref` / `design_markdown_sha256` /
     `design_version_ref` / `seed_manifest_sha256`）；全臂不一致 ⇒ 显式打印
     「不可归因」，不静默混算。判等口径是 confirmed 版本正文的 markdown 字节，
     不是带可变时间戳的 spec 记录。缺 design 字段的旧 run 记 not_observed，
     不据此声称「同一输入」。

用法：python3 harvest_by_role.py <run根目录> [sequence.log ...]
"""
import collections
import datetime
import glob
import itertools
import json
import os
import re
import statistics as st
import sys

PROJECT = "project_0001"
MANIFEST = "run-manifest.json"

# 方向性对照的核心指标（刻意收窄：n=4/臂只配方向性陈述，不铺大检验面）
DIRECTIONAL_METRICS = [
    ("author 耗时 s", "author_seconds"),
    ("author 工具调用", "author_cmds"),
    ("author 产出字节", "source_bytes"),
    ("token in", "in_tokens"),
    ("token out", "out_tokens"),
    ("reviewer findings", "review_findings"),
    ("reviewer 重复报", "review_repeat"),
    ("节点时长合计 s", "node_seconds_sum"),
]


def jload(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return json.load(fh)
    except Exception:
        return None


def load_samples(root, log_paths):
    """读全部 run-manifest.json；返回 (samples_by_tag, excluded)。"""
    manifests = []
    excluded = []
    for d in sorted(glob.glob(os.path.join(root, "run-*"))):
        if not os.path.isdir(d):
            continue
        m = jload(os.path.join(d, MANIFEST))
        if not m:
            excluded.append((os.path.basename(d), "缺 run-manifest.json"))
            continue
        if m.get("mode") != "arm":
            excluded.append((os.path.basename(d), "mode=%s 非 arm" % m.get("mode")))
            continue
        if not m.get("valid"):
            excluded.append((os.path.basename(d),
                             "invalid：%s" % (m.get("invalid_reason") or "未知原因")))
            continue
        manifests.append((d, m))
    # sequence.log 交叉核对（有则报不一致，不作为采样依据）
    if log_paths:
        declared = {}
        for lp in log_paths:
            if not os.path.exists(lp):
                continue
            for ln in open(lp, encoding="utf-8", errors="replace"):
                mt = re.search(r"(finished|failed) (before|after)\s+(\S+)\s+port=(\d+)", ln)
                if mt:
                    declared[mt.group(3)] = (mt.group(2), mt.group(1) == "finished")
        mismatch = [d for d, m in manifests
                    if m.get("arm") in declared
                    and declared[m.get("arm")][0] != m.get("tag")]
        for d in mismatch:
            excluded.append((os.path.basename(d),
                             "sequence.log 与 manifest 的 tag 不一致"))
        manifests = [(d, m) for d, m in manifests if d not in mismatch]
    by_tag = collections.defaultdict(list)
    for d, m in manifests:
        by_tag[m["tag"]].append((d, m))
    return by_tag, excluded


def collect(run_dir, manifest):
    """按角色拆分单 run 指标。issue/plan session 全部取自 manifest（不猜目录）。"""
    issue_id = manifest["issue_id"]
    project = manifest.get("project_id") or PROJECT
    base = os.path.join(run_dir, ".aria", "projects", project, "issues", issue_id)
    out = {
        "run": os.path.basename(run_dir),
        "arm": manifest.get("arm"),
        "binary_sha256": (manifest.get("binary_sha256") or "")[:12],
        "plan_session_id": manifest.get("plan_session_id"),
        "status": manifest.get("status"),
        "provider_start_ledger": manifest.get("provider_start_ledger", 0),
        # 输入来源身份（全部取自 manifest；seed 缺值即 None，不猜）
        "story_version_ref": manifest.get("story_version_ref"),
        "issue_input_sha256": manifest.get("issue_input_sha256"),
        "story_markdown_sha256": manifest.get("story_markdown_sha256"),
        "design_version_ref": manifest.get("design_version_ref"),
        "design_markdown_sha256": manifest.get("design_markdown_sha256"),
        "seed_manifest_sha256": manifest.get("seed_manifest_sha256"),
        "scratch_home": manifest.get("scratch_home"),
        "author_rounds": 0, "author_seconds": 0.0, "author_cmds": 0, "author_codegraph": 0,
        "author_stream_bytes": 0, "author_fail_kind": None,
        "review_rounds": 0, "review_seconds": 0.0, "review_cmds": 0, "review_codegraph": 0,
        "review_findings": 0, "review_must_fix": 0, "review_suggestion": 0,
        "review_repeat": 0, "review_fields": [], "review_preflight_codes": 0,
        "review_semantic_fields": 0,
        "preflight_codes": [], "ir_failed": 0, "ir_passed": 0,
        "compile_ok": False, "compile_seconds": 0.0,
        "validator": [], "cross_wi": 0, "wi": 0, "criterion": 0, "checks": 0, "tasks": 0,
        "source_bytes": 0, "done_when_refs": 0,
        "node_seconds_sum": 0.0,
        "needs_human": 1 if manifest.get("status") == "stopped_needs_human" else 0,
        "in_tokens": None, "out_tokens": None, "cache_tokens": None,
        "usage_events": 0, "usage_missing_runs_hint": 0,
        "first_token_seconds": None,  # 不可测：无 durable 首 token 时间戳来源
    }
    sid = manifest.get("plan_session_id")
    tbase = os.path.join(base, "workspace-timelines", sid or "_",
                         "timeline_node_details")
    nodes = [jload(f) or {} for f in
             sorted(glob.glob(os.path.join(tbase, "*.json")), key=os.path.getmtime)]
    review_rounds = []
    tin = tout = tcache = 0
    for nd in nodes:
        nt, status = nd.get("node_type"), nd.get("status")
        evs = nd.get("execution_events") or []
        cmds = [e for e in evs if e.get("kind") == "command"]
        cg = sum(1 for e in cmds if "codegraph" in json.dumps(e, ensure_ascii=False).lower())
        dur = 0.0
        st_, en = nd.get("started_at"), nd.get("ended_at")
        if st_ and en:
            try:
                dur = (datetime.datetime.fromisoformat(en)
                       - datetime.datetime.fromisoformat(st_)).total_seconds()
            except Exception:
                dur = 0.0
        out["node_seconds_sum"] += dur
        for e in evs:
            if e.get("kind") != "usage":
                continue
            try:
                j = json.loads(e["output"])
                tin += j.get("input_tokens") or 0
                tout += j.get("output_tokens") or 0
                tcache += j.get("cache_read_tokens") or 0
                out["usage_events"] += 1
            except Exception:
                pass
        if nt in ("author_run", "revision"):
            out["author_rounds"] += 1
            out["author_seconds"] += dur
            out["author_cmds"] += len(cmds)
            out["author_codegraph"] += cg
            sc = nd.get("streaming_content") or ""
            out["author_stream_bytes"] = max(out["author_stream_bytes"], len(sc))
            for e in evs:
                det = str(e.get("detail") or "")
                body = str(e.get("output") or "")
                if "教学重驱" not in det or "具体失败原因" not in body:
                    continue
                if "missing_section" in body or "invalid_work_item_id" in body \
                        or "unknown_structured_key" in body:
                    out["author_fail_kind"] = "missing_section"
                elif "unknown_done_when_ref" in body or "unknown_requirement_ref" in body:
                    out["author_fail_kind"] = "ir_family"
                else:
                    out["author_fail_kind"] = "teaching_reredrive"
                break
            if status == "failed" and out["author_fail_kind"] is None:
                out["author_fail_kind"] = "provider_or_parse"
        if nt == "reviewer_run":
            out["review_rounds"] += 1
            out["review_seconds"] += dur
            out["review_cmds"] += len(cmds)
            out["review_codegraph"] += cg
            fs = (nd.get("verdict") or {}).get("findings") or []
            out["review_findings"] += len(fs)
            for f in fs:
                if f.get("severity") == "must_fix":
                    out["review_must_fix"] += 1
                else:
                    out["review_suggestion"] += 1
                fld = f.get("contract_field") or ""
                out["review_fields"].append(fld)
                if fld.startswith("plan_preflight."):
                    out["review_preflight_codes"] += 1
                elif fld:
                    out["review_semantic_fields"] += 1
            review_rounds.append(fs)
            if dur <= 2 and len(cmds) == 0 and fs:
                for f in fs:
                    cf = f.get("contract_field") or ""
                    if cf.startswith("plan_preflight."):
                        out["preflight_codes"].append(cf.split(".")[-1])
        if nt == "work_item_plan_compile":
            out["compile_ok"] = status == "completed"
            out["compile_seconds"] += dur
    if out["usage_events"]:
        out["in_tokens"], out["out_tokens"], out["cache_tokens"] = tin, tout, tcache
    # 重复报：前两个带 findings 的 reviewer 轮
    if len(review_rounds) >= 2:
        c1 = {f.get("contract_field") for f in review_rounds[0]}
        c2 = {f.get("contract_field") for f in review_rounds[1]}
        out["review_repeat"] = len(c1 & c2)
    # IR 判定（按 manifest 的 plan session 过滤）
    ca = os.path.join(base, "candidate-attempts.jsonl")
    if os.path.exists(ca) and sid:
        for ln in open(ca, encoding="utf-8", errors="replace"):
            if not ln.strip():
                continue
            try:
                r = json.loads(ln)
            except Exception:
                continue
            if r.get("session_id") != sid:
                continue
            if r.get("outcome") == "failed":
                out["ir_failed"] += 1
            elif r.get("outcome") == "passed":
                out["ir_passed"] += 1
    if out["status"] != "failed":
        out["author_fail_kind"] = None  # 成功 run 的重驱已被修好，不算失败模式
    for f in glob.glob(os.path.join(base, "issue-work-item-plans/*.json")):
        p = jload(f) or {}
        for v in p.get("validator_findings") or []:
            out["validator"].append("%s:%s" % (v.get("severity"), v.get("code")))
        if p.get("status") == "confirmed":
            out["wi"] = len(p.get("work_item_ids") or [])
    for f in sorted(glob.glob(os.path.join(
            base, "work-item-plan-sources/*/source_revision/source-*.json")),
            key=os.path.getmtime):
        src = (jload(f) or {}).get("source") or ""
        out["source_bytes"] = len(src)
        out["checks"] = len(re.findall(r"^- check_id:", src, re.M))
        out["tasks"] = len(re.findall(r"^- task_id:", src, re.M))
        w, cur = [], None
        for line in src.split("\n"):
            m = re.match(r"^## Work Item (WI-\d+)", line)
            if m:
                cur = {"refs": [], "crits": []}
                w.append(cur)
                continue
            if cur is None:
                continue
            m = re.match(r"^- criterion_id:\s*(\S+)", line)
            if m:
                cur["crits"].append(m.group(1))
                continue
            m = re.match(r"^- done_when_refs:\s*(.*)$", line)
            if m:
                cur["refs"] += re.findall(r"\b(?:AC|WIC)-\d+\b", m.group(1))
        out["criterion"] = sum(len(x["crits"]) for x in w)
        out["done_when_refs"] = sum(len(x["refs"]) for x in w)
        for x in w:
            own = set(x["crits"])
            out["cross_wi"] += sum(1 for r in x["refs"] if r not in own)
    return out


def fmt_med(rs, key):
    vals = [r[key] for r in rs if isinstance(r.get(key), (int, float))
            and r.get(key) is not None]
    if not vals:
        return "n/a"
    if key in ("in_tokens", "out_tokens", "cache_tokens"):
        return "{:,.0f}".format(st.median(vals))
    if isinstance(st.median(vals), float) and key.endswith(("_seconds", "_sum")):
        return "{:.0f}".format(st.median(vals))
    return "{:.1f}".format(st.median(vals)) if isinstance(st.median(vals), float) \
        else str(int(st.median(vals)))


def summarize(tag, rs):
    print("\n===== %s (有效样本 n=%d) =====" % (tag, len(rs)))
    print("  二进制 sha(前12): %s" %
          ", ".join(sorted({r["binary_sha256"] for r in rs})) )
    # 输入来源 digest：来自 run-manifest（seed 冻结），全臂应完全一致
    inputs = sorted({(r.get("issue_input_sha256"), r.get("story_markdown_sha256"),
                      r.get("story_version_ref"), r.get("design_markdown_sha256"),
                      r.get("design_version_ref"), r.get("seed_manifest_sha256"))
                     for r in rs})
    if len(inputs) == 1:
        i_sha, s_sha, vref, d_sha, dref, seed_sha = inputs[0]
        print("  输入 digest: issue_input=%s story_markdown=%s design_markdown=%s "
              "seed_manifest=%s (story=%s design=%s)"
              % (str(i_sha)[:12] or "not_observed",
                 str(s_sha)[:12] or "not_observed",
                 str(d_sha)[:12] or "not_observed",
                 str(seed_sha)[:12] or "not_observed", vref, dref))
    else:
        print("  !! 输入 digest 不一致（%d 组）：这些样本不可归因，检查 seed 复制/重指向"
              % len(inputs))
    print("  终态: %s" % dict(collections.Counter(r["status"] for r in rs)))
    print("  【平台】节点时长合计(sum)中位=%s  计划产出(confirmed)=%d/%d  人工介入=%d/%d"
          % (fmt_med(rs, "node_seconds_sum"),
             sum(1 for r in rs if r["status"] == "confirmed"), len(rs),
             sum(r["needs_human"] for r in rs), len(rs)))
    print("  【平台】provider_start_ledger: %s（0 = 非真实 provider run，已按 invalid 排除）"
          % sorted({r["provider_start_ledger"] for r in rs}))
    print("  【author】轮次中位=%s 耗时中位=%ss 工具调用中位=%s codegraph中位=%s 产出字节中位=%s"
          % (fmt_med(rs, "author_rounds"), fmt_med(rs, "author_seconds"),
             fmt_med(rs, "author_cmds"), fmt_med(rs, "author_codegraph"),
             fmt_med(rs, "source_bytes")))
    print("  【author】失败模式=%s" % dict(collections.Counter(
        r["author_fail_kind"] for r in rs if r["author_fail_kind"])))
    print("  【author】token in中位=%s out中位=%s（缺 usage 事件的 run=%d）"
          % (fmt_med(rs, "in_tokens"), fmt_med(rs, "out_tokens"),
             sum(1 for r in rs if r["in_tokens"] is None)))
    print("  【reviewer】轮次中位=%s 耗时中位=%ss findings中位=%s must_fix中位=%s "
          "suggestion中位=%s 重复报中位=%s"
          % (fmt_med(rs, "review_rounds"), fmt_med(rs, "review_seconds"),
             fmt_med(rs, "review_findings"), fmt_med(rs, "review_must_fix"),
             fmt_med(rs, "review_suggestion"), fmt_med(rs, "review_repeat")))
    print("  【reviewer】机械码field中位=%s 语义field中位=%s 工具调用中位=%s"
          % (fmt_med(rs, "review_preflight_codes"),
             fmt_med(rs, "review_semantic_fields"), fmt_med(rs, "review_cmds")))
    print("  【preflight/compiler】拦截码=%s IR failed=%d passed=%d 编译通过=%d/%d"
          % (dict(collections.Counter(c for r in rs for c in r["preflight_codes"])),
             sum(r["ir_failed"] for r in rs), sum(r["ir_passed"] for r in rs),
             sum(1 for r in rs if r["compile_ok"]), len(rs)))
    print("  【产物】WI中位=%s criterion中位=%s checks中位=%s tasks中位=%s cross-WI越界合计=%d"
          % (fmt_med(rs, "wi"), fmt_med(rs, "criterion"), fmt_med(rs, "checks"),
             fmt_med(rs, "tasks"), sum(r["cross_wi"] for r in rs)))


def exact_perm_split(vb, va):
    """精确置换检验：枚举 C(n_total, n_before) 全部分组（n=4/4 ⇒ 70 个）。

    统计量 = rank-sum |W 差|；p = 至少与观测同样极端的分组占比。
    分辨率下限 = 1/C(8,4) = 1/70 —— 这就是 n=4/臂能分辨的最小 p。
    """
    pool = list(vb) + list(va)
    n1 = len(vb)
    ranks = sorted(range(len(pool)), key=lambda i: pool[i])
    rr = [0.0] * len(pool)
    i = 0
    while i < len(ranks):
        j = i
        while j + 1 < len(ranks) and pool[ranks[j + 1]] == pool[ranks[i]]:
            j += 1
        for k in range(i, j + 1):
            rr[ranks[k]] = (i + j) / 2 + 1
        i = j + 1

    def stat(idxs):
        w = sum(rr[i] for i in idxs)
        return abs(w - n1 * (n1 + 1) / 2 - n1 * (len(pool) - n1) / 2)

    obs = stat(range(n1))
    worse = 0
    total = 0
    for combo in itertools.combinations(range(len(pool)), n1):
        total += 1
        if stat(combo) >= obs - 1e-9:
            worse += 1
    return worse / total, total


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "/tmp/aria-wig"
    logs = sys.argv[2:]
    by_tag, excluded = load_samples(root, logs)
    print("run 根目录: %s" % root)
    if excluded:
        print("\n----- 排除的 run（不进入对照） -----")
        for name, why in excluded:
            print("  %s: %s" % (name, why))
    before = sorted(by_tag.get("before", []), key=lambda x: x[1]["arm"])
    after = sorted(by_tag.get("after", []), key=lambda x: x[1]["arm"])
    if not before or not after:
        print("\n有效样本不足：before=%d after=%d（至少各 1 才能对照）"
              % (len(before), len(after)))
        sys.exit(2)
    b = [collect(d, m) for d, m in before]
    a = [collect(d, m) for d, m in after]
    summarize("改前（before 二进制）", b)
    summarize("改后（after 二进制）", a)

    digests = {(r.get("issue_input_sha256"), r.get("story_markdown_sha256"),
                r.get("story_version_ref"), r.get("design_markdown_sha256"),
                r.get("design_version_ref"), r.get("seed_manifest_sha256"))
               for r in b + a}
    if len(digests) != 1:
        print("\n!! before/after 的输入来源 digest 不一致（%d 组）："
              "这些 run 不是同一输入，对照不可归因" % len(digests))
    else:
        i_sha, s_sha, vref, d_sha, dref, seed_sha = next(iter(digests))
        print("\n输入来源（全臂一致）：issue_input=%s story_markdown=%s %s "
              "design_markdown=%s %s seed_manifest=%s"
              % (str(i_sha)[:12], str(s_sha)[:12], vref,
                 str(d_sha)[:12] or "not_observed", dref or "not_observed",
                 str(seed_sha)[:12]))

    print("\n" + "=" * 96)
    print("改前 → 改后 方向性对照（同一 seed 输入 × 同装置 × ABBA 串行；"
          "n=%d/%d 有效样本）" % (len(b), len(a)))
    print("=" * 96)
    print("  %-24s %14s  →  %14s" % ("指标", "改前中位", "改后中位"))
    for lab, key in DIRECTIONAL_METRICS:
        print("  %-24s %14s  →  %14s" % (lab, fmt_med(b, key), fmt_med(a, key)))
    print("  说明：总耗时口径 = 各 timeline 节点时长之和（sum），非墙钟起止差；")
    print("  first-token time 不可测（durable 产物无首 token 时间戳来源），不估算。")
    print("  token 缺 usage 事件的 run 记 None（中位只在观测到的值上算），不补 0。")

    print("\n" + "=" * 96)
    print("精确置换检验（枚举全部 C(%d,%d) 分组，方向性参考，不作显著性判定）"
          % (len(b) + len(a), len(b)))
    print("=" * 96)
    total = 0
    for lab, key in DIRECTIONAL_METRICS:
        vb = [float(r[key]) for r in b if isinstance(r.get(key), (int, float))
              and r.get(key) is not None]
        va = [float(r[key]) for r in a if isinstance(r.get(key), (int, float))
              and r.get(key) is not None]
        if len(vb) != len(b) or len(va) != len(a) or not vb:
            print("  %-24s %13s %13s %9s"
                  % (lab, fmt_med(b, key), fmt_med(a, key), "n/a(缺值)"))
            continue
        p, total = exact_perm_split(vb, va)
        print("  %-24s %13.1f %13.1f   p=%.4f（%d/%d 分组 ≥ 观测）"
              % (lab, st.median(vb), st.median(va), p, round(p * total), total))
    print("  n=%d/臂 ⇒ 精确枚举 C(%d,%d)=%d 个分组，分辨率 1/%d≈%.4f；离散小样本"
          "检验功效有限，只作方向性记录，不做显著性强弱判定、不做因果推断。"
          % (len(b), len(b) + len(a), len(b), total, total, 1.0 / total))


if __name__ == "__main__":
    main()
