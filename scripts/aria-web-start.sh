#!/usr/bin/env bash
# 4317 共享实例的**唯一权威启动入口**（仓内源；/tmp/aria-deploy/start.sh 是它的安装副本）。
#
# 为什么有这份脚本：2026-10-30 真实故障——绕过脚本手工
# `script -qfc ".../aria web --port 4317"` 直起二进制 ⇒ 脚本导出的
# ARIA_API_TRUSTED_ORIGINS 没进进程环境 ⇒ owner 一访问就报
# `[request origin not allowed]`（端口在听但服务不可用）。
#
# 本脚本负责三件事，任何一件都不能丢：
#   ① 导出 ARIA_API_TRUSTED_ORIGINS（远端域名 + 本机各 dev 端口白名单）
#   ② 停旧实例（同 workspace / 同端口）
#   ③ exec script -qfc 分配 TTY（T1 鉴权 bootstrap token 靠 TTY 一次性交付）
#
# 用法：
#   # 1) 安装到权威路径（源在仓里，/tmp 只是安装副本）
#   install -m755 scripts/aria-web-start.sh /tmp/aria-deploy/start.sh
#   # 2) 启动（用 harness 具名长驻服务 + pty 托管，勿用 nohup/setsid 手工后台）
#   bash /tmp/aria-deploy/start.sh
#   # 3) 启动后 6 项自检
#   bash /tmp/aria-deploy/start.sh --check
#
# 环境变量：
#   ARIA_BIN          二进制路径（默认 /tmp/deploy-target/release/aria；
#                     开发期可用当前 worktree 的 target/debug/aria）
#   ARIA_WORKSPACE    工作区（默认 /tmp/aria-verify）
#   ARIA_PORT         端口（默认 4317）
#   ARIA_WEB_LOG      会话日志（默认 /tmp/aria-deploy/session.log）
#   ARIA_API_TRUSTED_ORIGINS  追加/覆盖 origin 白名单（默认按 hostname 推导）
#   ARIA_FINGERPRINTS 指纹字符串（每行一个；--check 时用 grep -a 扫描）
#   ARIA_ROLLBACK     回滚件路径（--check 时与新件对照）
set -euo pipefail

WS="${ARIA_WORKSPACE:-/tmp/aria-verify}"
PORT="${ARIA_PORT:-4317}"
LOG="${ARIA_WEB_LOG:-/tmp/aria-deploy/session.log}"
BIN="${ARIA_BIN:-/tmp/deploy-target/release/aria}"
ORIGIN_ENV_NAME="ARIA_API_TRUSTED_ORIGINS"

mkdir -p "$(dirname "$LOG")"

# ── ① origin 白名单（唯一真源：改本文件，不要改启动命令行）────────────────────
# ★ 本机 hostname 命令把名字写到 stderr ⇒ 2>/dev/null 会丢它 ⇒ 用内核文件取
HOSTN="$(cat /proc/sys/kernel/hostname 2>/dev/null || hostname 2>/dev/null || echo localhost)"
DEFAULT_ORIGINS="http://127.0.0.1:${PORT},http://localhost:${PORT},http://${HOSTN}:${PORT}"
# 开发期前端端口（vite 直供 web/src 源码）：后端 4317 + 前端 5173 必须同在白名单，
# 否则 vite 代理过来的 /api 请求会被来源校验拒（forbidden_origin / forbidden_host）。
for dev_port in 5173 4319; do
  DEFAULT_ORIGINS="${DEFAULT_ORIGINS},http://127.0.0.1:${dev_port},http://localhost:${dev_port},http://${HOSTN}:${dev_port}"
done
# owner 2026-11-06 新增：前端域名的独立部署 + 4317 的公网反代域
DEFAULT_ORIGINS="${DEFAULT_ORIGINS},https://arch-code.mikechelazyc.heiyu.space"
DEFAULT_ORIGINS="${DEFAULT_ORIGINS},https://arch-code-fe.mikechelazyc.heiyu.space"
export ARIA_API_TRUSTED_ORIGINS="${ARIA_API_TRUSTED_ORIGINS:-$DEFAULT_ORIGINS}"
export NO_PROXY="127.0.0.1,localhost,${HOSTN}"
export no_proxy="$NO_PROXY"

# ── 自检模式：只查不启 ────────────────────────────────────────────────────────
check_instance() {
  local ok=0
  local pid
  pid="$(ss -ltnp 2>/dev/null | awk -v p=":${PORT}\$" '$4 ~ p {print $0}' | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2 || true)"

  echo "== 1) 端口在听 =="
  if ss -ltnp 2>/dev/null | grep -q ":${PORT} "; then
    echo "   OK  ${PORT} 在听"
  else
    echo "   FAIL ${PORT} 没在听"; ok=1
  fi

  echo "== 2) 全站可用 =="
  local root_code health_code
  root_code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/" || echo 000)"
  health_code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/api/health" || echo 000)"
  [ "$root_code" = "200" ] && echo "   OK  GET / → 200" || { echo "   FAIL GET / → ${root_code}"; ok=1; }
  [ "$health_code" = "200" ] && echo "   OK  GET /api/health → 200" || { echo "   FAIL GET /api/health → ${health_code}"; ok=1; }

  echo "== 3) 启动自证（日志） =="
  if [ -f "$LOG" ] && grep -q "aria web listening on http://127.0.0.1:${PORT}" "$LOG"; then
    echo "   OK  日志含 listening"
  else
    echo "   FAIL 日志缺 listening（${LOG}）"; ok=1
  fi
  if [ -f "$LOG" ] && grep -q "governance_complete=" "$LOG"; then
    local gov
    gov="$(grep -o "governance_complete=[a-z]*" "$LOG" | tail -1)"
    if [ "$gov" = "governance_complete=true" ]; then
      echo "   OK  日志含 governance_complete=true"
    else
      echo "   WARN 日志含 ${gov}（工作区存在悬空引用等治理遗留项，与启动本身无关）"
    fi
  else
    echo "   FAIL 日志缺 governance_complete=（启动自证没跑）"; ok=1
  fi

  echo "== 4) 环境变量在位 =="
  if [ -n "${pid}" ] && tr '\0' '\n' < "/proc/${pid}/environ" 2>/dev/null | grep -q "^${ORIGIN_ENV_NAME}=..*"; then
    echo "   OK  ${ORIGIN_ENV_NAME} 非空（pid=${pid}）"
  else
    echo "   FAIL ${ORIGIN_ENV_NAME} 缺失/为空（pid=${pid:-?}）⇒ owner 访问会报 request origin not allowed"; ok=1
  fi

  echo "== 5) 跑的是新件 =="
  if [ -n "${pid}" ] && [ -e "/proc/${pid}/exe" ]; then
    local running
    running="$(readlink -f "/proc/${pid}/exe" 2>/dev/null || echo unknown)"
    echo "   运行中: ${running}"
    if [ "$(readlink -f "$BIN" 2>/dev/null || echo none)" = "$running" ]; then
      echo "   OK  跑的就是 ${BIN}"
    else
      echo "   WARN 跑的件与 ARIA_BIN 不同（期望 $(readlink -f "$BIN" 2>/dev/null || echo "$BIN")）"; ok=1
    fi
  else
    echo "   FAIL 找不到运行中的 pid"; ok=1
  fi

  echo "== 6) 新代码指纹在件里（grep -a，不用 strings） =="
  if [ -n "${ARIA_FINGERPRINTS:-}" ]; then
    while IFS= read -r needle; do
      [ -z "$needle" ] && continue
      local hit_new hit_old
      hit_new="$(grep -a -c -- "$needle" "$BIN" 2>/dev/null | head -1)" || hit_new=0
      [ -z "${hit_new}" ] && hit_new=0
      if [ -n "${ARIA_ROLLBACK:-}" ] && [ -f "${ARIA_ROLLBACK}" ]; then
        hit_old="$(grep -a -c -- "$needle" "${ARIA_ROLLBACK}" 2>/dev/null | head -1)" || hit_old=0
        [ -z "${hit_old}" ] && hit_old=0
      else
        hit_old="(无回滚件可比对)"
      fi
      echo "   指纹「${needle}」新件命中=${hit_new} 回滚件命中=${hit_old}"
    done <<< "${ARIA_FINGERPRINTS}"
  else
    echo "   SKIP 未提供 ARIA_FINGERPRINTS（本次改动的稳定字符串，每行一个）"
  fi

  echo
  if [ "$ok" = "0" ]; then
    echo "自检结果：全部通过"
  else
    echo "自检结果：有 FAIL 项 ⇒ 部署未完成，不得声称已重发"
  fi
  return "$ok"
}

if [ "${1:-}" = "--check" ]; then
  check_instance
  exit $?
fi

if [ ! -x "$BIN" ]; then
  echo "启动失败：二进制不存在或不可执行：${BIN}" >&2
  echo "构建顺序不可颠倒：pnpm -C web build → CARGO_TARGET_DIR=/tmp/deploy-target cargo build --release --locked" >&2
  exit 1
fi

echo "启动参数："
echo "  BIN   = ${BIN}"
echo "  WS    = ${WS}"
echo "  PORT  = ${PORT}"
echo "  LOG   = ${LOG}"
echo "  ${ORIGIN_ENV_NAME} = ${ARIA_API_TRUSTED_ORIGINS}"

# ── ② 停旧实例（只停本实例，不误伤其它进程） ────────────────────────────────
pkill -f "aria web --workspace $WS" 2>/dev/null || true
pkill -f "aria web --host 127.0.0.1 --port ${PORT}" 2>/dev/null || true
sleep 1

# ── ③ TTY 启动 ───────────────────────────────────────────────────────────────
exec script -qfc "$BIN web --workspace $WS --host 127.0.0.1 --port ${PORT}" "$LOG"