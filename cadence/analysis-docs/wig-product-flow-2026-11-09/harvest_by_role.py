#!/usr/bin/env python3
"""按角色拆分收割 WIG 真实链路的每个 run，并输出 before/after 对照。

角色维度：
  author      —— 作者轮（首轮 + 重写）：耗时 / 工具调用 / 产出 / 失败模式 / token
  reviewer    —— 评审轮：耗时 / findings / severity / 重复报 / 议题粒度 / token
  preflight+compiler —— 机械拦截码 / IR 判定 / 编译 / validator / cross-WI
  platform    —— 总墙钟 / 终态 / 人工介入 / 重写轮 / 计划产出率
"""
import collections
import datetime
import glob
import json
import os
import re
import statistics as st
import sys

ISSUES = ".aria/projects/project_0001/issues/issue_0001"


def jload(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return json.load(fh)
    except Exception:
        return None


def collect(run_dir):
    """run_dir: 含 .aria/ 的工作区目录。返回按角色拆分的指标。"""
    base = os.path.join(run_dir, ISSUES)
    out = {
        "run": os.path.basename(run_dir),
        "author_rounds": 0, "author_seconds": 0.0, "author_cmds": 0, "author_codegraph": 0,
        "author_stream_bytes": 0, "author_parse_failed": False, "author_fail_kind": None,
        "review_rounds": 0, "review_seconds": 0.0, "review_cmds": 0, "review_codegraph": 0,
        "review_findings": 0, "review_must_fix": 0, "review_suggestion": 0,
        "review_repeat": 0, "review_fields": [], "review_preflight_codes": 0,
        "review_semantic_fields": 0,
        "preflight_codes": [], "ir_failed": 0, "ir_passed": 0,
        "compile_ok": False, "compile_seconds": 0.0,
        "validator": [], "cross_wi": 0, "wi": 0, "criterion": 0, "checks": 0, "tasks": 0,
        "source_bytes": 0, "done_when_refs": 0,
        "total_seconds": 0.0, "status": None, "needs_human": 0,
        "in_tokens": 0, "out_tokens": 0, "cache_tokens": 0,
    }
    tbase = glob.glob(os.path.join(base, "workspace-timelines/*auto_*/timeline_node_details"))
    if not tbase:
        return None
    nodes = []
    for f in sorted(glob.glob(tbase[0] + "/*.json"), key=os.path.getmtime):
        nodes.append((jload(f) or {}))
    # 会话终态
    for p in glob.glob(os.path.join(base, "workspace-sessions/*auto_*.json")):
        rec = jload(p) or {}
        out["status"] = rec.get("status")
        out["needs_human"] = 1 if rec.get("status") == "stopped_needs_human" else 0
        break
    # 节点
    review_rounds = []
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
        out["total_seconds"] += dur
        for e in evs:
            if e.get("kind") == "usage":
                try:
                    j = json.loads(e["output"])
                    out["in_tokens"] += j.get("input_tokens", 0)
                    out["out_tokens"] += j.get("output_tokens", 0)
                    out["cache_tokens"] += j.get("cache_read_tokens", 0)
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
                if "missing_section" in body or "invalid_work_item_id" in body or "unknown_structured_key" in body:
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
            review_rounds.append((dur, fs, nd))
            # preflight 节点（0.1s、无工具调用）承载机械拦截
            if dur <= 2 and len(cmds) == 0 and fs:
                for f in fs:
                    cf = f.get("contract_field") or ""
                    if cf.startswith("plan_preflight."):
                        out["preflight_codes"].append(cf.split(".")[-1])
        if nt == "work_item_plan_compile":
            out["compile_ok"] = status == "completed"
            out["compile_seconds"] += dur
    # 重复报：按节点顺序的第 1/第 2 个带 findings 的 reviewer_run
    if len(review_rounds) >= 2:
        c1 = {f.get("contract_field") for f in review_rounds[0][1]}
        c2 = {f.get("contract_field") for f in review_rounds[1][1]}
        out["review_repeat"] = len(c1 & c2)
    # IR 判定（按 session 过滤，避免模板残留）
    sess = glob.glob(os.path.join(base, "workspace-sessions/*auto_*.json"))
    sid = os.path.basename(sess[0])[:-5] if sess else None
    ca = os.path.join(base, "candidate-attempts.jsonl")
    if os.path.exists(ca):
        for ln in open(ca, encoding="utf-8", errors="replace"):
            if not ln.strip():
                continue
            r = jload_line(ln) if False else None
            try:
                r = json.loads(ln)
            except Exception:
                continue
            if sid and r.get("session_id") != sid:
                continue
            if r.get("outcome") == "failed":
                out["ir_failed"] += 1
            elif r.get("outcome") == "passed":
                out["ir_passed"] += 1
    # 重驱「发生过」与「失败终局」区分：成功 run 的重驱被修好，不算失败模式
    if out["status"] != "failed":
        out["author_fail_kind"] = None
    # 计划与产物
    for f in glob.glob(os.path.join(base, "issue-work-item-plans/*.json")):
        p = jload(f) or {}
        for v in p.get("validator_findings") or []:
            out["validator"].append(f"{v.get('severity')}:{v.get('code')}")
        if p.get("status") == "confirmed":
            out["wi"] = len(p.get("work_item_ids") or [])
    for f in sorted(glob.glob(os.path.join(base, "work-item-plan-sources/*/source_revision/source-*.json")),
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


def jload_line(_):
    return None


def summarize(tag, runs):
    rs = [collect(d) for d in runs]
    rs = [r for r in rs if r]
    if not rs:
        return
    def med(k):
        vals = [r[k] for r in rs if isinstance(r.get(k), (int, float)) and r.get(k) is not None]
        return st.median(vals) if vals else 0
    print(f"\n===== {tag}  (n={len(rs)}) =====")
    print(f"  终态: {dict(collections.Counter(r['status'] for r in rs))}")
    print(f"  【平台】总墙钟中位数={med('total_seconds'):.0f}s  计划产出(confirmed)={sum(1 for r in rs if r['status']=='confirmed')}/{len(rs)}"
          f"  人工介入={sum(r['needs_human'] for r in rs)}/{len(rs)}")
    print(f"  【author】轮次中位={med('author_rounds'):.1f} 耗时中位={med('author_seconds'):.0f}s 工具调用中位={med('author_cmds'):.0f}"
          f" codegraph中位={med('author_codegraph'):.0f} 产出字节中位={med('source_bytes'):.0f}"
          f" 失败模式={dict(collections.Counter(r['author_fail_kind'] for r in rs if r['author_fail_kind']))}")
    print(f"  【author】token in中位={med('in_tokens'):,.0f} out中位={med('out_tokens'):,.0f}")
    print(f"  【reviewer】轮次中位={med('review_rounds'):.1f} 耗时中位={med('review_seconds'):.0f}s"
          f" findings中位={med('review_findings'):.0f} must_fix中位={med('review_must_fix'):.0f}"
          f" suggestion中位={med('review_suggestion'):.0f} **重复报中位={med('review_repeat'):.0f}**"
          f" 机械码field中位={med('review_preflight_codes'):.0f} 语义field中位={med('review_semantic_fields'):.0f}")
    print(f"  【reviewer】工具调用中位={med('review_cmds'):.0f} codegraph中位={med('review_codegraph'):.0f}")
    print(f"  【preflight/compiler】拦截码={dict(collections.Counter(c for r in rs for c in r['preflight_codes']))}"
          f" IR failed={sum(r['ir_failed'] for r in rs)} passed={sum(r['ir_passed'] for r in rs)}"
          f" 编译通过={sum(1 for r in rs if r['compile_ok'])}/{len(rs)}")
    print(f"  【产物】WI={med('wi'):.0f} criterion={med('criterion'):.0f} done_when_refs={med('done_when_refs'):.0f}"
          f" checks={med('checks'):.0f} tasks={med('tasks'):.0f} **cross-WI={sum(r['cross_wi'] for r in rs)}**"
          f" validator={dict(collections.Counter(v for r in rs for v in r['validator']))}")
    return rs


def arm_map_from_logs(log_paths):
    """从序列日志读「端口 → (臂, 档位)」的权威映射（避免手写标签出错）。"""
    import re
    m = {}
    for lp in log_paths:
        if not os.path.exists(lp):
            continue
        for ln in open(lp, encoding="utf-8", errors="replace"):
            mt = re.search(r"finished (before|after)\s+(\w+)\s+eff=(\S+)\s+port=(\d+)", ln)
            if mt:
                m[mt.group(4)] = (mt.group(1), mt.group(2), mt.group(3))
    return m


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "/tmp/aria-wig"
    logs = sys.argv[2:] or ["/tmp/aria-wig/ba/sequence.log"]
    amap = arm_map_from_logs(logs)
    before, after = [], []
    for port, (tag, arm, _eff) in sorted(amap.items()):
        d = os.path.join(root, f"run-{arm}-{port}")
        if not os.path.isdir(d):
            continue
        (before if tag == "before" else after).append(d)
    b = summarize("改前（db34f3f0，无 D5/D6）", before)
    a = summarize("改后（HEAD，含 D5/D6）", after)
    if b and a:
        print("\n" + "=" * 100)
        print("改前 → 改后 对照（同样时段、同样装置、n=%d/%d）" % (len(b), len(a)))
        print("=" * 100)
        rows = [
            ("【平台】计划产出率(confirmed)", lambda rs: f"{sum(1 for r in rs if r['status']=='confirmed')}/{len(rs)}"),
            ("【平台】需人工介入率", lambda rs: f"{sum(r['needs_human'] for r in rs)}/{len(rs)}"),
            ("【平台】未产出计划(failed)", lambda rs: f"{sum(1 for r in rs if r['status']=='failed')}/{len(rs)}"),
            ("【平台】总墙钟中位数 s", lambda rs: f"{st.median([r['total_seconds'] for r in rs]):.0f}"),
            ("【author】轮次中位", lambda rs: f"{st.median([r['author_rounds'] for r in rs]):.1f}"),
            ("【author】耗时中位 s", lambda rs: f"{st.median([r['author_seconds'] for r in rs]):.0f}"),
            ("【author】工具调用中位", lambda rs: f"{st.median([r['author_cmds'] for r in rs]):.0f}"),
            ("【author】codegraph 中位", lambda rs: f"{st.median([r['author_codegraph'] for r in rs]):.0f}"),
            ("【author】产出字节中位", lambda rs: f"{st.median([r['source_bytes'] for r in rs]):.0f}"),
            ("【author】token in 中位", lambda rs: f"{st.median([r['in_tokens'] for r in rs]):,.0f}"),
            ("【author】token out 中位", lambda rs: f"{st.median([r['out_tokens'] for r in rs]):,.0f}"),
            ("【reviewer】轮次中位", lambda rs: f"{st.median([r['review_rounds'] for r in rs]):.1f}"),
            ("【reviewer】耗时中位 s", lambda rs: f"{st.median([r['review_seconds'] for r in rs]):.0f}"),
            ("【reviewer】findings 中位", lambda rs: f"{st.median([r['review_findings'] for r in rs]):.0f}"),
            ("【reviewer】must_fix 中位", lambda rs: f"{st.median([r['review_must_fix'] for r in rs]):.0f}"),
            ("【reviewer】suggestion 中位", lambda rs: f"{st.median([r['review_suggestion'] for r in rs]):.0f}"),
            ("【reviewer】重复报 中位", lambda rs: f"{st.median([r['review_repeat'] for r in rs]):.0f}"),
            ("【reviewer】机械码 field 中位", lambda rs: f"{st.median([r['review_preflight_codes'] for r in rs]):.0f}"),
            ("【reviewer】语义 field 中位", lambda rs: f"{st.median([r['review_semantic_fields'] for r in rs]):.0f}"),
            ("【reviewer】工具调用中位", lambda rs: f"{st.median([r['review_cmds'] for r in rs]):.0f}"),
            ("【preflight】拦截次数中位", lambda rs: f"{st.median([len(r['preflight_codes']) for r in rs]):.0f}"),
            ("【compiler】IR failed 合计", lambda rs: f"{sum(r['ir_failed'] for r in rs)}"),
            ("【compiler】编译通过", lambda rs: f"{sum(1 for r in rs if r['compile_ok'])}/{len(rs)}"),
            ("【产物】WI 中位", lambda rs: f"{st.median([r['wi'] for r in rs]):.0f}"),
            ("【产物】criterion 中位", lambda rs: f"{st.median([r['criterion'] for r in rs]):.0f}"),
            ("【产物】checks 中位", lambda rs: f"{st.median([r['checks'] for r in rs]):.0f}"),
            ("【产物】tasks 中位", lambda rs: f"{st.median([r['tasks'] for r in rs]):.0f}"),
            ("【产物】cross-WI 越界合计", lambda rs: f"{sum(r['cross_wi'] for r in rs)}"),
        ]
        for lab, fn in rows:
            print(f"  {lab:34s} {fn(b):>14s}  →  {fn(a):>14s}")


if __name__ == "__main__":
    main()
