# Spike 4：50 个合成子仓的 CodeGraph 预算测量

- **执行日期**：2026-08-08
- **CodeGraph**：`1.5.0`（`@colbymchenry/codegraph@1.5.0`；Node `v24.17.0`）
- **执行地点**：`/tmp/spike-50repos-20260808-203106/parent/`；该 `parent/` 故意不是 Git 仓，内含 50 个独立 Git 子仓。
- **清理说明**：为审阅和复现暂保留 `/tmp/spike-50repos-20260808-203106/`；没有在 worktree 写入 fixture、数据库或脚本。

## 1. Fixture 生成脚本（可复现）

本次实际运行的脚本保留在 `/tmp/spike-50repos-20260808-203106/generate-fixture.sh`。以下为完整内容：

```bash
#!/usr/bin/env bash
# Usage: ./generate-fixture.sh /absolute/path/to/parent
set -euo pipefail
parent=$1
mkdir -p "$parent"
for n in $(seq -w 1 50); do
  repo="$parent/service-$n"
  mkdir -p "$repo/src/main/java/com/spike/service$n"
  git -C "$repo" init -q
  git -C "$repo" config user.email spike@example.invalid
  git -C "$repo" config user.name 'CodeGraph Spike'
  if [ "$n" = 05 ] || [ "$n" = 20 ] || [ "$n" = 35 ]; then
    cat > "$repo/package.json" <<EOF
{"name":"service-$n","private":true,"version":"1.0.0"}
EOF
    mkdir -p "$repo/src"
    for i in $(seq -w 1 28); do
      cat > "$repo/src/Feature$i.ts" <<EOF
export class Feature${n}_${i} {
  private readonly label: string = 'service-$n';
  run(input: string): string { return this.label + ':' + input; }
}
export function createFeature${n}_${i}(input: string): string {
  return new Feature${n}_${i}().run(input);
}
EOF
    done
    for i in $(seq -w 1 3); do
      cat > "$repo/src/Widget$i.vue" <<EOF
<script setup lang="ts">
const service = 'service-$n';
const label = service + '-widget-$i';
</script>
<template><div>{{ label }}</div></template>
EOF
    done
  else
    for i in $(seq -w 1 28); do
      cat > "$repo/src/main/java/com/spike/service$n/Feature$i.java" <<EOF
package com.spike.service$n;

public class Feature$i {
    private final String service = "service-$n";
    private final int sequence = $((10#$i));
    public String execute(String request) {
        return service + ":" + sequence + ":" + request;
    }
    public int sequence() { return sequence; }
}
EOF
    done
  fi
  git -C "$repo" add .
  git -C "$repo" commit -qm 'synthetic fixture'
done
```

复现：

```sh
base=/tmp/spike-50repos-$(date +%Y%m%d-%H%M%S)
mkdir -p "$base"
# 将上述脚本保存为 "$base/generate-fixture.sh" 后：
chmod +x "$base/generate-fixture.sh"
"$base/generate-fixture.sh" "$base/parent"
cd "$base/parent"
codegraph init .
```

### 实际 fixture 规模

| 项目 | 实测值 |
| --- | ---: |
| 独立 Git 子仓 | 50 |
| Java 后端仓 | 47 |
| TypeScript/Vue 前端仓 | 3（`service-05`、`service-20`、`service-35`） |
| Java 文件 | 1,316 |
| TypeScript 文件 | 84 |
| Vue 文件 | 9 |
| 可索引源码总数 | 1,409 |
| fixture 磁盘占用（含 50 个 `.git`） | 18 MiB |

每仓有 28 个小源文件；前端仓另各有 `package.json` 与 3 个 `.vue` 文件。源文件量为“几十 KB 级/仓”的小型微服务模拟，而不是实际生产仓的依赖树规模。

## 2. 测量方法

实测前先执行：

```sh
codegraph --version       # 1.5.0
codegraph init --help     # 确认：codegraph init [options] [path]
```

首次索引：

```sh
cd /tmp/spike-50repos-20260808-203106/parent
codegraph init .
```

增量测量前，在 `service-01` 修改 3 个既有 Java 源文件并执行 Git commit：

```text
0e8edfa incremental fixture change
3 files changed, 4 insertions(+), 2 deletions(-)
```

随后：

```sh
codegraph sync .
```

本机没有 `/usr/bin/time` 或 `/bin/time`，故耗时由 Python `time.perf_counter()` 包围子进程测得；峰值内存同时记录：

1. 每 20 ms 对 CodeGraph 父进程执行 `ps -o rss=` 的采样峰值（会低估极短生命周期的子 worker）；
2. `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss` 的子进程累计峰值（更保守，但可能包含 CodeGraph 启动的短生命周期 worker）。

因此报告同时给出两个值，而不将较小的采样数字伪装成精确峰值。

## 3. 性能数据

### Index / sync

| 场景 | 命令 | CLI 内部处理时间 | 端到端墙钟时间 | 峰值 RSS（20 ms 父进程采样） | `ru_maxrss` 子进程峰值 | 结果 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 首次建立统一索引 | `codegraph init .` | 372 ms | **1.363 s** | 81,996 KiB（约 80.1 MiB） | 423,160 KiB（约 413.2 MiB） | 1,409 文件、9,668 nodes、8,445 edges |
| 3 文件 Git 提交后的增量 | `codegraph sync .` | 128 ms | **0.418 s** | 82,000 KiB（约 80.1 MiB） | 167,720 KiB（约 163.8 MiB） | `Synced 3 changed files`；Modified 3 / 21 nodes |

首次命令的真实 CLI 摘要：

```text
Indexed 1,409 files
9,668 nodes, 8,445 edges in 372ms
```

增量命令的真实 CLI 摘要：

```text
Synced 3 changed files
Modified: 3 — 21 nodes in 128ms
```

### DB 大小

| 时点 | `codegraph status` DB Size | `du -sh .codegraph` | 说明 |
| --- | ---: | ---: | --- |
| 首次 `init` 后 | 9.14 MB | 9.2 MiB | `.codegraph/codegraph.db` + 4 KiB `.gitignore` |
| 3 文件 `sync` 后 | 9.79 MB | 9.8 MiB | SQLite/WAL 的增长，未执行 vacuum |

索引目录形态：

```text
.codegraph/
├── .gitignore
└── codegraph.db
```

`status` 显示 `Backend: node:sqlite — built-in (full WAL)` 和 `Journal: wal`。

### 语言分布校验

首次 `codegraph status` 显示：Java 1,316、TypeScript 84、Vue 9。说明三种前端仓没有被 fixture 脚本或索引器意外遗漏。

## 4. 50 仓 inventory 的 prompt 预算

生成的模拟成员清单为 JSON，每项含 `id`、`alias`、`path`、`role`、`profile` 摘要（50 项，3 个前端角色、47 个后端角色）。实测文件：`/tmp/spike-50repos-20260808-203106/inventory.json`。

| 指标 | 实测值 |
| --- | ---: |
| JSON 字节数（`wc -c`） | 9,857 B |
| 行数（`wc -l`） | 354 |
| 粗估 token（`ceil(bytes / 3)`） | **3,286 tokens** |

`byte / 3` 是用户要求的保守粗估，不等同于特定 tokenizer 的精确计数。50 仓完整清单若每次 prompt 都原样注入，约占 3.3k token；规模按成员数近似线性增长，100 仓约 6.6k token、200 仓约 13.2k token（假设字段同样简短）。

## 5. 建议阈值与运行策略

这些数字仅针对本节硬件、CodeGraph 1.5.0 和 1,409 个极小源文件的合成 fixture；它们是**守门阈值/告警基线**，不是对真实仓库的容量承诺。

| 项目 | 建议 | 依据与处置 |
| --- | --- | --- |
| 50 成员首次聚合索引预算 | 目标 ≤ 10 s；软告警 > 30 s；硬失败/转异步 > 120 s | 实测墙钟 1.363 s，留出 7 倍目标余量给真实解析、冷缓存与更多关系；超时不能阻塞注册流程，应报进度/异步完成。 |
| 3–5 文件增量 sync 预算 | 目标 ≤ 3 s；软告警 > 10 s；硬失败/转后台 > 30 s | 实测 0.418 s，目标给约 7 倍余量。sync 失败必须保留 stale 标记，不能报“已更新”。 |
| 内存防护 | 预算 512 MiB；告警 ≥ 768 MiB | 保守的 `ru_maxrss` 首次为约 413 MiB；应使用 `/usr/bin/time -v` 可用的 CI/目标机再校准。 |
| stale 检测周期 | 30 s 轮询兜底；文件变更/成员表变化即时标记 stale；对活跃写入使用 2 s 静默去抖 | CodeGraph 文档称 watcher 自动 sync，但 Cadence 不应把正确性仅托付给常驻 watcher；工作流调用前应检查索引状态。 |
| inventory 注入预算 | 默认最多 4 KiB / ~1,400 token 的紧凑清单；上限 8 KiB / ~2,700 token；超出时只注入目标成员 + 计数/摘要并按需查询全量 | 本次 50 项详细 JSON 已为 9,857 B / ~3,286 tokens；不能无截断地每轮塞全量。保留 `id`、alias、相对路径、role，profile 仅按需展开。 |
| DB 容量告警（同类 fixture） | 50 仓基线 10 MiB；告警 > 25 MiB | 实测初始 9.2 MiB、一次 sync 后 9.8 MiB；真实仓应按“可索引源码数/图节点数”记录新基线，而不是按仓数机械限额。 |

## 6. 测试硬件、OS 与可解释边界

| 项目 | 实测值 |
| --- | --- |
| OS / kernel | Arch Linux rolling；Linux `6.18.25-x64v3-xanmod1`，x86_64 |
| CPU | 13th Gen Intel Core i5-13500H；`nproc=16`（物理 core/socket 报 12，2 threads/core） |
| 内存 | 46 GiB；测试时 available 约 37 GiB；swap 31 GiB |
| Git | 2.54.0 |
| Node | v24.17.0 |
| CodeGraph | 1.5.0 |
| 索引根 | 本地 Linux `/tmp`，非网络盘；`parent/` 本身非 Git，50 个子目录各有独立 Git 元数据 |

### 风险与解释

1. 这不是“50 个真实仓库”的性能预测：源码非常短，没有依赖树、生成代码、大量跨仓符号或百万级文件，真实 DB、边数和解析时间可能显著上升。
2. CodeGraph 的 CLI 内部计时（372/128 ms）不含完整 Node 进程启动等端到端开销；调度预算采用 1.363/0.418 s 的墙钟数字。
3. 本机缺少 `/usr/bin/time -v`；内存为采样与 `ru_maxrss` 的双口径。CI 或目标开发机应安装 GNU time，并把 `Maximum resident set size` 作为统一标准重测。
4. sync 后 DB 由 9.2 MiB 增至 9.8 MiB；长期修改可能出现 SQLite 文件增长，需在长期 soak 中测量是否需要 CodeGraph 自身的重建/压实策略。
5. 一次 `codegraph query 'Incremental spike marker' --json` 对注释文本没有提供可靠的精确符号验证（返回同 namespace 的 FTS 结果）；增量正确性的主证据是 CLI 明确报告 3 changed files、21 nodes 并返回 up-to-date status。后续回归应使用新增/改名的可声明符号作查询断言。

## 7. Worktree 隔离

运行 spike 前和写入报告前均检查了目标 worktree：已有 14 个 OpenSpec 文件处于修改状态，来自其他工作；本 spike 的 fixture、`.codegraph`、脚本和 Git commits 均位于 `/tmp`。本任务没有执行 `git add`；报告是唯一写入 worktree 的产物。
