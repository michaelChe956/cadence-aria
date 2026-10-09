#!/usr/bin/env bash
# lcg_matrix_watchdog.sh — LC 网关多 provider 矩阵看门狗 v3(MaxParallel3 三件套之二)。
#
# v2(/tmp/lcg-watchdog-any.sh)三处判定的结构性修复:
#   1) UTC/本地混用假 stale:v2 把 harness 足迹 `[fp HH:MM:SS]`(Utc::now,
#      harness.rs fp())当本地时间 `date -d` 解析,时区差被计入事件年龄,
#      产生 540min 级假 stale(实证:/tmp/lcg-watchdog-codex-7.state 在
#      EXIT=0 后仍持续报 stale)。v3 完全不解析日志时间戳——以本进程
#      单调时钟(/proc/uptime)记录「实质事件到达时间」,日志时间戳写错、
#      跨午夜、UTC/本地差均不影响判定。
#   2) 一家结束被别家掩盖:v2 用全局 `pgrep -f lcg_live_` 判存活,任一家
#      还在跑就把已结束家判活。v3 绑定本家根 PID + /proc/<pid>/stat 第
#      22 字段 starttime:PID 消失或 starttime 失配(PID 复用)都按本家
#      TEST_ENDED 归属处理并退出监控。
#   3) 全局 CLI 计数弱信号:v2 全局数 claude CLI。v3 只在本家根 PID 的
#      递归子树内按本家 provider CLI 模式计数,并排除含 `lcg_live_` 的
#      cargo/测试包装进程。
# 与 v2 一致:只记录、不杀进程。误判会诱导错误人工介入,判定必须精确。
#
# 用法:
#   lcg_matrix_watchdog.sh --provider codex --pid <cargo_pid> --log <log> --state <state> \
#       [--cli-pattern ERE] [--heartbeat-ere ERE] [--stale-after SEC] [--interval SEC] \
#       [--expect-starttime TICKS] [--once]
#   lcg_matrix_watchdog.sh --print-subtree PID          # 输出 PID+全部后代(启动器屏障用)
#   lcg_matrix_watchdog.sh --self-test                  # 伪日志+伪时钟驱动判定自测
#
# 判定语义(与设计 local://parallel3-design.md 监控表对齐):
#   WARMUP          尚未观察到任何实质 [fp 事件(不算 stale)。
#   OK              最近实质事件年龄 <= --stale-after(默认 1200s)。
#   STALE_PROGRESS  最近实质事件年龄 >  --stale-after(单家诊断信号,保留他家)。
#   TEST_ENDED      本家根 PID 消失(pid_gone)或 starttime 失配(pid_reused)。
set -euo pipefail

readonly HEARTBEAT_DEFAULT_ERE='type=ping|pump_idle_ping|health_poll'
readonly STALE_AFTER_DEFAULT=1200
readonly INTERVAL_DEFAULT=30
# CLI cmd 匹配模式:词边界排除下划线/连字符/字母数字,避免 lcg_live_codex_
# 之类测试过滤器误中;仅在本家子树内匹配,全局无关进程不参与。
readonly CLI_ERE_BOUNDARY_LEFT='(^|[^a-z0-9_-])'
readonly CLI_ERE_BOUNDARY_RIGHT='([^a-z0-9_-]|$)'

# ---------------- 基元:单调时钟 ----------------
# 单调时钟 = /proc/uptime 字段1(boot 起秒,monotonic,不受墙钟跳变影响)。
# 自测/外部注入用 LCG_WD_CLOCK_FILE 覆盖(文件首行即返回值)。
monotonic_now() {
    if [[ -n "${LCG_WD_CLOCK_FILE:-}" && -r "${LCG_WD_CLOCK_FILE:-}" ]]; then
        head -n 1 "$LCG_WD_CLOCK_FILE"
        return 0
    fi
    cut -d' ' -f 1 /proc/uptime
}

# ---------------- 基元:进程身份与子树 ----------------
# /proc/<pid>/stat 的 comm 后字段序列(comm 可含空格/括号,必须按 `) ` 截断)。
pid_stat_tail() {
    local stat
    stat=$(cat "/proc/$1/stat" 2>/dev/null) || return 0
    printf '%s\n' "${stat##*) }"
}

# /proc/<pid>/stat 字段22 starttime(boot 起嘀嗒)= tail 第 20 字段。
# 空输出 = 进程不存在。PID 复用时该值改变,据此与 PID 名字解耦。
pid_starttime() {
    local tail_fields
    tail_fields=$(pid_stat_tail "$1")
    [[ -n "$tail_fields" ]] || return 0
    # shellcheck disable=SC2086
    set -- $tail_fields
    printf '%s\n' "${20}"
}

# /proc/<pid>/stat 字段4 ppid = tail 第 2 字段。
pid_ppid() {
    local tail_fields
    tail_fields=$(pid_stat_tail "$1")
    [[ -n "$tail_fields" ]] || return 0
    # shellcheck disable=SC2086
    set -- $tail_fields
    printf '%s\n' "${2}"
}

# 根 PID + 全部后代(每行一个)。纯 /proc PPid 链闭包,无 pgrep。
subtree_pids() {
    local root=$1 entry pid ppid frontier next
    local -A ppid_map=()
    for entry in /proc/[0-9]*/stat; do
        [[ -r "$entry" ]] || continue
        pid=${entry#/proc/}
        pid=${pid%/stat}
        ppid=$(pid_ppid "$pid")
        [[ -n "$ppid" && "$ppid" != 0 ]] && ppid_map[$ppid]+=" $pid"
    done
    frontier=$root
    local -A seen=()
    while [[ -n "$frontier" ]]; do
        next=""
        for pid in $frontier; do
            [[ -n "${seen[$pid]:-}" ]] && continue
            seen[$pid]=1
            printf '%s\n' "$pid"
            next+="${ppid_map[$pid]:-}"
        done
        frontier=${next# }
    done
}

cmdline_of() {
    # 内核线程无 cmdline(文件不存在),静默返回空。
    [[ -r "/proc/$1/cmdline" ]] || return 0
    tr '\0' ' ' < "/proc/$1/cmdline" || true
}

# 本家子树内匹配 CLI 模式的进程数;排除含 lcg_live_ 的 cargo/测试包装
# (测试过滤器携带 provider 名,是包装进程不是 provider CLI)。
cli_count_in_subtree() {
    local root=$1 pattern=$2 pid cmdline count=0
    for pid in $(subtree_pids "$root"); do
        cmdline=$(cmdline_of "$pid")
        [[ -n "$cmdline" ]] || continue
        [[ "$cmdline" == *lcg_live_* ]] && continue
        if printf '%s' "$cmdline" | grep -qE -- "$pattern"; then
            count=$((count + 1))
        fi
    done
    printf '%s\n' "$count"
}

# ---------------- 基元:日志实质事件(到达时间口径) ----------------
# 读 log 自 PROCESSED_LINES 之后新增的完整行(wc -l 只数换行,天然不含
# 未收尾半行);推进 PROCESSED_LINES;新行经 stdin 语义过滤。
# 文件截断/轮转(total < processed)时重置从 0 计。
read_new_lines() {
    local log=$1 total
    NEW_LINES=""
    [[ -r "$log" ]] || return 0
    total=$(wc -l < "$log" 2>/dev/null) || total=0
    if (( total < PROCESSED_LINES )); then
        PROCESSED_LINES=0
    fi
    if (( total > PROCESSED_LINES )); then
        NEW_LINES=$(tail -n +"$((PROCESSED_LINES + 1))" "$log" | head -n "$((total - PROCESSED_LINES))")
        PROCESSED_LINES=$total
    fi
}

# stdin → 保留实质 [fp 事件行(排除心跳类;默认 type=ping/pump_idle_ping/
# health_poll,--heartbeat-ere 可覆盖)。绝不解析行内 HH:MM:SS。
meaningful_fp_lines() {
    grep -E '\[fp ' | grep -vE -- "$HEARTBEAT_ERE" || true
}

# stale 判定纯函数:仅依赖单调时钟数值,墙钟/日志时间戳无关——v2 的
# 「UTC 当本地」小时级假 stale 在此结构性不可达。
is_stale() {
    local now=$1 last=$2 threshold=$3
    awk -v n="$now" -v l="$last" -v t="$threshold" 'BEGIN { exit !(n - l > t) }'
}

# ---------------- 状态记录 ----------------
record() {
    printf '%s mono=%s %s\n' "$(date -u +%FT%TZ)" "$(monotonic_now)" "$*" >>"$STATE" 2>/dev/null || true
}

# ---------------- 主判定循环 ----------------
PROCESSED_LINES=0
NEW_LINES=""
LAST_ARRIVAL=""
ENDED=0

evaluate_once() {
    local observed st now age verdict cli
    observed=$(pid_starttime "$ROOT_PID")
    if [[ -z "$observed" ]]; then
        record "TEST_ENDED root_pid=$ROOT_PID reason=pid_gone provider=$PROVIDER"
        ENDED=1
        return 0
    fi
    if [[ "$observed" != "$EXPECTED_STARTTIME" ]]; then
        record "TEST_ENDED root_pid=$ROOT_PID reason=pid_reused observed_starttime=$observed expected_starttime=$EXPECTED_STARTTIME provider=$PROVIDER"
        ENDED=1
        return 0
    fi
    read_new_lines "$LOG"
    if [[ -n "$NEW_LINES" ]] && printf '%s\n' "$NEW_LINES" | meaningful_fp_lines | grep -q .; then
        # 到达时间口径:事件在本周期抵达 tailer 即记龄,不解析日志内时间戳。
        LAST_ARRIVAL=$(monotonic_now)
    fi
    now=$(monotonic_now)
    cli=$(cli_count_in_subtree "$ROOT_PID" "$CLI_ERE")
    # 子树快照旁路落盘(启动器阶段屏障 drain 用;每周期覆盖)。
    subtree_pids "$ROOT_PID" >"$SUBTREE_FILE" 2>/dev/null || true
    if [[ -z "$LAST_ARRIVAL" ]]; then
        verdict=WARMUP
        age="-1"
    else
        age=$(awk -v n="$now" -v l="$LAST_ARRIVAL" 'BEGIN { printf "%.1f", n - l }')
        if is_stale "$now" "$LAST_ARRIVAL" "$STALE_AFTER"; then
            verdict=STALE_PROGRESS
        else
            verdict=OK
        fi
    fi
    record "HEARTBEAT verdict=$verdict provider=$PROVIDER root_pid=$ROOT_PID cli_subtree=$cli last_event_age=${age}s processed_lines=$PROCESSED_LINES"
    if [[ "$verdict" == STALE_PROGRESS ]]; then
        record "*** STALE_PROGRESS: ${age}s 无实质事件(仅心跳)——按 Ruling 8 诊断本家,保留他家 ***"
    fi
}

run_loop() {
    local st0
    st0=$(pid_starttime "$ROOT_PID")
    if [[ -z "$st0" ]]; then
        # 启动器装配看门狗与本家退出之间存在竞态:起步时根 PID 已死按
        # 本家 TEST_ENDED 归属记录,不算监控器错误。
        record "TEST_ENDED root_pid=$ROOT_PID reason=pid_gone provider=$PROVIDER note=root_dead_at_watchdog_start"
        exit 0
    fi
    if [[ -n "$EXPECT_STARTTIME" ]]; then
        EXPECTED_STARTTIME=$EXPECT_STARTTIME
    else
        EXPECTED_STARTTIME=$st0
    fi
    record "START provider=$PROVIDER root_pid=$ROOT_PID expected_starttime=$EXPECTED_STARTTIME cli_ere=$CLI_ERE stale_after=${STALE_AFTER}s interval=${INTERVAL}s log=$LOG"
    while :; do
        evaluate_once
        if [[ "$ENDED" == 1 || "$ONCE" == 1 ]]; then
            break
        fi
        sleep "$INTERVAL"
    done
    if [[ "$ENDED" == 1 && "$ONCE" != 1 ]]; then
        record "MONITOR_EXIT root_pid=$ROOT_PID provider=$PROVIDER(本家收束,监控结束)"
    fi
    exit 0
}

# ---------------- 参数 ----------------
PROVIDER=""
ROOT_PID=""
LOG=""
STATE=""
CLI_ERE=""
HEARTBEAT_ERE=$HEARTBEAT_DEFAULT_ERE
STALE_AFTER=$STALE_AFTER_DEFAULT
INTERVAL=$INTERVAL_DEFAULT
EXPECT_STARTTIME=""
ONCE=0

provider_cli_ere() {
    case $1 in
        claude) printf '%sclaude%s\n' "$CLI_ERE_BOUNDARY_LEFT" "$CLI_ERE_BOUNDARY_RIGHT" ;;
        codex) printf '%scodex%s\n' "$CLI_ERE_BOUNDARY_LEFT" "$CLI_ERE_BOUNDARY_RIGHT" ;;
        pi) printf '%spi%s\n' "$CLI_ERE_BOUNDARY_LEFT" "$CLI_ERE_BOUNDARY_RIGHT" ;;
        kimi | kimi-code) printf '%skimi%s\n' "$CLI_ERE_BOUNDARY_LEFT" "$CLI_ERE_BOUNDARY_RIGHT" ;;
        *) return 1 ;;
    esac
}

usage() {
    sed -n '3,27p' "$0" | sed 's/^# \{0,1\}//'
}

main() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --provider) PROVIDER=$2; shift 2 ;;
            --pid) ROOT_PID=$2; shift 2 ;;
            --log) LOG=$2; shift 2 ;;
            --state) STATE=$2; shift 2 ;;
            --cli-pattern) CLI_ERE=$2; shift 2 ;;
            --heartbeat-ere) HEARTBEAT_ERE=$2; shift 2 ;;
            --stale-after) STALE_AFTER=$2; shift 2 ;;
            --interval) INTERVAL=$2; shift 2 ;;
            --expect-starttime) EXPECT_STARTTIME=$2; shift 2 ;;
            --once) ONCE=1; shift ;;
            --print-subtree) shift; subtree_pids "$1"; exit 0 ;;
            --self-test) shift; self_test "$@"; exit 0 ;;
            -h | --help) usage; exit 0 ;;
            *) echo "未知参数: $1" >&2; usage >&2; exit 2 ;;
        esac
    done
    [[ -n "$PROVIDER" ]] || { echo "--provider 必填(claude|codex|pi|kimi)" >&2; exit 2; }
    [[ -n "$ROOT_PID" ]] || { echo "--pid 必填(本家 cargo/测试进程)" >&2; exit 2; }
    [[ -n "$LOG" ]] || { echo "--log 必填" >&2; exit 2; }
    [[ -n "$STATE" ]] || { echo "--state 必填" >&2; exit 2; }
    if [[ -z "$CLI_ERE" ]]; then
        CLI_ERE=$(provider_cli_ere "$PROVIDER") || { echo "未知 provider: $PROVIDER" >&2; exit 2; }
    fi
    SUBTREE_FILE="${STATE}.subtree"
    : >"$SUBTREE_FILE" 2>/dev/null || true
    run_loop
}

# ---------------- 自测(伪日志 + 伪时钟驱动判定) ----------------
SELF_TEST_PIDS=()
self_test_cleanup() {
    local pid child
    for pid in "${SELF_TEST_PIDS[@]:-}"; do
        for child in $(subtree_pids "$pid"); do
            kill "$child" 2>/dev/null || true
        done
    done
}
trap self_test_cleanup EXIT

SELF_TEST_FAILED=0

st_pass() { echo "PASS: $1"; }
st_fail() { echo "FAIL: $1" >&2; SELF_TEST_FAILED=1; }

st_expect_eq() {
    local label=$1 got=$2 want=$3
    if [[ "$got" == "$want" ]]; then
        st_pass "$label"
    else
        st_fail "$label — got=[$got] want=[$want]"
    fi
}

self_test() {
    local tmp
    tmp=$(mktemp -d /tmp/lcg-wd-selftest.XXXXXX)

    # 1) 单调时钟:数值形态 + 伪时钟覆盖(子shell 防函数调用 env 泄漏)。
    if [[ $(monotonic_now) =~ ^[0-9]+\.[0-9][0-9]$ ]]; then
        st_pass "monotonic_now 输出 boot 秒(小数)"
    else
        st_fail "monotonic_now 形态异常: $(monotonic_now)"
    fi
    printf '100000.50\n' >"$tmp/clock"
    st_expect_eq "伪时钟覆盖生效(子 shell 无泄漏)" \
        "$(LCG_WD_CLOCK_FILE=$tmp/clock bash -c 'head -n1 "$LCG_WD_CLOCK_FILE"')" "100000.50"
    if [[ $(monotonic_now) != 100000.50 ]]; then
        st_pass "覆盖不泄漏出子 shell"
    else
        st_fail "LCG_WD_CLOCK_FILE 泄漏进本 shell"
    fi

    # 2) starttime:非空、稳定、进程间可区分。
    local st_a st_a2 st_b sleep_pid
    sleep 10 & SELF_TEST_PIDS+=($!)
    sleep_pid=$!
    st_a=$(pid_starttime "$$")
    st_a2=$(pid_starttime "$$")
    st_b=$(pid_starttime "$sleep_pid")
    if [[ -n "$st_a" ]]; then st_pass "starttime 非空"; else st_fail "starttime 为空"; fi
    st_expect_eq "starttime 稳定(两次一致)" "$st_a" "$st_a2"
    if [[ "$st_a" != "$st_b" ]]; then st_pass "starttime 进程间可区分"; else st_fail "两进程 starttime 相同"; fi

    # 3) 子树闭包:含子/孙,不含无关进程。
    local subtree_root subtree count unrelated_leak=0
    bash -c 'sleep 10 & sleep 10 & wait' &
    subtree_root=$!
    SELF_TEST_PIDS+=("$subtree_root")
    sleep 0.4
    subtree=$(subtree_pids "$subtree_root")
    count=$(printf '%s\n' "$subtree" | grep -c .)
    if (( count >= 3 )); then st_pass "子树含根+子+孙(共 $count)"; else st_fail "子树应≥3 个进程,得 $count"; fi
    if printf '%s\n' "$subtree" | grep -qx "$$"; then
        unrelated_leak=1
        st_fail "子树不应包含无关进程 $$"
    else
        st_pass "子树不含无关进程"
    fi

    # 4) CLI 计数:子树内匹配、lcg_live_ 包装排除。
    local cnt_before cnt_after wrapper
    cnt_before=$(cli_count_in_subtree "$subtree_root" "${CLI_ERE_BOUNDARY_LEFT}sleep${CLI_ERE_BOUNDARY_RIGHT}")
    bash -c 'exec -a lcg_live_fake_sleep_marker sleep 10' &
    wrapper=$!
    SELF_TEST_PIDS+=("$wrapper")
    sleep 0.3
    cnt_after=$(cli_count_in_subtree "$subtree_root" "${CLI_ERE_BOUNDARY_LEFT}sleep${CLI_ERE_BOUNDARY_RIGHT}")
    st_expect_eq "lcg_live_ 包装进程不计入 CLI(argv0 含过滤词)" "$cnt_after" "$cnt_before"

    # 5) 日志 tailer:完整行才消费、心跳排除、绝不解析 HH:MM:SS。
    local fixture="$tmp/fake.log" meaningful
    : >"$fixture"
    PROCESSED_LINES=0
    read_new_lines "$fixture"
    st_expect_eq "空日志无新行" "$NEW_LINES" ""
    cat >>"$fixture" <<'EOF'
[fp 23:59:59.999] [matrix/env/build] phase_begin matrix/env/build
[fp 00:00:01.123] [matrix/env/build] health_poll provider health 未就绪,继续轮询(30s 心跳)
[fp 08:00:00.001] [matrix/env/provider_trust_ensure] provider_trust_ready Codex=Register
EOF
    read_new_lines "$fixture"
    meaningful=$(printf '%s\n' "$NEW_LINES" | meaningful_fp_lines)
    st_expect_eq "心跳排除后仅剩实质事件(UTC 时间戳写啥都不参与判定)" \
        "$(printf '%s\n' "$meaningful" | grep -c 'provider_trust_ready')" "1"
    printf '[fp 08:00:05.000] [matrix/env/build] half_line_no_newline' >>"$fixture"
    read_new_lines "$fixture"
    st_expect_eq "半行(无换行)不消费" "$PROCESSED_LINES" "3"
    printf ' tail_done\n' >>"$fixture"
    read_new_lines "$fixture"
    st_expect_eq "补齐换行后消费" "$PROCESSED_LINES" "4"

    # 6) stale 判定纯函数:只看单调数值,墙钟不可达。
    if ! is_stale 1000.0 100.0 1200; then st_pass "age=900s<=1200s 判非 STALE"; else st_fail "age=900s 误判 STALE"; fi
    if is_stale 1301.0 100.0 1200; then st_pass "age=1201s>1200s 判 STALE"; else st_fail "age=1201s 未判 STALE"; fi
    if is_stale 100000.51 100000.50 0; then st_pass "伪时钟 0.01s/阈值0 判 STALE(单调口径)"; else st_fail "伪时钟 stale 判定"; fi

    # 7) TEST_ENDED 归属自家:本家根死→ENDED;他家存活不掩盖(不再全局 pgrep)。
    local fam_a fam_b state_a state_b
    sleep 10 & fam_a=$!
    SELF_TEST_PIDS+=("$fam_a")
    sleep 10 & fam_b=$!
    kill "$fam_b" 2>/dev/null || true
    wait "$fam_b" 2>/dev/null || true
    state_a="$tmp/fam-a.state"
    state_b="$tmp/fam-b.state"
    LCG_WD_CLOCK_FILE=$tmp/clock "$0" --once --provider codex --pid "$fam_a" \
        --log "$fixture" --state "$state_a" --cli-pattern 'codex' >/dev/null 2>&1 || true
    LCG_WD_CLOCK_FILE=$tmp/clock "$0" --once --provider pi --pid "$fam_b" \
        --log "$fixture" --state "$state_b" --cli-pattern 'pi' >/dev/null 2>&1 || true
    if grep -q 'TEST_ENDED.*reason=pid_gone' "$state_b"; then
        st_pass "本家根死记 TEST_ENDED(pid_gone)"
    elif grep -q 'TEST_ENDED' "$state_b"; then
        st_pass "本家根死记 TEST_ENDED(PID 即时复用按 starttime 失配归属)"
    else
        st_fail "本家根死未记 TEST_ENDED"
    fi
    if grep -q 'TEST_ENDED' "$state_a"; then
        st_fail "他家(fam-a)存活却被判 ENDED"
    else
        st_pass "他家存活不受本家死亡影响(v2 全局 pgrep 掩盖缺陷修复)"
    fi
    st_expect_eq "存活家 verdict(到达时间口径,伪时钟零龄)" \
        "$(grep -o 'verdict=[A-Z_]*' "$state_a" | tail -1)" "verdict=OK"

    # 8) PID 复用:starttime 失配按本家结束处理。
    local state_c="$tmp/fam-c.state"
    "$0" --once --provider codex --pid "$fam_a" --log "$fixture" --state "$state_c" \
        --cli-pattern 'codex' --expect-starttime 99999999 >/dev/null 2>&1 || true
    if grep -q 'TEST_ENDED.*reason=pid_reused' "$state_c"; then
        st_pass "starttime 失配记 TEST_ENDED(pid_reused)"
    else
        st_fail "PID 复用判定失效"
    fi

    rm -rf "$tmp"
    if [[ "$SELF_TEST_FAILED" == 1 ]]; then
        echo "self-test: 存在失败项" >&2
        exit 1
    fi
    echo "self-test: 全部通过"
}

main "$@"
