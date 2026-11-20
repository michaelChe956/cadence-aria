#!/usr/bin/env bash
# 仓库结构核验（只读）：core.bare / 工作树位置 / main 与远端一致性。
#
# 规则来源：cadence/project-rules/repo-structure-policy.md
# 用法：
#   scripts/verify-repo-structure.sh            # 三项检查，失败即非零退出
#   scripts/verify-repo-structure.sh --repair-bare   # 仅在 core.bare 被改坏时复原为 true
#
# 退出码：0 全部通过；1 存在违规。
set -uo pipefail

repair_bare=0
[ "${1:-}" = "--repair-bare" ] && repair_bare=1

fail=0
note() { printf '[verify-repo-structure] %s\n' "$*"; }
bad() { printf '[verify-repo-structure] ✗ %s\n' "$*" >&2; fail=1; }

# --- 定位仓库根与共享 gitdir ---
common_dir=$(git rev-parse --git-common-dir 2>/dev/null) || {
  note "不在 git 仓库内，跳过。"
  exit 0
}
common_abs=$(cd "$common_dir" && pwd)
repo_root=$(cd "$common_dir/.." && pwd)
top=$(git rev-parse --show-toplevel 2>/dev/null || true)

# --- 检查 1：core.bare 恒为 true ---
bare=$(git config --get core.bare 2>/dev/null || true)
if [ "$bare" != "true" ]; then
  if [ "$repair_bare" = "1" ]; then
    git config core.bare true
    note "core.bare 由 '${bare}' 复原为 true。"
  else
    bad "core.bare 必须是 true，当前为 '${bare}'。根目录会被 git 当成工作区。"
    bad "复原命令：git config core.bare true（规则见 cadence/project-rules/repo-structure-policy.md）"
  fi
fi

# --- 检查 2：不得在仓库根目录工作区操作 ---
if [ -n "$top" ] && [ "$top" = "$repo_root" ]; then
  bad "当前工作区就是仓库根目录（$repo_root）；所有内容必须经 .worktrees/ 下的工作树实现。"
elif [ -z "$top" ]; then
  note "当前不在任何工作树内（bare 布局的共享 gitdir 目录），符合预期。"
else
  note "工作树位置 OK：$top"
fi

# --- 检查 3：main 不得有本地领先提交（PR 之外的落地） ---
if git show-ref --verify --quiet refs/heads/main; then
  local_main=$(git rev-parse refs/heads/main)
  if git show-ref --verify --quiet refs/remotes/origin/main; then
    origin_main=$(git rev-parse refs/remotes/origin/main)
    ahead=$(git rev-list --count refs/remotes/origin/main..refs/heads/main)
    if [ "$ahead" -gt 0 ]; then
      bad "本地 main 比 origin/main 领先 $ahead 个提交；main 只能经 PR 合并落地。"
    else
      note "main 与 origin/main 一致（$local_main）。"
    fi
  else
    note "缺 origin/main 引用，跳过 main 一致性核验。"
  fi
fi

# --- 检查 4：当前分支不是 main ---
# 只在工作树内判定：bare 布局下根目录的 HEAD 指向 refs/heads/main，
# 但那里不是工作区、也无法提交，不属于「在 main 上提交」的情形。
if [ -n "$top" ]; then
  branch=$(git symbolic-ref --short HEAD 2>/dev/null || true)
  if [ "$branch" = "main" ]; then
    bad "当前分支是 main；提交请走功能分支 + PR。"
  else
    note "当前分支：${branch:-（detached / 无）}"
  fi
else
  note "当前不在工作树内，跳过分支检查。"
fi

if [ "$fail" -ne 0 ]; then
  note "核验未通过。"
  exit 1
fi
note "核验通过（共享 gitdir：$common_abs）。"
exit 0
