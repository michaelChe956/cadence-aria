#!/usr/bin/env bash
# target 目录清理工具。三档：--stale / --all / --dry-run
# --stale   : 清 incremental 全部缓存；deps 保留（仅 incremental 是纯累积缓存）
# --all     : 等价 cargo clean（全删 target）
# --dry-run : 只报告将释放多少空间，不删除
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

mode="${1:---dry-run}"

size_of() { du -sh "$1" 2>/dev/null | awk '{print $1}'; }

case "$mode" in
  --all)
    echo "[clean-target] cargo clean（全删 target）..."
    cargo clean
    echo "[clean-target] 完成。"
    ;;
  --stale)
    inc="target/debug/incremental"
    if [ -d "$inc" ]; then
      echo "[clean-target] 删除增量编译缓存：$inc（当前 $(size_of "$inc")）"
      rm -rf "$inc"
    else
      echo "[clean-target] 无 incremental 缓存，跳过。"
    fi
    echo "[clean-target] 完成。deps 当前 $(size_of target/debug/deps 2>/dev/null)。"
    echo "[clean-target] 提示：deps 陈旧产物可用 'cargo clean' 整体回收后重建。"
    ;;
  --dry-run)
    echo "[clean-target] 干跑（不删除）："
    echo "  target 总计       : $(size_of target)"
    echo "  deps              : $(size_of target/debug/deps)"
    echo "  incremental(可清) : $(size_of target/debug/incremental)"
    echo "  --stale 将释放 incremental 部分；--all 释放全部 target。"
    ;;
  *)
    echo "用法: $0 [--stale|--all|--dry-run]" >&2
    exit 2
    ;;
esac
