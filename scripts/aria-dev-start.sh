#!/usr/bin/env bash
# 开发态双服务（后端 cargo watch + 前端 vite dev）的**唯一入口**脚本。
#
# 为什么有这份脚本（故障史，勿删）：
#   - 2026-10-30：绕过 scripts/aria-web-start.sh 手工启动发布实例 ⇒ 脚本导出的
#     ARIA_API_TRUSTED_ORIGINS 没进进程环境 ⇒ owner 一访问就报
#     `request origin not allowed`（端口在听但服务不可用）。
#   - 2026-11-12：开发态同款故障——用 Skill 里的裸命令起服务（后端 4317 + 前端
#     5173），未配受信 origin；vite 代理把浏览器的 `Host: 127.0.0.1:5173` 原样
#     转给后端 ⇒ 后端 Host 校验拒绝 ⇒ 所有受守卫的 /api/** 与 /ws/** 全部 403
#     forbidden_host（整页「PROVIDER 状态不可用」）；而 GET /api/health 在 Host
#     校验之前放行，健康检查完全看不到故障，报错离原因十万八千里。
#
# 因此本脚本把「来源校验必须放行前端 dev origin」做成**硬性门禁**（不是尽力而为）：
#   ① 导出 ARIA_API_TRUSTED_ORIGINS：后端+前端端口的 127.0.0.1 / localhost /
#      hostname 全套 origin；外部已设的值**追加**（不覆盖）。
#   ② 导出 NO_PROXY/no_proxy；后台起后端（flock /tmp/aria-cargo.lock 串行化
#      编译）与前端，等两者就绪。
#   ③ 门禁探测：用「浏览器同款 Host + Origin」经前端代理打一个**受守卫**的只读
#      端点；非 200 即失败退出并打印可复制的修复命令。
#      ★ 禁止用 /api/health 代替探测：它在 Host 校验之前放行，探不出此故障。
#
# 用法：
#   bash scripts/aria-dev-start.sh            # 启动双服务并做门禁探测
#   bash scripts/aria-dev-start.sh --check    # 只探测已运行的服务，不启动
#
# 环境变量（换端口验证时用，如 ARIA_PORT=4327 ARIA_WEB_PORT=5273，
# 避免与共享实例 4317/5173 冲突）：
#   ARIA_PORT                 后端端口（默认 4317）
#   ARIA_WEB_PORT             前端 dev server 端口（默认 5173）
#   ARIA_DEV_PROBE_PATH       门禁探测端点（默认 /api/image-create/sessions；
#                             必须是受守卫的 /api/** 只读端点，不得用 /api/health）
#   ARIA_DEV_BACKEND_LOG      后端日志（默认 /tmp/aria-dev-backend-<port>.log）
#   ARIA_DEV_FRONTEND_LOG     前端日志（默认 /tmp/aria-dev-frontend-<port>.log）
#   ARIA_DEV_WAIT_BACKEND_SECS / ARIA_DEV_WAIT_FRONTEND_SECS  就绪等待超时
#   ARIA_API_TRUSTED_ORIGINS  追加到推导白名单之后（逗号分隔，不覆盖）
set -euo pipefail

# ── 仓库根：从脚本自身位置推导（scripts/ 的上一级），任意 worktree 可用 ──────
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PORT="${ARIA_PORT:-4317}"
WEB_PORT="${ARIA_WEB_PORT:-5173}"
PROBE_PATH="${ARIA_DEV_PROBE_PATH:-/api/image-create/sessions}"
BACKEND_LOG="${ARIA_DEV_BACKEND_LOG:-/tmp/aria-dev-backend-${PORT}.log}"
FRONTEND_LOG="${ARIA_DEV_FRONTEND_LOG:-/tmp/aria-dev-frontend-${WEB_PORT}.log}"
WAIT_BACKEND_SECS="${ARIA_DEV_WAIT_BACKEND_SECS:-300}"
WAIT_FRONTEND_SECS="${ARIA_DEV_WAIT_FRONTEND_SECS:-120}"
CARGO_LOCK_FILE="/tmp/aria-cargo.lock"
ORIGIN_ENV_NAME="ARIA_API_TRUSTED_ORIGINS"

# 本机 hostname 命令可能把名字写到 stderr ⇒ 优先用内核文件取（同 aria-web-start.sh）
HOSTN="$(cat /proc/sys/kernel/hostname 2>/dev/null || hostname 2>/dev/null || echo localhost)"

# ── ① 受信 origin 白名单：后端+前端端口 × {127.0.0.1, localhost, hostname} ────
# 外部已设的 ARIA_API_TRUSTED_ORIGINS 追加在推导值之后（不覆盖）。
DEV_ORIGINS=""
for port in "$PORT" "$WEB_PORT"; do
  for host in 127.0.0.1 localhost "$HOSTN"; do
    DEV_ORIGINS="${DEV_ORIGINS:+$DEV_ORIGINS,}http://${host}:${port}"
  done
done
export ARIA_API_TRUSTED_ORIGINS="${ARIA_API_TRUSTED_ORIGINS:+$ARIA_API_TRUSTED_ORIGINS,}$DEV_ORIGINS"
export NO_PROXY="127.0.0.1,localhost,${HOSTN}${NO_PROXY:+,$NO_PROXY}"
export no_proxy="$NO_PROXY"
# vite dev proxy 的后端目标（web/dev-server-proxy.ts 读 ARIA_E2E_PORT，默认 4317）：
# 本脚本启动的前端必须指向本脚本的后端端口，否则代理打到别的实例。
export ARIA_E2E_PORT="$PORT"

# ── 小工具 ───────────────────────────────────────────────────────────────────
port_listening() {
  ss -ltnH "sport = :$1" 2>/dev/null | grep -q .
}

health_backend() {
  curl --noproxy '*' -sf -m 5 "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1
}

frontend_up() {
  curl --noproxy '*' -sf -m 5 -o /dev/null "http://127.0.0.1:${WEB_PORT}/" 2>/dev/null
}

# 等 $1 描述的条件就绪，最多 $2 秒、每 2s 轮询一次 $3.. 的命令；超时返回 1。
wait_until() {
  local name="$1" timeout_secs="$2" waited=0
  shift 2
  while ! "$@"; do
    if [ "$waited" -ge "$timeout_secs" ]; then
      echo "✗ 等待 ${name} 超时（${timeout_secs}s）" >&2
      return 1
    fi
    sleep 2
    waited=$((waited + 2))
  done
}

# 门禁探测：浏览器同款 Host+Origin，经前端代理打**受守卫**只读端点。
# 输出 HTTP 状态码；响应体写入 $1。
probe_guarded_status() {
  local body_file="$1"
  curl --noproxy '*' -sS -o "$body_file" -w '%{http_code}' -m 15 \
    -H "Host: 127.0.0.1:${WEB_PORT}" \
    -H "Origin: http://127.0.0.1:${WEB_PORT}" \
    "http://127.0.0.1:${WEB_PORT}${PROBE_PATH}" 2>/dev/null || true
}

# 直连后端（后端自身 Host，无 Origin，GET 只读）——用于区分「后端坏了」与「来源校验拒绝」。
probe_direct_status() {
  curl --noproxy '*' -sS -o /dev/null -w '%{http_code}' -m 15 \
    -H "Host: 127.0.0.1:${PORT}" \
    "http://127.0.0.1:${PORT}${PROBE_PATH}" 2>/dev/null || true
}

# 门禁失败时打印确切原因与可直接复制执行的修复。
gate_fail_hint() {
  local err_code="$1" direct="$2"
  echo "" >&2
  echo "✗ 门禁探测失败（HTTP 403，错误码 ${err_code}）" >&2
  echo "  原因：前端 dev origin 未在后端受信列表——vite 代理把浏览器的 Host/Origin 原样" >&2
  echo "        转给后端，后端来源校验（src/web/api_auth.rs）拒绝了所有受守卫的 /api/**" >&2
  echo "        与 /ws/** 请求（后端直连探测：${direct}）。" >&2
  echo "  注意：/api/health 在 Host 校验之前放行，健康检查看不到此故障；" >&2
  echo "        必须探测受守卫端点（本脚本已用 ${PROBE_PATH}）。" >&2
  echo "  修复（可直接复制执行）：" >&2
  echo "    export ${ORIGIN_ENV_NAME}=\"http://127.0.0.1:${WEB_PORT},http://localhost:${WEB_PORT},http://${HOSTN}:${WEB_PORT}\"" >&2
  echo "    # 然后重启后端；或直接用本脚本重启双服务（自动带上全套受信 origin 并做门禁探测）：" >&2
  echo "    bash \"${REPO_ROOT}/scripts/aria-dev-start.sh\"" >&2
  if port_listening "$PORT" && [ "$started_backend_pid" -eq 0 ]; then
    echo "  端口 ${PORT} 上是外部启动的旧后端实例：本脚本不会代杀非自己启动的进程，" >&2
    echo "  请先人工停止它（ss -ltnp | grep :${PORT} 找 pid），再重跑本脚本。" >&2
  fi
}

# 硬门禁：受守卫端点 + 浏览器同款 Host/Origin 必须 200，否则打印修复并以非零码失败。
gate_probe() {
  local body_file status err_code direct
  body_file="$(mktemp /tmp/aria-dev-probe.XXXXXX.json)"
  status="$(probe_guarded_status "$body_file")"
  case "$status" in
    200)
      rm -f "$body_file"
      echo "✓ 门禁探测通过（HTTP 200：浏览器同款 Host+Origin 经前端代理访问受守卫端点 ${PROBE_PATH}）"
      return 0
      ;;
    403)
      err_code="$(grep -o '"code"[[:space:]]*:[[:space:]]*"[^"]*"' "$body_file" 2>/dev/null | head -1 | sed 's/.*"\([^"]*\)"$/\1/' || true)"
      rm -f "$body_file"
      [ -n "$err_code" ] || err_code="forbidden_host/forbidden_origin（body 未解析出 code）"
      direct="$(probe_direct_status)"
      case "$direct" in ""|000) direct="连接失败";; esac
      gate_fail_hint "$err_code" "$direct"
      return 1
      ;;
    ""|000)
      rm -f "$body_file"
      echo "" >&2
      echo "✗ 门禁探测失败：前端 ${WEB_PORT} 不可达（连接失败）。前端未启动或端口不对。" >&2
      echo "  修复：先启动双服务：bash \"${REPO_ROOT}/scripts/aria-dev-start.sh\"" >&2
      return 1
      ;;
    *)
      cp "$body_file" /tmp/aria-dev-probe-last.json 2>/dev/null || true
      rm -f "$body_file"
      echo "" >&2
      echo "✗ 门禁探测失败：意外状态码 HTTP ${status}（响应体留存 /tmp/aria-dev-probe-last.json）。" >&2
      echo "  若 404/405：ARIA_DEV_PROBE_PATH 指向的端点不可用，请换一个受守卫的只读端点" >&2
      echo "  （不得用 /api/health，它在 Host 校验之前放行，探不出来源校验故障）。" >&2
      echo "  若 500/502：后端未就绪或 vite 代理异常，查看 ${BACKEND_LOG} 与 ${FRONTEND_LOG}。" >&2
      return 1
      ;;
  esac
}

report_ready() {
  echo ""
  echo "开发服务就绪："
  echo "  前端地址    http://127.0.0.1:${WEB_PORT}"
  echo "  后端地址    http://127.0.0.1:${PORT}"
  echo "  可访问页面  http://127.0.0.1:${WEB_PORT}/workbench（建议从这里开始）"
  echo "  后端日志    ${BACKEND_LOG}"
  echo "  前端日志    ${FRONTEND_LOG}"
  echo "  ${ORIGIN_ENV_NAME}（已注入后端进程）= ${ARIA_API_TRUSTED_ORIGINS}"
}

# 本脚本启动的进程（用于失败清理；外部实例一律不杀）
started_backend_pid=0
started_frontend_pgid=0

cleanup_started() {
  if [ "$started_frontend_pgid" -ne 0 ]; then
    kill -- "-$started_frontend_pgid" 2>/dev/null || true
  fi
  if [ "$started_backend_pid" -ne 0 ]; then
    kill -- "-$started_backend_pid" 2>/dev/null || true
  fi
}

# ── 自检模式：只查不启 ────────────────────────────────────────────────────────
if [ "${1:-}" = "--check" ]; then
  echo "自检（ARIA_PORT=${PORT} ARIA_WEB_PORT=${WEB_PORT} 探测端点=${PROBE_PATH}）："
  fail=0
  if health_backend; then
    echo "✓ 后端 /api/health 正常（http://127.0.0.1:${PORT}/api/health）"
  else
    echo "✗ 后端 /api/health 不可达（http://127.0.0.1:${PORT}/api/health）"
    fail=1
  fi
  if frontend_up; then
    echo "✓ 前端 dev server 正常（http://127.0.0.1:${WEB_PORT}/）"
  else
    echo "✗ 前端 dev server 不可达（http://127.0.0.1:${WEB_PORT}/）"
    fail=1
  fi
  # 门禁探测是硬性项：403 即失败退出并给修复
  gate_probe || fail=1
  if [ "$fail" -ne 0 ]; then
    exit 1
  fi
  report_ready
  exit 0
elif [ "${1:-}" != "" ]; then
  echo "用法：bash scripts/aria-dev-start.sh [--check]" >&2
  exit 2
fi

# ── 启动模式 ─────────────────────────────────────────────────────────────────
echo "启动开发服务（仓库根：${REPO_ROOT}）："
echo "  PORT=${PORT}  WEB_PORT=${WEB_PORT}"
echo "  ${ORIGIN_ENV_NAME} = ${ARIA_API_TRUSTED_ORIGINS}"

# 工具与前置产物检查（缺了就停，不擅自安装/构建）
command -v cargo >/dev/null || { echo "✗ 缺少 cargo" >&2; exit 1; }
cargo watch --version >/dev/null 2>&1 || { echo "✗ 缺少 cargo-watch（cargo install cargo-watch）" >&2; exit 1; }
command -v pnpm >/dev/null || { echo "✗ 缺少 pnpm" >&2; exit 1; }
[ -f "${REPO_ROOT}/web/dist/index.html" ] || {
  echo "✗ web/dist 缺失：后端 build.rs 会在编译期 panic。先执行 pnpm -C web install && pnpm -C web build" >&2
  exit 1
}
[ -d "${REPO_ROOT}/web/node_modules" ] || {
  echo "✗ web/node_modules 缺失：先执行 pnpm -C web install（需用户确认）" >&2
  exit 1
}

# ── ② 后端：已监听且健康则复用；否则 flock 串行预编译 + 后台 cargo watch ──────
if port_listening "$PORT"; then
  if health_backend; then
    echo "◦ 后端端口 ${PORT} 已有健康实例，复用（若其缺受信 origin，随后的门禁探测会暴露）"
  else
    echo "✗ 后端端口 ${PORT} 被占用且不是健康的 aria 后端；本脚本不代杀未知进程。" >&2
    echo "  请先确认来源：ss -ltnp | grep :${PORT}" >&2
    exit 1
  fi
else
  echo "◦ 预编译后端（flock ${CARGO_LOCK_FILE} 串行化，避免并发编译互踩）…"
  ( flock "$CARGO_LOCK_FILE" cargo build --locked ) >&2 || {
    echo "✗ 后端预编译失败（见上方 cargo 输出）" >&2
    exit 1
  }
  : >"$BACKEND_LOG"
  # setsid：独立进程组，失败清理可整组收割；nohup：脚本退出后长期运行
  setsid nohup cargo watch -w src -w Cargo.toml -w Cargo.lock \
    -x "run --locked -- web --workspace . --host 127.0.0.1 --port ${PORT}" \
    >>"$BACKEND_LOG" 2>&1 &
  started_backend_pid=$!
  if ! wait_until "后端 /api/health 就绪（日志 ${BACKEND_LOG}）" "$WAIT_BACKEND_SECS" health_backend; then
    tail -n 40 "$BACKEND_LOG" >&2 || true
    cleanup_started
    exit 1
  fi
  echo "✓ 后端就绪（http://127.0.0.1:${PORT}/api/health，日志 ${BACKEND_LOG}）"
fi

# ── ③ 前端：已可访问则复用；否则后台 pnpm dev ────────────────────────────────
if frontend_up; then
  echo "◦ 前端端口 ${WEB_PORT} 已有可访问实例，复用"
elif port_listening "$WEB_PORT"; then
  echo "✗ 前端端口 ${WEB_PORT} 被占用且不是可访问的 HTTP 服务；本脚本不代杀未知进程。" >&2
  echo "  请先确认来源：ss -ltnp | grep :${WEB_PORT}" >&2
  cleanup_started
  exit 1
else
  : >"$FRONTEND_LOG"
  (
    cd "${REPO_ROOT}/web"
    exec setsid nohup pnpm dev --port "${WEB_PORT}" >>"$FRONTEND_LOG" 2>&1
  ) &
  started_frontend_pgid=$!
  if ! wait_until "前端 dev server 就绪（日志 ${FRONTEND_LOG}）" "$WAIT_FRONTEND_SECS" frontend_up; then
    tail -n 40 "$FRONTEND_LOG" >&2 || true
    cleanup_started
    exit 1
  fi
  echo "✓ 前端就绪（http://127.0.0.1:${WEB_PORT}/，日志 ${FRONTEND_LOG}）"
fi

# ── ④ 硬门禁：受守卫端点 + 浏览器同款 Host/Origin 必须 200 ────────────────────
gate_probe || exit 1

report_ready
