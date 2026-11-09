#!/usr/bin/env python3
"""收割串行交替的产品 WIG 全流程运行（n=2/档）并做对照统计。"""
import collections
import datetime
import json
import pathlib
import random
import re
import statistics as st
import sys

BASE = pathlib.Path("/tmp/aria-wig")
ISSUE = "projects/project_0001/issues/issue_0001"


def T(v):
    try:
        return datetime.datetime.fromisoformat(v.replace("Z", "+00:00")) if v else None
    except Exception:
        return None


def ir_compliance(src):
    """复刻 work_item_contract/validation.rs:245-312 的 plan 域成员校验谓词。"""
    wis, cur = [], None
    for line in src.split("\n"):
        m = re.match(r"^## Work Item (WI-\d+)", line)
        if m:
            cur = {"wi": m.group(1), "refs": [], "crits": []}
            wis.append(cur); continue
        if cur is None:
            continue
        m = re.match(r"^- criterion_id:\s*(\S+)", line)
        if m:
            cur["crits"].append(m.group(1)); continue
        m = re.match(r"^- done_when_refs:\s*(.*)$", line)
        if m:
            cur["refs"] += re.findall(r"\b(?:AC|WIC)-\d+\b", m.group(1))
    cross, total, nc = [], 0, 0
    for w in wis:
        own = set(w["crits"]); nc += len(own)
        for r in w["refs"]:
            total += 1
            if r not in own:
                cross.append((w["wi"], r))
    return {"wis": len(wis), "criteria": nc, "refs": total, "cross": len(cross), "detail": cross[:12]}


def harvest_run(ws: pathlib.Path):
    A = ws / ".aria"
    if not A.exists():
        return None
    ss = sorted((A / ISSUE / "workspace-sessions").glob("*.json"), key=lambda p: p.stat().st_mtime)
    if not ss:
        return None
    # 只认自动化 session（id 含 auto_）；否则会读到复制进来的旧 session
    auto = [p for p in ss if "auto_" in p.stem]
    sess = (auto or ss)[-1].stem
    if "auto_" not in sess:
        return None
    r = {"ws": str(ws), "session": sess, "nodes": [], "usage": collections.Counter(),
         "tools": collections.Counter(), "codegraph": 0, "findings": [], "plan": None, "ir": [],
         "author_rounds": 0, "reviewer_rounds": 0, "wall": 0.0, "compiles": 0,
         "ir_fail": 0, "rewrite": 0, "ir_predicate": None, "src_file": None}
    tl = A / ISSUE / "workspace-timelines" / sess / "timeline_node_details"
    for p in sorted(tl.glob("*.json"), key=lambda x: x.stat().st_mtime) if tl.exists() else []:
        d = json.loads(p.read_text(encoding="utf-8"))
        st_, en = T(d.get("started_at")), T(d.get("ended_at"))
        dur = (en - st_).total_seconds() if (st_ and en) else None
        evs = d.get("execution_events") or []
        cmds = [e for e in evs if e.get("kind") == "command"]
        cg = sum(1 for e in cmds if "codegraph" in json.dumps(e, ensure_ascii=False).lower())
        for e in cmds:
            c = (e.get("command") or "")
            k = "codegraph" if "codegraph" in c.lower() else (c.split()[0] if c.split() else "?")
            r["tools"][k] += 1
        r["codegraph"] += cg
        for e in evs:
            if e.get("kind") == "usage" and isinstance(e.get("output"), str):
                try:
                    u = json.loads(e["output"])
                    r["usage"]["in"] += u.get("input_tokens", 0)
                    r["usage"]["out"] += u.get("output_tokens", 0)
                    r["usage"]["cache"] += u.get("cache_read_tokens", 0)
                except Exception:
                    pass
        nt = d.get("node_type")
        if nt in ("author_run", "revision"):
            r["author_rounds"] += 1
        if nt == "reviewer_run":
            r["reviewer_rounds"] += 1
        if nt == "work_item_plan_compile":
            r["compiles"] += 1
        if dur:
            r["wall"] += dur
        for f in ((d.get("verdict") or {}).get("findings") or []):
            r["findings"].append({"type": nt, "disposition": f.get("disposition"),
                                  "class_hint": f.get("class_hint"), "category": f.get("category"),
                                  "field": f.get("contract_field"), "msg": str(f.get("message"))[:120]})
        r["nodes"].append({"file": p.stem, "type": nt, "status": d.get("status"), "dur": dur,
                           "cmds": len(cmds), "codegraph": cg})
    r["rewrite"] = max(0, r["author_rounds"] - 1)
    r["preflight_fail"] = 0
    # 会话终态：stopped_needs_human = 自动预算耗尽 ⇒ 人工介入（用户抱怨③的终点）
    try:
        sj = json.loads((A / ISSUE / "workspace-sessions" / f"{sess}.json").read_text(encoding="utf-8"))
        r["session_status"] = sj.get("status")
        r["needs_human"] = 1 if sj.get("status") in ("stopped_needs_human", "waiting_for_human") else 0
        r["run_policy"] = sj.get("run_policy")
        r["review_rounds"] = sj.get("review_rounds")
    except Exception:
        r["session_status"] = None; r["needs_human"] = 0
    # preflight 机械校验码（reviewer verdict comments 里的 [code]）
    r["preflight_codes"] = []
    for f in r["findings"]:
        for m in re.findall(r"\[([a-z_]{8,44})\]", f.get("msg") or ""):
            if m not in r["preflight_codes"]:
                r["preflight_codes"].append(m)
    r["preflight_fail"] = 1 if r["preflight_codes"] else 0
    # IR 判定（平台 compiler）
    ca = A / ISSUE / "candidate-attempts.jsonl"
    if ca.exists():
        for ln in ca.read_text(encoding="utf-8", errors="replace").splitlines():
            if not ln.strip():
                continue
            try:
                rec = json.loads(ln)
            except Exception:
                continue
            if rec.get("session_id") == sess:
                r["ir"].append(rec)
    r["ir_fail"] = sum(1 for x in r["ir"] if x.get("outcome") == "failed")
    # plan
    d = A / ISSUE / "issue-work-item-plans"
    ps = sorted(d.glob("*.json"), key=lambda x: x.stat().st_mtime) if d.exists() else []
    if ps:
        pd = json.loads(ps[-1].read_text(encoding="utf-8"))
        r["plan"] = {"file": ps[-1].name, "status": pd.get("status"), "wis": pd.get("work_item_ids"),
                     "validator": [(f.get("severity"), f.get("code")) for f in (pd.get("validator_findings") or [])]}
        # 最终 source → 按编译器谓词算 cross-WI
        sd = d.parent / "work-item-plan-sources" / ps[-1].stem / "source_revision"
        sfs = sorted(sd.glob("source-*.json"), key=lambda x: x.stat().st_mtime) if sd.exists() else []
        if sfs:
            try:
                src = json.loads(sfs[-1].read_text(encoding="utf-8")).get("source") or ""
                r["ir_predicate"] = ir_compliance(src)
                r["src_file"] = sfs[-1].name
            except Exception:
                pass
    return r


def collect():
    runs = []
    for d in sorted(BASE.glob("run-*")):
        r = harvest_run(d)
        if r:
            r["arm"] = "high" if "/run-high-" in str(d) else "xhigh"
            r["eff"] = "high" if r["arm"] == "high" else "inherit(xhigh)"
            runs.append(r)
    return runs


def perm(x, h, N=20000, seed=23):
    pool = list(x) + list(h)
    n1 = len(x)
    def ranks(v):
        o = sorted(range(len(v)), key=lambda i: v[i]); rr = [0.0]*len(v); i = 0
        while i < len(o):
            j = i
            while j+1 < len(o) and v[o[j+1]] == v[o[i]]: j += 1
            for k in range(i, j+1): rr[o[k]] = (i+j)/2+1
            i = j+1
        return rr
    ra = ranks(pool)
    obs = abs(sum(ra[:n1]) - n1*(n1+1)/2 - n1*len(h)/2)
    cnt = 0
    rnd = random.Random(seed)
    for _ in range(N):
        rnd.shuffle(ra)
        u = abs(sum(ra[:n1]) - n1*(n1+1)/2 - n1*len(h)/2)
        if u >= obs - 1e-9:
            cnt += 1
    return (cnt + 1) / (N + 1)


def main():
    runs = collect()
    if not runs:
        print("尚无完成的自动化 session")
        return
    runs.sort(key=lambda r: (r["arm"], r["ws"]))
    print("=" * 112)
    print("产品 WIG 全流程 —— 串行交替对照（唯一变量 = effort 档位；同一真实案例 + codegraph + reviewer + 编译重驱链）")
    print("=" * 112)
    for r in runs:
        print(f"\n--- {r['arm']}  ({r['ws'].split('/')[-1]})  session={r['session'][:40]}")
        for n in r["nodes"]:
            print(f"    {n['file']}: {str(n['type']):22s} {str(n['status']):10s} "
                  f"dur={('%9.1fs'%n['dur']) if n['dur'] else '        ?'} cmds={n['cmds']:3d} codegraph={n['codegraph']}")
        print(f"    合计: 墙钟={r['wall']:.1f}s 作者轮次={r['author_rounds']}(含重写{r['rewrite']}) "
              f"reviewer轮次={r['reviewer_rounds']} compile={r['compiles']} codegraph={r['codegraph']} cmds={sum(r['tools'].values())}")
        print(f"    usage: in={r['usage']['in']:,} out={r['usage']['out']:,} cache_read={r['usage']['cache']:,}")
        print(f"    IR 判定（平台 compiler）: {[(x.get('outcome'), x.get('stage')) for x in r['ir']]}")
        print(f"    session 终态={r.get('session_status')} policy={r.get('run_policy')} "
              f"review_rounds={r.get('review_rounds')} ⇒ 需人工介入={r.get('needs_human')}")
        if r.get("preflight_codes"):
            print(f"    preflight 机械校验码: {r['preflight_codes']}")
        print(f"    plan: status={(r['plan'] or {}).get('status')} WIs={(r['plan'] or {}).get('wis')} "
              f"validator={(r['plan'] or {}).get('validator')}")
        if r.get("ir_predicate"):
            ip = r["ir_predicate"]
            print(f"    最终 source: {r['src_file']} | WI={ip['wis']} criterion={ip['criteria']} "
                  f"refs={ip['refs']} **cross-WI={ip['cross']}**")
            if ip["detail"]:
                print(f"        越界明细: {ip['detail']}")
        for f in r["findings"]:
            print(f"    reviewer finding [{f['type']}]: disposition={f['disposition']} class_hint={f['class_hint']} "
                  f"category={f['category']} field={f['field']}")
            print(f"        {f['msg'][:130]}")
    # 完成判定：会话不再是 running（confirmed / failed / stopped_needs_human 都算跑完）
    done = [r for r in runs if r.get("session_status") and r["session_status"] != "running"]
    X = [r for r in done if r["arm"] == "xhigh"]
    H = [r for r in done if r["arm"] == "high"]
    if len(done) < len(runs):
        print(f"（进行中的 run 未计入对照：{len(runs)-len(done)} 个）")
    if X and H:
        print(f"\n{'='*112}\n=== 对照（xhigh n={len(X)} vs high n={len(H)}）\n{'='*112}")
        print(f"{'指标':24s} {'xhigh':>16s} {'high':>16s} {'差(high-xhigh)':>20s} {'置换p':>10s}")
        for key, lab in (("wall", "墙钟合计 s"), ("__in", "input token"), ("__out", "output token"),
                         ("__cache", "cache_read token"), ("codegraph", "codegraph 调用"),
                         ("author_rounds", "作者轮次(含重写)"), ("reviewer_rounds", "reviewer 轮次"),
                         ("rewrite", "重写轮次"), ("ir_fail", "IR 校验失败次数"),
                         ("needs_human", "需人工介入次数"), ("preflight_fail", "preflight 机械校验失败"),
                         ("compiles", "编译次数")):
            def pick(rs, k):
                return [r["usage"]["in"] if k == "__in" else r["usage"]["out"] if k == "__out"
                        else r["usage"]["cache"] if k == "__cache" else r[k] for r in rs]
            xs = pick(X, key); hs = pick(H, key)
            mx, mh = st.median(xs), st.median(hs)
            d = mh - mx
            p = perm(xs, hs) if len(set(xs)) > 1 or len(set(hs)) > 1 else 1.0
            print(f"{lab:24s} {mx:>16,.1f} {mh:>16,.1f} {d:>+20,.1f} ({(d/mx*100 if mx else 0):>+6.1f}%) {p:>10.4f}")
        xf = sum(len(r["findings"]) for r in X); hf = sum(len(r["findings"]) for r in H)
        print(f"{'reviewer findings 合计':24s} {xf:>16d} {hf:>16d} {hf-xf:>+20d}")
        xc = sum((r.get("ir_predicate") or {}).get("cross", 0) for r in X)
        hc = sum((r.get("ir_predicate") or {}).get("cross", 0) for r in H)
        xt = sum((r.get("ir_predicate") or {}).get("refs", 0) for r in X)
        ht = sum((r.get("ir_predicate") or {}).get("refs", 0) for r in H)
        print(f"{'最终 plan cross-WI 越界':24s} {xc:>10d}/{xt:<5d} {hc:>10d}/{ht:<5d}")


if __name__ == "__main__":
    main()