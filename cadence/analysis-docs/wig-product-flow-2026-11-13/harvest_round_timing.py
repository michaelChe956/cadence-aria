#!/usr/bin/env python3
"""首字延迟归因 harvest：把我们的四段时刻 sidecar 与 provider 原生转录 join，
回答「① 首 token 慢到底慢在哪」。

只读、零新增模型调用；**只输出聚合数字**，不落任何 prompt 原文 / provider
原文 / 凭据（遵守「不提交 raw prompt 与 provider 输出」）。

用法：
    python3 harvest_round_timing.py [--root /tmp/wig-measurement-setup] \
        [--max-match-gap-s 180]

产出（stdout）：每轮分段 + 汇总表（段位中位/p90、工具轮次相关性、匹配率）。
"""
import argparse
import glob
import json
import os
import statistics
from datetime import datetime, timezone


def parse_ts(text):
    return datetime.fromisoformat(text.replace("Z", "+00:00"))


def median(values):
    return statistics.median(values) if values else None


def p90(values):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, int(round(0.9 * (len(ordered) - 1))))
    return ordered[index]


def rank(values):
    order = sorted(range(len(values)), key=lambda i: values[i])
    ranks = [0] * len(values)
    for position, index in enumerate(order):
        ranks[index] = position + 1
    return ranks


def spearman(xs, ys):
    if len(xs) < 3:
        return None
    rx, ry = rank(xs), rank(ys)
    n = len(xs)
    mean_x = sum(rx) / n
    mean_y = sum(ry) / n
    num = sum((a - mean_x) * (b - mean_y) for a, b in zip(rx, ry))
    den_x = sum((a - mean_x) ** 2 for a in rx) ** 0.5
    den_y = sum((b - mean_y) ** 2 for b in ry) ** 0.5
    if den_x == 0 or den_y == 0:
        return None
    return num / (den_x * den_y)


def load_rollouts(root):
    """provider 原生转录：thread 起始时刻 + 该轮的 provider 侧分段。"""
    out = []
    pattern = os.path.join(root, "home", ".codex", "sessions", "*", "*", "*", "rollout-*.jsonl")
    for path in sorted(glob.glob(pattern)):
        meta_ts = None
        prompt_ts = None
        first_activity = None
        last_activity = None
        first_text = None
        tool_calls = 0
        try:
            with open(path, encoding="utf-8", errors="replace") as handle:
                for line in handle:
                    try:
                        event = json.loads(line)
                    except Exception:
                        continue
                    stamp = event.get("timestamp")
                    kind = event.get("type")
                    payload = event.get("payload") or {}
                    if kind == "session_meta" and meta_ts is None:
                        meta_ts = stamp
                    elif kind == "response_item":
                        sub = payload.get("type")
                        if sub == "message" and payload.get("role") == "user" and prompt_ts is None:
                            prompt_ts = stamp
                        elif sub == "reasoning":
                            if first_activity is None:
                                first_activity = stamp
                            last_activity = stamp
                        elif sub == "function_call":
                            tool_calls += 1
                            if first_activity is None:
                                first_activity = stamp
                            last_activity = stamp
                        elif sub == "message" and payload.get("role") == "assistant":
                            if first_text is None:
                                first_text = stamp
                    elif kind == "event_msg" and payload.get("type") == "agent_message_delta":
                        if first_text is None:
                            first_text = stamp
        except Exception:
            continue
        if meta_ts is None or prompt_ts is None:
            continue
        out.append({
            "meta": parse_ts(meta_ts),
            "prompt": parse_ts(prompt_ts),
            "first_activity": parse_ts(first_activity) if first_activity else None,
            "last_activity": parse_ts(last_activity) if last_activity else None,
            "first_text": parse_ts(first_text) if first_text else None,
            "tool_calls": tool_calls,
        })
    return out


def load_rounds(root):
    """我们侧的四段时刻（sidecar）。"""
    out = []
    for path in sorted(glob.glob(os.path.join(root, "measure-*", "run-*", ".aria", "projects", "*",
                                           "issues", "*", "workspace-timelines", "*",
                                           "round-timing.jsonl"))):
        parts = path.split(os.sep)
        batch = next((part for part in parts if part.startswith("measure-")), "unknown")
        arm = next((part for part in parts if part.startswith("run-")), "unknown")
        with open(path, encoding="utf-8", errors="replace") as handle:
            for line in handle:
                try:
                    record = json.loads(line)
                except Exception:
                    continue
                start = record.get("round_start_unix_ms")
                if not isinstance(start, int):
                    continue
                out.append({
                    "batch": batch,
                    "arm": arm,
                    "kind": record.get("round_kind"),
                    "seq": record.get("role_run_seq"),
                    "start": datetime.fromtimestamp(start / 1000, tz=timezone.utc),
                    "pre_round_ms": record.get("provider_session_ready_ms"),
                    "ttft_ms": record.get("first_model_delta_ms"),
                    "display_ms": record.get("first_display_chunk_ms"),
                    "terminal_ms": record.get("drive_terminal_ms"),
                    "client_receive": record.get("client_receive_ms"),
                })
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default="/tmp/wig-measurement-setup")
    parser.add_argument("--max-match-gap-s", type=float, default=180.0)
    args = parser.parse_args()

    rollouts = load_rollouts(args.root)
    rounds = load_rounds(args.root)
    print(f"provider 转录 {len(rollouts)} 条；我方轮 {len(rounds)} 轮")

    matched, unmatched = [], 0
    for entry in rounds:
        best = None
        best_gap = None
        for rollout in rollouts:
            gap = abs((rollout["prompt"] - entry["start"]).total_seconds())
            if gap <= args.max_match_gap_s and (best_gap is None or gap < best_gap):
                best, best_gap = rollout, gap
        if best is None:
            unmatched += 1
            continue
        entry["rollout"] = best
        entry["match_gap_s"] = round(best_gap, 1)
        matched.append(entry)

    print(f"匹配 {len(matched)} 轮，未匹配 {unmatched} 轮"
          f"（匹配率 {len(matched) / max(len(rounds), 1):.1%}）")

    def ms(value):
        return value if isinstance(value, int) else None

    pre_round, provider_first, think_tools, ttft, tail, gaps = [], [], [], [], [], []
    tool_counts, ttft_pairs = [], []
    for entry in matched:
        rollout = entry["rollout"]
        if isinstance(entry.get("pre_round_ms"), int):
            pre_round.append(entry["pre_round_ms"])
        if isinstance(entry.get("ttft_ms"), int):
            ttft.append(entry["ttft_ms"])
            tool_counts.append(rollout["tool_calls"])
            ttft_pairs.append((rollout["tool_calls"], entry["ttft_ms"]))
        if isinstance(entry.get("display_ms"), int) and isinstance(entry.get("ttft_ms"), int):
            gaps.append(entry["display_ms"] - entry["ttft_ms"])
        if isinstance(entry.get("terminal_ms"), int) and isinstance(entry.get("ttft_ms"), int):
            tail.append(entry["terminal_ms"] - entry["ttft_ms"])
        if rollout["first_activity"]:
            provider_first.append(round((rollout["first_activity"] - rollout["prompt"]).total_seconds() * 1000))
            if rollout["first_text"]:
                think_tools.append(round((rollout["first_text"] - rollout["first_activity"]).total_seconds() * 1000))

    def line(label, values):
        if not values:
            print(f"{label:34s} 无样本")
            return
        print(f"{label:34s} n={len(values):3d} 中位={median(values) / 1000:8.1f}s "
              f"p90={p90(values) / 1000:8.1f}s max={max(values) / 1000:8.1f}s")

    print("\n== 我方分段（我们侧）")
    line("轮前开销 spawn→session_ready", pre_round)
    line("首字 ttft（我方可见首字节）", ttft)
    line("发射缓冲 display−ttft", gaps)
    line("首字→轮末（生成尾部）", tail)
    print("\n== provider 侧分段（原生转录）")
    line("prompt→模型首个活动", provider_first)
    line("首个活动→首条正文（思考+工具）", think_tools)

    if ttft_pairs:
        heavy = [t for c, t in ttft_pairs if c >= 5]
        light = [t for c, t in ttft_pairs if c == 0]
        print("\n== 工具轮次与首字延迟")
        line(f"工具轮 >=5 的 ttft（n={len(heavy)}）", heavy)
        line(f"工具轮 0 的 ttft（n={len(light)}）", light)
        if heavy and light:
            print(f"中位倍数 = {median(heavy) / max(median(light), 1):.2f}x")
        rho = spearman([c for c, _ in ttft_pairs], [t for _, t in ttft_pairs])
        if rho is not None:
            print(f"Spearman(工具轮次, ttft) = {rho:.3f}（n={len(ttft_pairs)}）")

    unknown_client = sum(1 for entry in matched if entry.get("client_receive") == "unknown")
    print(f"\nclient_receive 未埋点：{unknown_client}/{len(matched)} 轮")
    batches = sorted({entry["batch"] for entry in matched})
    print(f"覆盖批次 {len(batches)} 个：{batches}")


if __name__ == "__main__":
    main()
