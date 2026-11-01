# 部署核查快照 2026-09-03T13:00:02Z (UTC)

## 1. 运行中服务器进程 (PID 401632, 监听 127.0.0.1:4317)
```
    PID                  STARTED     ELAPSED CMD
 401632 Thu Sep  3 14:15:24 2026    06:44:37 /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/target/debug/aria web --workspace /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate
lrwxrwxrwx 1 michaelche michaelche 0 Sep  3 20:59 /proc/401632/exe -> /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/target/debug/aria (deleted)
Device: 0,202	Inode: 222175111   Links: 0
Modify: 2026-09-03 09:51:43.352384209 +0800
```
- 运行二进制 inode=222175111, 构建于 2026-09-03 09:51:43 +0800, links=0 (deleted, 旧 inode 仅由进程持有)

## 2. 磁盘新二进制
```
Device: 0,202	Inode: 225117478   Links: 2
Modify: 2026-09-03 20:45:51.600683646 +0800
```
- 磁盘二进制 inode=225117478, 构建于 2026-09-03 20:45:51 +0800 (含 1e7a7247 接缝修复 + dc4d0d0d Evaluate route 修复)
- 服务器进程 14:15:25 启动 < 接缝修复 19:48:52 < Evaluate 修复 20:39:05 < 磁盘重建 20:45:51 ⇒ 两修复均未加载

## 3. 行为佐证 (curl 复现, 与修复前 v1 报告逐字节同错)
```
$ curl -s -X POST http://127.0.0.1:4317/api/projects/project_0001/issues/issue_0111/work-item-plans/issue_work_item_plan_0001/coding-attempts -H "content-type: application/json" -d "{}"
{"code":"coding_group_attempt_incomplete","message":"existing group coding attempt is only partially initialized or inconsistent","details":{}}
```
- 修复前根因链即此 400: journal_matches_request 全等校验败于 admission_kind/worktree_path → coding_group_attempt_incomplete

## 4. 本次重跑 (阶段A 第1跑, 环境类失败)
```
{
  "project_id": "project_0001",
  "issue_id": "issue_0111",
  "plan_id": "issue_work_item_plan_0001",
  "repository_id": "repository_0001",
  "provider": "codex",
  "attempt_id": null,
  "outDir": "/tmp/aria-stage35-20260903/amendment-rerun/coding-codex-preflight-failed-20260903125638535",
  "startedAt": "2026-09-03T12:56:38.535Z",
  "finishedAt": "2026-09-03T12:56:38.596Z",
  "elapsedSec": 0.061,
  "stageTimeline": [],
  "gates": [],
  "permissions": [],
  "choices": [],
  "review_results": [],
  "worktree": {
    "branch_name": null,
    "base_branch": null,
    "worktree_path": null,
    "head_commit": null,
    "push_status": null,
    "review_request_url": null
  },
  "usage": {
    "usage_unavailable": true
  },
  "timeline_nodes": [],
  "failureClass": "setup_or_preflight_error",
  "error": "create group coding attempt HTTP 400: existing group coding attempt is only partially initialized or inconsistent",
  "completed": false
}```
