#!/usr/bin/env bash
# lcg_matrix_batch.sh — LC 网关多 provider 矩阵两阶段批启动器(MaxParallel3 三件套之三)。
#
# 形态(设计 local://parallel3-design.md 方案 B + OracleParallel3 终裁):
#   1) 预构建一次:cargo test --locked --test it_web --no-run(经 Cargo 包装,
#      保留 rust-toolchain/.cargo/config 的 jobs/栈配置;不绕过 Cargo 直 exec)。
#   2) 阶段一(capture/full)串行:批内所有需要 capture/full 的 provider 按
#      codex→pi→kimi→claude 顺序逐一跑完——HOME machine_skills 写面
#      (PREPARE_LOCK 仅进程内)与固定 Claude root recipe 必须串行。
#   3) 阶段屏障:阶段一每家收束后 drain 其看门狗子树快照(逐 PID 有界等待
#      消失),再加咨询级有界全局兜底(仅 /tmp 证据根 cwd 的同家 CLI),
#      全部收束才允许阶段二启动;资源红线触发则阶段二降级串行。
#   4) 阶段二(resume)并行:resume 家并行启动(每家独立 env/log/state/
#      看门狗;--test-threads=1 只约束各自 libtest 进程,不串行化三家)。
#   5) 串行回退开关:--policy serial(默认,金丝雀通过前的保守档)|
#      resume_parallel;env LCG_MATRIX_BATCH_POLICY 同义。serial 只改外层
#      调度,不改测试、provider filters、快照门禁与验收口径。
#
# 资源红线(金丝雀前的候选值,env 可覆盖,运行中只记录不杀):
#   MemAvailable < 6GiB(LCG_REDLINE_MEM_BYTES=6442450944)
#   SwapUsed 较批基线增长 > 2GiB(LCG_REDLINE_SWAP_GROWTH_BYTES=2147483648)
#   load5 > 14(LCG_REDLINE_LOAD5=14)
#   cgroup memory.events oom/oom_kill 增量(LCG_CGROUP_EVENTS 指定文件)
#
# 隔离纪律(设计「可直接落地的启动形态」):
#   每条命令独立 env -u LIVE_MATRIX_SNAPSHOT_ID(禁批级全局 export snapshot
#   id)+ LC_GATEWAY_E2E=1 + LIVE_MATRIX_RUN_MODE=<本家模式>;每家独立日志、
#   看门狗状态;批中冻结 HEAD(禁 commit/rebase/cargo clean——启动器自身
#   对仓库只读)。子进程经「直下子 + 递归子树看门狗」跟踪:不用 setsid
#   包装层,根 PID 即 cargo 进程,PID+starttime+exit code 全精确。
#
# 用法:
#   scripts/lcg_matrix_batch.sh --capture codex --resume pi,kimi --policy resume_parallel
#   scripts/lcg_matrix_batch.sh --full codex --resume pi,kimi
#   scripts/lcg_matrix_batch.sh --resume codex,pi,kimi --policy serial
#   scripts/lcg_matrix_batch.sh --dry-run ...      # 打印完整计划与预检,不执行
#   scripts/lcg_matrix_batch.sh --self-test        # 资源解析/排序/红线/计划自测
set -euo pipefail

CANONICAL_ORDER="codex pi kimi claude"
DEFAULT_POLICY=serial

# 阈值读取器(非 readonly:env 可按机覆盖,自测可注入)。
threshold_mem() { echo "${LCG_REDLINE_MEM_BYTES:-6442450944}"; }                 # 6GiB
threshold_swap_growth() { echo "${LCG_REDLINE_SWAP_GROWTH_BYTES:-2147483648}"; } # 2GiB
threshold_load5() { echo "${LCG_REDLINE_LOAD5:-14}"; }
cgroup_events_file() { echo "${LCG_CGROUP_EVENTS:-/sys/fs/cgroup/memory.events}"; }
barrier_grace_seconds() { echo "${LCG_BARRIER_GRACE_SECONDS:-300}"; }
sample_interval() { echo "${LCG_SAMPLE_INTERVAL:-30}"; }
stale_after_setting() { echo "${LCG_STALE_AFTER:-1200}"; }
watch_interval_setting() { echo "${LCG_WATCH_INTERVAL:-30}"; }

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
WATCHDOG="$SCRIPT_DIR/lcg_matrix_watchdog.sh"

CAPTURE_LIST=""
FULL_LIST=""
RESUME_LIST=""
POLICY=""
DRY_RUN=0
SKIP_PREBUILD=0
LOG_DIR=""
SAMPLER_PID=""
BASELINE_SWAP=0
BASELINE_OOM=0
BASELINE_OOM_KILL=0

usage() {
    sed -n '3,36p' "$0" | sed 's/^# \{0,1\}//'
}

die() {
    echo "错误: $*" >&2
    exit 2
}

batch_record() {
    printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >>"$BATCH_STATE" 2>/dev/null || true
}

# ---------------- provider 词表 ----------------
provider_short() {
    case $1 in
        codex) echo codex ;;
        pi) echo pi ;;
        kimi | kimi-code) echo kimi ;;
        claude) echo claude ;;
        *) return 1 ;;
    esac
}

# 逗号列表 → 按规范顺序(codex→pi→kimi→claude)去重输出(空列表输出空)。
normalize_provider_list() {
    local raw=$1 item short seen="" ordered="" candidate canonical
    [[ -z "$raw" ]] && return 0
    local -A seen_map=()
    local -a items=()
    IFS=',' read -ra items <<<"$raw"
    for item in "${items[@]}"; do
        item=${item//[[:space:]]/}
        [[ -z "$item" ]] && continue
        short=$(provider_short "$item") || die "未知 provider: $item(合法:codex|pi|kimi|claude)"
        [[ -n "${seen_map[$short]:-}" ]] && continue
        seen_map[$short]=1
        seen+=" $short"
    done
    for canonical in $CANONICAL_ORDER; do
        for candidate in $seen; do
            [[ "$candidate" == "$canonical" ]] && ordered+=" $canonical"
        done
    done
    echo "${ordered# }"
}

list_contains() {
    local list=$1 want=$2 item
    [[ -z "$list" ]] && return 1
    for item in $list; do
        [[ "$item" == "$want" ]] && return 0
    done
    return 1
}

# ---------------- 资源采样与红线 ----------------
mem_available_bytes() {
    awk '/^MemAvailable:/ { print $2 * 1024; exit }' /proc/meminfo
}

swap_used_bytes() {
    awk '/^SwapTotal:/ { t = $2 } /^SwapFree:/ { f = $2 } END { print (t - f) * 1024 }' /proc/meminfo
}

load5() {
    awk '{ print $2; exit }' /proc/loadavg
}

# 输出 "oom oom_kill";文件缺失输出 "0 0"(事件文件不可依赖时零基线)。
cgroup_oom_counters() {
    local file oom="" oom_kill=""
    file=$(cgroup_events_file)
    if [[ -r "$file" ]]; then
        oom=$(awk '$1 == "oom" { print $2; exit }' "$file")
        oom_kill=$(awk '$1 == "oom_kill" { print $2; exit }' "$file")
    fi
    echo "${oom:-0} ${oom_kill:-0}"
}

# 输出触发原因(空=未触发)。红线数值比较全部走 awk(浮点安全)。
evaluate_redlines() {
    local mem swap load oom oom_kill reasons=""
    mem=$(mem_available_bytes)
    swap=$(swap_used_bytes)
    load=$(load5)
    read -r oom oom_kill <<<"$(cgroup_oom_counters)"
    if awk -v m="$mem" -v t="$(threshold_mem)" 'BEGIN { exit !(m < t) }'; then
        reasons+="mem_below_redline(mem=${mem}B) "
    fi
    if awk -v s="$swap" -v b="$BASELINE_SWAP" -v t="$(threshold_swap_growth)" 'BEGIN { exit !(s - b > t) }'; then
        reasons+="swap_growth_over_redline(delta=$((swap - BASELINE_SWAP))B) "
    fi
    if awk -v l="$load" -v t="$(threshold_load5)" 'BEGIN { exit !(l > t) }'; then
        reasons+="load5_over_redline(load5=$load) "
    fi
    if (( ${oom:-0} > BASELINE_OOM || ${oom_kill:-0} > BASELINE_OOM_KILL )); then
        reasons+="cgroup_oom(oom=$oom oom_kill=$oom_kill baseline=$BASELINE_OOM/$BASELINE_OOM_KILL) "
    fi
    echo "$reasons"
}

resource_snapshot_line() {
    local counters
    counters=$(cgroup_oom_counters)
    printf 'RESOURCE mem_avail=%sB swap_used=%sB load5=%s oom=%s oom_kill=%s' \
        "$(mem_available_bytes)" "$(swap_used_bytes)" "$(load5)" \
        "${counters% *}" "${counters#* }"
}

start_resource_sampler() {
    (
        while :; do
            echo "$(date -u +%FT%TZ) $(resource_snapshot_line)" >>"${BATCH_STATE}.resource"
            local reasons
            reasons=$(evaluate_redlines)
            if [[ -n "$reasons" ]]; then
                echo "$(date -u +%FT%TZ) REDLINE $reasons" >>"${BATCH_STATE}.resource"
                : >"$LOG_DIR/REDLINE"
            fi
            sleep "$(sample_interval)"
        done
    ) &
    SAMPLER_PID=$!
}

stop_resource_sampler() {
    if [[ -n "${SAMPLER_PID:-}" ]]; then
        kill "$SAMPLER_PID" 2>/dev/null || true
        wait "$SAMPLER_PID" 2>/dev/null || true
        SAMPLER_PID=""
    fi
}

# ---------------- 阶段屏障 ----------------
# /proc/<pid>/stat 字段22(comm 后第 20 字段)——与看门狗同口径,comm 含
# 空格也不失位。
stat_starttime() {
    local stat_line tail_fields
    stat_line=$(cat "/proc/$1/stat" 2>/dev/null) || return 0
    tail_fields=${stat_line##*) }
    # shellcheck disable=SC2086
    set -- $tail_fields
    printf '%s\n' "${20}"
}

# drain 一家收束后的遗留子树:看门狗每周期把根子树快照写 <state>.subtree;
# 逐 PID 等待消失。每个 PID 有界(barrier 宽限),超时记录放行——后续
# advisory 阶段兜底。快照缺失视为已收束。
drain_subtree_snapshot() {
    local provider=$1 subtree_file=$2 pid waited
    [[ -s "$subtree_file" ]] || return 0
    for pid in $(cat "$subtree_file"); do
        waited=0
        while [[ -d "/proc/$pid" ]] && (( waited < "$(barrier_grace_seconds)" )); do
            sleep 5
            waited=$((waited + 5))
        done
        if [[ -d "/proc/$pid" ]]; then
            batch_record "BARRIER_DRAIN_TIMEOUT provider=$provider pid=$pid 仍存活(宽限 $(barrier_grace_seconds)s),交由 advisory 兜底"
        fi
    done
    batch_record "BARRIER_DRAIN provider=$provider subtree_drained=1"
}

# 咨询级有界兜底:根退出瞬间新孤儿子进程可能不在任何快照内;按本阶段
# provider CLI 词形全局找候选,再以 cwd 是否在 /tmp(证据根 TempDir 族)
# 过滤,排除宿主用户自己的同名 CLI。超时只记录放行,不无限阻塞。
advisory_lingering_cli_check() {
    local providers=$1 short ere candidate waited found cwd
    for short in $providers; do
        case $short in
            codex) ere='(^|[^a-z0-9_-])codex([^a-z0-9_-]|$)' ;;
            pi) ere='(^|[^a-z0-9_-])pi([^a-z0-9_-]|$)' ;;
            kimi) ere='(^|[^a-z0-9_-])kimi([^a-z0-9_-]|$)' ;;
            claude) ere='(^|[^a-z0-9_-])claude([^a-z0-9_-]|$)' ;;
            *) continue ;;
        esac
        waited=0
        found=0
        candidate=""
        while (( waited < "$(barrier_grace_seconds)" )); do
            found=0
            for candidate in $(pgrep -f -- "$ere" 2>/dev/null || true); do
                [[ "$candidate" == "$$" || "$candidate" == "$PPID" ]] && continue
                cwd=$(readlink "/proc/$candidate/cwd" 2>/dev/null || true)
                [[ "$cwd" == /tmp || "$cwd" == /tmp/* ]] || continue
                found=1
                break
            done
            (( found == 0 )) && break
            batch_record "BARRIER_ADVISORY_WAIT provider=$short lingering_cli_pid=$candidate cwd=$cwd waited=${waited}s"
            sleep 5
            waited=$((waited + 5))
        done
        if (( found == 1 )); then
            batch_record "BARRIER_ADVISORY_TIMEOUT provider=$short 宽限 $(barrier_grace_seconds)s 后仍有 /tmp 证据根下的本家 CLI,放行并行段(人工核对)"
        fi
    done
}

# ---------------- 单家执行(串行段) ----------------
run_provider() {
    local provider=$1 mode=$2 phase=$3
    local short log state pid exit_code=0 started ended wd_pid
    short=$(provider_short "$provider")
    log=$LOG_DIR/$short.log
    state=$LOG_DIR/$short.watchdog.state
    started=$(date +%s)
    batch_record "PROVIDER_START provider=$short phase=$phase mode=$mode log=$log"
    env -u LIVE_MATRIX_SNAPSHOT_ID LC_GATEWAY_E2E=1 LIVE_MATRIX_RUN_MODE=$mode \
        cargo test --locked --test it_web "lcg_live_${short}_five_stages_fresh_resume" \
        -- --ignored --nocapture --test-threads=1 >"$log" 2>&1 &
    pid=$!
    "$WATCHDOG" --provider "$short" --pid "$pid" --log "$log" --state "$state" \
        --stale-after "$(stale_after_setting)" --interval "$(watch_interval_setting)" &
    wd_pid=$!
    set +e
    wait "$pid"
    exit_code=$?
    set -e
    ended=$(date +%s)
    # 看门狗在根消失后一个周期内记录 TEST_ENDED 并退出。
    wait "$wd_pid" 2>/dev/null || true
    batch_record "PROVIDER_EXIT provider=$short phase=$phase exit=$exit_code duration=$((ended - started))s log=$log"
    PROVIDER_EXIT_CODES[$short]=$exit_code
    PROVIDER_SUBTREE_FILES[$short]="$state.subtree"
    return "$exit_code"
}

# ---------------- 计划打印(dry-run 共用) ----------------
print_plan() {
    local p short
    echo "policy=$POLICY"
    echo "log_dir=$LOG_DIR"
    echo "prebuild=$( (( SKIP_PREBUILD )) && echo skip || echo 'cargo test --locked --test it_web --no-run')"
    echo "phase1(serial, capture/full): ${PHASE1_PROVIDERS:-<无>}"
    for p in $CAPTURE_ORDERED; do
        short=$p
        echo "  [$short] env -u LIVE_MATRIX_SNAPSHOT_ID LC_GATEWAY_E2E=1 LIVE_MATRIX_RUN_MODE=capture_plan_snapshot cargo test --locked --test it_web lcg_live_${short}_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1 >$LOG_DIR/$short.log 2>&1"
    done
    for p in $FULL_ORDERED; do
        short=$p
        echo "  [$short] env -u LIVE_MATRIX_SNAPSHOT_ID LC_GATEWAY_E2E=1 LIVE_MATRIX_RUN_MODE=full_chain cargo test --locked --test it_web lcg_live_${short}_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1 >$LOG_DIR/$short.log 2>&1"
    done
    echo "phase2($PHASE2_DESC, resume): ${PHASE2_PROVIDERS:-<无>}"
    for p in $PHASE2_PROVIDERS; do
        short=$p
        echo "  [$short] env -u LIVE_MATRIX_SNAPSHOT_ID LC_GATEWAY_E2E=1 LIVE_MATRIX_RUN_MODE=resume_from_plan_snapshot cargo test --locked --test it_web lcg_live_${short}_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1 >$LOG_DIR/$short.log 2>&1"
    done
    echo "watchdog/家: $WATCHDOG --provider <short> --pid <cargo_pid> --log <log> --state <state> --stale-after $(stale_after_setting) --interval $(watch_interval_setting)"
    echo "barrier: 子树快照 drain + 咨询级 CLI 兜底(宽限 $(barrier_grace_seconds)s) + 资源红线(触发则 phase2 降级串行)"
    echo "$(resource_snapshot_line) (当前)"
    echo "redlines: mem_avail<$(threshold_mem)B swap_growth>$(threshold_swap_growth)B load5>$(threshold_load5) oom/oom_kill 增量($(cgroup_events_file))"
}

# ---------------- 预检 ----------------
# 批原子性:上一批必须全部收束(lcg_live 测试过滤器词形为矩阵运行专属);
# 启动器对仓库只读,批中冻结 HEAD 由纪律保证。
preflight_no_active_matrix() {
    if pgrep -f -- 'lcg_live_.*five_stages' >/dev/null 2>&1; then
        die "已有 lcg_live 矩阵进程在运行——批原子性要求上一批全部收束后再启动"
    fi
}

# ---------------- 自测 ----------------
self_test() {
    local failed=0 mem swap load
    if [[ $(provider_short kimi-code) == kimi ]]; then echo "PASS: kimi-code 别名映射 kimi"; else echo "FAIL: kimi 别名"; failed=1; fi
    if [[ $(normalize_provider_list "kimi,codex,pi") == "codex pi kimi" ]]; then
        echo "PASS: 列表按 codex→pi→kimi 规范排序"
    else
        echo "FAIL: 排序: $(normalize_provider_list 'kimi,codex,pi')"
        failed=1
    fi
    if [[ $(normalize_provider_list "codex, codex") == "codex" ]]; then echo "PASS: 去重"; else echo "FAIL: 去重"; failed=1; fi
    if [[ $(normalize_provider_list "") == "" ]]; then echo "PASS: 空列表"; else echo "FAIL: 空列表"; failed=1; fi
    if (normalize_provider_list "openai") >/dev/null 2>&1; then echo "FAIL: 非法 provider 未拒绝"; failed=1; else echo "PASS: 非法 provider 拒绝"; fi
    mem=$(mem_available_bytes)
    swap=$(swap_used_bytes)
    load=$(load5)
    if [[ $mem =~ ^[0-9]+$ ]] && (( mem > 1048576 )); then echo "PASS: MemAvailable 解析(${mem}B)"; else echo "FAIL: MemAvailable=$mem"; failed=1; fi
    if [[ $swap =~ ^[0-9]+$ ]]; then echo "PASS: SwapUsed 解析(${swap}B)"; else echo "FAIL: SwapUsed=$swap"; failed=1; fi
    if [[ $load =~ ^[0-9.]+$ ]]; then echo "PASS: load5 解析($load)"; else echo "FAIL: load5=$load"; failed=1; fi
    # 红线判定:子 shell 注入阈值,压低必触发、全宽松零误报(不污染本进程)。
    if [[ -n "$( (BASELINE_SWAP=$swap BASELINE_OOM=0 BASELINE_OOM_KILL=0 LCG_REDLINE_MEM_BYTES=$((mem + 1)) evaluate_redlines) )" ]]; then
        echo "PASS: MemAvailable 低于阈值触发红线"
    else
        echo "FAIL: MemAvailable 红线未触发"
        failed=1
    fi
    if [[ -z "$( (BASELINE_SWAP=$swap BASELINE_OOM=0 BASELINE_OOM_KILL=0 LCG_REDLINE_MEM_BYTES=1 LCG_REDLINE_SWAP_GROWTH_BYTES=1 LCG_REDLINE_LOAD5=999999 evaluate_redlines) )" ]]; then
        echo "PASS: 全阈值宽松零误报"
    else
        echo "FAIL: 全阈值宽松仍误报"
        failed=1
    fi
    # starttime 解析:与自身 /proc 一致且非空。
    if [[ -n "$(stat_starttime "$$")" ]]; then echo "PASS: stat_starttime 解析"; else echo "FAIL: stat_starttime"; failed=1; fi
    # dry-run:计划完整(策略/两阶段/命令/红线)。
    local plan
    plan=$("$0" --dry-run --capture codex --resume pi,kimi --policy resume_parallel 2>/dev/null)
    if grep -q '^policy=resume_parallel' <<<"$plan" && grep -q 'phase1(serial, capture/full): codex' <<<"$plan" \
        && grep -q 'phase2(parallel, resume): pi kimi' <<<"$plan" \
        && grep -q 'LIVE_MATRIX_RUN_MODE=capture_plan_snapshot' <<<"$plan" \
        && grep -q 'LIVE_MATRIX_RUN_MODE=resume_from_plan_snapshot' <<<"$plan" \
        && grep -q 'env -u LIVE_MATRIX_SNAPSHOT_ID' <<<"$plan" \
        && grep -q 'redlines:' <<<"$plan"; then
        echo "PASS: dry-run 计划完整(两阶段+env 隔离+红线)"
    else
        echo "FAIL: dry-run 计划缺项" >&2
        grep -E '^(policy|phase[12])' <<<"$plan" | sed 's/^/    /' >&2
        failed=1
    fi
    # 互斥校验:同家同时 capture+resume 必须拒绝。
    if "$0" --dry-run --capture codex --resume codex >/dev/null 2>&1; then
        echo "FAIL: 模式互斥未生效"
        failed=1
    else
        echo "PASS: 同家模式互斥拒绝"
    fi
    # serial 默认档:无 --policy 时 dry-run 报 serial。
    local default_plan
    default_plan=$("$0" --dry-run --resume codex 2>/dev/null)
    if [[ "$default_plan" == *$'\n'"policy=serial"* || "$default_plan" == "policy=serial"* ]]; then
        echo "PASS: 默认策略 serial(金丝雀前保守档)"
    else
        echo "FAIL: 默认策略非 serial"
        failed=1
    fi
    if (( failed )); then
        echo "self-test: 存在失败项" >&2
        exit 1
    fi
    echo "self-test: 全部通过"
}
main() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --capture) CAPTURE_LIST=$2; shift 2 ;;
            --full) FULL_LIST=$2; shift 2 ;;
            --resume) RESUME_LIST=$2; shift 2 ;;
            --policy) POLICY=$2; shift 2 ;;
            --dry-run) DRY_RUN=1; shift ;;
            --skip-prebuild) SKIP_PREBUILD=1; shift ;;
            --log-dir) LOG_DIR=$2; shift 2 ;;
            --self-test) self_test; exit 0 ;;
            -h | --help) usage; exit 0 ;;
            *) die "未知参数: $1" ;;
        esac
    done

    POLICY=${POLICY:-${LCG_MATRIX_BATCH_POLICY:-$DEFAULT_POLICY}}
    case $POLICY in
        serial | resume_parallel) ;;
        *) die "policy 非法: $POLICY(合法:serial|resume_parallel)" ;;
    esac
    [[ -n "$CAPTURE_LIST$FULL_LIST$RESUME_LIST" ]] || die "至少指定 --capture/--full/--resume 之一"

    CAPTURE_ORDERED=$(normalize_provider_list "$CAPTURE_LIST")
    FULL_ORDERED=$(normalize_provider_list "$FULL_LIST")
    RESUME_ORDERED=$(normalize_provider_list "$RESUME_LIST")

    local p
    for p in $CAPTURE_ORDERED; do
        list_contains "$RESUME_ORDERED" "$p" && die "provider $p 同时出现在 capture 与 resume——模式互斥"
        list_contains "$FULL_ORDERED" "$p" && die "provider $p 同时出现在 capture 与 full——模式互斥"
    done
    for p in $FULL_ORDERED; do
        list_contains "$RESUME_ORDERED" "$p" && die "provider $p 同时出现在 full 与 resume——模式互斥"
    done

    # 阶段一 = capture/full(串行,规范序);阶段二 = resume。
    local combined=""
    [[ -n "$CAPTURE_ORDERED" ]] && combined="$CAPTURE_ORDERED"
    [[ -n "$combined" && -n "$FULL_ORDERED" ]] && combined+=","
    [[ -n "$FULL_ORDERED" ]] && combined+="$FULL_ORDERED"
    PHASE1_PROVIDERS=$(normalize_provider_list "$combined")
    PHASE2_PROVIDERS=$RESUME_ORDERED
    if [[ "$POLICY" == serial ]]; then
        PHASE2_DESC=serial
    else
        PHASE2_DESC=parallel
    fi

    if (( DRY_RUN )); then
        LOG_DIR=${LOG_DIR:-/tmp/lcg-batch-dryrun}
        echo "== dry-run:批计划(不执行) =="
        print_plan
        if pgrep -f -- 'lcg_live_.*five_stages' >/dev/null 2>&1; then
            echo "预检: 存在活动 lcg_live 矩阵进程——真实批将被拒绝(批原子性)"
        else
            echo "预检: 无活动 lcg_live 矩阵进程"
        fi
        exit 0
    fi

    if [[ -z "$LOG_DIR" ]]; then
        LOG_DIR=$(mktemp -d "/tmp/lcg-batch-$(date -u +%Y%m%dT%H%M%SZ).XXXXXX")
    else
        mkdir -p "$LOG_DIR"
    fi
    BATCH_STATE=$LOG_DIR/batch.state
    : >"$BATCH_STATE"

    BASELINE_SWAP=$(swap_used_bytes)
    read -r BASELINE_OOM BASELINE_OOM_KILL <<<"$(cgroup_oom_counters)"

    preflight_no_active_matrix
    batch_record "BATCH_START policy=$POLICY phase1=[${PHASE1_PROVIDERS:-}] phase2=[${PHASE2_PROVIDERS:-}]"
    batch_record "$(resource_snapshot_line) (baseline)"

    trap stop_resource_sampler EXIT
    start_resource_sampler

    if (( ! SKIP_PREBUILD )); then
        batch_record "PREBUILD_BEGIN cargo test --locked --test it_web --no-run"
        if ! cargo test --locked --test it_web --no-run >"$LOG_DIR/prebuild.log" 2>&1; then
            batch_record "PREBUILD_FAILED(见 $LOG_DIR/prebuild.log)——批终止,未启动任何 provider"
            die "预构建失败,批终止(未启动任何 provider)"
        fi
        batch_record "PREBUILD_OK"
    fi

    declare -A PROVIDER_EXIT_CODES=()
    declare -A PROVIDER_SUBTREE_FILES=()
    local overall=0

    # 阶段一:capture/full 串行(零并行——HOME skills 写面与固定 root recipe)。
    for p in $PHASE1_PROVIDERS; do
        if list_contains "$CAPTURE_ORDERED" "$p"; then
            run_provider "$p" capture_plan_snapshot phase1 || overall=1
        else
            run_provider "$p" full_chain phase1 || overall=1
        fi
        drain_subtree_snapshot "$p" "${PROVIDER_SUBTREE_FILES[$p]:-}"
    done

    # 阶段屏障:咨询级兜底 + 资源红线(触发→phase2 降级串行)。
    if [[ -n "${PHASE1_PROVIDERS:-}" ]]; then
        advisory_lingering_cli_check "$PHASE1_PROVIDERS"
        if [[ -e "$LOG_DIR/REDLINE" ]]; then
            batch_record "BARRIER_REDLINE 资源红线已触发——phase2 降级串行(候选值,金丝雀后校准)"
            PHASE2_DESC=serial
        fi
    fi

    # 阶段二:resume 并行(resume_parallel)或逐家串行(serial/降级)。
    if [[ "$PHASE2_DESC" == parallel && -n "$PHASE2_PROVIDERS" ]]; then
        local -A pids=()
        local -A wd_pids=()
        local pid short log state started exit_code
        started=$(date +%s)
        batch_record "PHASE2_PARALLEL_BEGIN providers=[$PHASE2_PROVIDERS]"
        for p in $PHASE2_PROVIDERS; do
            short=$p
            log=$LOG_DIR/$short.log
            state=$LOG_DIR/$short.watchdog.state
            env -u LIVE_MATRIX_SNAPSHOT_ID LC_GATEWAY_E2E=1 LIVE_MATRIX_RUN_MODE=resume_from_plan_snapshot \
                cargo test --locked --test it_web "lcg_live_${short}_five_stages_fresh_resume" \
                -- --ignored --nocapture --test-threads=1 >"$log" 2>&1 &
            pid=$!
            pids[$short]=$pid
            PROVIDER_EXIT_CODES[$short]=running
            batch_record "PROVIDER_START provider=$short phase=phase2-parallel mode=resume_from_plan_snapshot pid=$pid log=$log"
            "$WATCHDOG" --provider "$short" --pid "$pid" --log "$log" --state "$state" \
                --stale-after "$(stale_after_setting)" --interval "$(watch_interval_setting)" &
            wd_pids[$short]=$!
        done
        for p in $PHASE2_PROVIDERS; do
            short=$p
            exit_code=0
            set +e
            wait "${pids[$short]}"
            exit_code=$?
            set -e
            wait "${wd_pids[$short]}" 2>/dev/null || true
            PROVIDER_EXIT_CODES[$short]=$exit_code
            (( exit_code != 0 )) && overall=1
            batch_record "PROVIDER_EXIT provider=$short phase=phase2-parallel exit=$exit_code log=$LOG_DIR/$short.log"
            drain_subtree_snapshot "$short" "$LOG_DIR/$short.watchdog.state.subtree"
        done
        batch_record "PHASE2_PARALLEL_END duration=$(( $(date +%s) - started ))s"
    else
        for p in $PHASE2_PROVIDERS; do
            run_provider "$p" resume_from_plan_snapshot phase2-serial || overall=1
        done
    fi

    local summary=""
    for p in $CANONICAL_ORDER; do
        [[ -n "${PROVIDER_EXIT_CODES[$p]:-}" ]] || continue
        summary+=" $p=${PROVIDER_EXIT_CODES[$p]}"
    done
    if [[ -e "$LOG_DIR/REDLINE" ]]; then
        summary+=" REDLINE=triggered(下一批不发;按监控表回串行并立案)"
    fi
    batch_record "BATCH_END exits:${summary# }"
    echo "批收束:${summary# }"
    echo "批记录:$BATCH_STATE(资源样本:${BATCH_STATE}.resource)"
    exit "$overall"
}

main "$@"
