# Spike 2：CodeGraph 2–3 子仓统一索引实测

- **执行日期**：2026-08-08
- **结论状态**：假设**部分成立**；可以在非 Git 公共父目录建立一个统一索引，但它是按目录递归扫描，不是按 Git 子仓/manifest 自动成员发现。因此必须以根层 `codegraph.json` 的 `exclude` 加上子仓 `.gitignore` 作为显式边界契约。
- **临时 fixture**：为便于复核，暂保留于 `/tmp/spike-codegraph-20260808-202824/`（其中 `parent/` 为被索引的非 Git 父目录）；未在 worktree 中创建 fixture 或索引。

## 1. 版本、二进制来源与 CLI 语法

实测命令：

```sh
command -v codegraph
ls -l "$(command -v codegraph)"
readlink -f "$(command -v codegraph)"
codegraph --version
codegraph init --help
```

输出要点：

| 项目 | 实测值 |
| --- | --- |
| CodeGraph 版本 | `1.5.0` |
| PATH 入口 | `/home/michaelche/.local/share/fnm/aliases/default/bin/codegraph` |
| 入口实际指向 | `/home/michaelche/.local/share/fnm/node-versions/v24.17.0/installation/lib/node_modules/@colbymchenry/codegraph/npm-shim.js` |
| npm 包 | `@colbymchenry/codegraph@1.5.0`，仓库为 `github.com/colbymchenry/codegraph` |
| Node | `v24.17.0` |
| `init` 语法 | `codegraph init [options] [path]`；仅 `--index`（已废弃）、`--force`、`--verbose`，**没有** allowlist / exclude CLI 参数 |
| 查询语法 | `codegraph query <search> [--json]`、`codegraph explore <query...>`；另有 `callers` / `callees` |

## 2. Fixture 与执行 argv

父目录刻意不执行 `git init`：`git -C parent rev-parse --is-inside-work-tree` 返回 `fatal: not a git repository`。

目录内容如下：

- `parent/repo-a`、`repo-b`、`repo-c`：各自独立 `git init`、写入 Java/TypeScript fixture 并提交；
- `repo-a/.worktrees/test/src/WorktreeOnly.java`：模拟工作树资产；
- `repo-a/.aria/AriaAggregationAsset.java`：模拟聚合资产；
- `repo-a/build/generated/BuildOnly.java`：模拟构建产物；
- `parent/not-a-repo/src/{NotARepoOnly.java,not_a_repo.ts}`：**不是 Git 仓**的非成员目录。

首次（无根配置）执行的 argv：

```sh
cd /tmp/spike-codegraph-20260808-202824/parent
codegraph init . --verbose
```

其输出列出 8 个文件：`repo-a` 2、`repo-b` 2、`repo-c` 2，外加 `not-a-repo` 2；最终 `Indexed 8 files`、`28 nodes, 26 edges in 136ms`。

## 3. 子仓发现行为

### 观察结果

`init` 在非 Git 父目录成功建立 `.codegraph/`，并将三个子仓的源码均纳入一个 SQLite 图：

| 路径 | 首次索引（含 `repo-a/.gitignore`） | 说明 |
| --- | --- | --- |
| `repo-a` | 纳入 | 包含 Java 和 TypeScript 源码 |
| `repo-b` | 纳入 | 包含 Java 和 TypeScript 源码 |
| `repo-c` | 纳入 | 包含 Java 和 TypeScript 源码 |
| `not-a-repo` | **错误地纳入成员语义** | 2 个源文件被索引 |

这证明“在非 Git 公共父目录建立一个图”可行，也证明它并**未**将 Git 边界当作成员 allowlist：`not-a-repo` 没有 `.git` 仍被扫描。实测中未看到“发现 3 个 repository”之类的仓级注册输出；结果是一个以 `parent` 为根、路径带 `repo-a/` 等前缀的目录图。

### 代表性跨子仓关系

`repo-a` 的 `AConsumer` 导入/调用 `repo-b` 的 `BSharedService`：

```java
// repo-a/src/main/java/com/example/a/AConsumer.java
import com.example.b.BSharedService;
public String callSharedService(String name) {
    return service.crossRepoGreeting(name);
}
```

```java
// repo-b/src/main/java/com/example/b/BSharedService.java
public String crossRepoGreeting(String name) { ... }
```

实测：

```sh
codegraph callers crossRepoGreeting
# Callers of "crossRepoGreeting" (1):
# method callSharedService
#   repo-a/src/main/java/com/example/a/AConsumer.java:7

codegraph callees callSharedService
# Callees of "callSharedService" (1):
# method crossRepoGreeting
#   repo-b/src/main/java/com/example/b/BSharedService.java:5
```

`codegraph explore 'trace callSharedService and crossRepoGreeting across repo-a and repo-b' --max-files 10` 返回两个 Java 文件的源码，并给出 `crossRepoGreeting ... 1 caller; tests: repo-a/.../AConsumer.java`。这是一条可解析的跨子仓边，说明统一索引中的跨路径解析可以工作（fixture 的 Java 包名和 TypeScript 相对导入均故意跨仓）。

## 4. allowlist / exclude 配置实测

### CLI 参数

`codegraph init --help` 和 `codegraph index --help` 均没有 allowlist / exclude 参数；因此不能把成员边界放在 CLI argv 中。

### 配置文件形态

已安装包的 `README.md` “Configuration”章节和 `dist/project-config.d.ts` 声明项目根配置文件为 `codegraph.json`，支持 `exclude`、`include`、`includeIgnored`。其中：

- `exclude`：根目录相对、gitignore 风格模式；索引、sync、watch 都会执行；
- `include`：将被 `.gitignore` 排除的普通第一方源码重新纳入；
- `includeIgnored`：只用于重新发现被忽略目录下的嵌套 Git 仓，**不是**普通源码 allowlist；
- 内建排除（例如 `.git`、`build`、`node_modules`、`dist`）不可被重新纳入。

下列配置在 `parent/codegraph.json` 写入后，以 `codegraph index . --verbose` 实测生效：从 8 个文件降到 6 个，`not-a-repo` 不再出现在 `codegraph files`。

```json
{
  "exclude": [
    "not-a-repo/",
    "repo-a/.worktrees/",
    "repo-a/.aria/",
    "repo-a/build/"
  ]
}
```

最终 status：`Files: 6`、`Nodes: 23`、`Edges: 23`。

> 这里的 `exclude` 是 denylist，不是“只允许 manifest 成员”的真正 allowlist。若 manifest 可枚举成员，实施层必须将所有非成员目录转换为受控的根级排除模式，或改为仅把允许成员投影/链接到一个专用索引根；不能假定 CodeGraph 会读 manifest 或只扫描 Git 子仓。

## 5. 排除行为矩阵与查询证据

查询均用唯一符号与 `codegraph query <symbol> --json` 验证；`[]` 代表未命中。

| 目标 | 无根配置、但 `repo-a/.gitignore` 存在 | 移除该 `.gitignore` 且根配置仅排除 `not-a-repo/` | 使用上述完整根 `codegraph.json` | 结论 |
| --- | --- | --- | --- | --- |
| `not-a-repo` | **纳入**；`SHOULD_NOT_INDEX_NONMEMBER` 命中 Java 和 TS 两项 | `[]` | `[]` | 默认不按 Git 成员过滤；必须显式根层排除 |
| `.worktrees` | `[]` | **纳入**；`SHOULD_NOT_INDEX_WORKTREE` 命中 | `[]` | 首轮排除来自子仓 `.gitignore`，不是 CodeGraph 默认 |
| `.aria` | `[]` | **纳入**；`SHOULD_NOT_INDEX_ARIA` 命中 | `[]` | 同上，必须作为项目规则/配置契约 |
| `build` | `[]` | `[]` | `[]` | CodeGraph 1.5.0 内建排除 `build`；仍建议显式写入契约以消除版本行为漂移 |
| `.git` | 未出现 | 未出现 | 未出现 | 内建排除；未对其放置可查询源码（Git 元数据非源码） |

尤其重要的是第二列：在暂时移除 `repo-a/.gitignore` 后，重新索引输出明确列出：

```text
repo-a/.aria/AriaAggregationAsset.java
repo-a/.worktrees/test/src/WorktreeOnly.java
```

这排除了“`.worktrees`/`.aria` 是默认安全排除项”的误判。此试验后已恢复 fixture 的 `.gitignore`，并再次按完整 `codegraph.json` 全量重建，最终索引回到 6 文件。

## 6. DB 目录与大小

非 Git 父目录下生成：

```text
parent/.codegraph/
├── .gitignore
└── codegraph.db
```

`codegraph status` 报 `Backend: node:sqlite — built-in (full WAL)`、`Journal: wal`。实测磁盘用量：

| 阶段 | DB / 目录大小 |
| --- | --- |
| 首次 8 文件索引 | `codegraph.db` 164 KiB；`.codegraph/` 172 KiB |
| 完整配置、6 文件索引 | `codegraph.db` 164 KiB；`.codegraph/` 172 KiB |

`.codegraph/.gitignore` 是 CodeGraph 生成的本地索引忽略规则；不存在仓清单、子仓注册表或自动成员 manifest。

## 7. D3 结论与建议

### 是否成立

D3 的“非 Git 公共父目录建一份统一 CodeGraph 索引”作为**技术能力**成立：`codegraph init . --verbose` 成功，三个子仓同时入图，跨子仓 `callers`/`callees` 命中。

D3 的“自动发现子仓、基于 manifest allowlist 排除非成员和工作资产”作为**当前 CodeGraph 原生能力**不成立：1.5.0 没有 allowlist argv，也没有按 Git 仓自动成员约束；它会扫描非成员普通目录。`.worktrees` / `.aria` 的安全性依赖 `.gitignore` 或 `codegraph.json`，不是内建保证。

### 应写入变更的配置契约

1. 聚合根必须是 manifest 所属的、非 Git 的公共父目录；在该根执行 `codegraph init .` / `sync .`。
2. 聚合根必须由 Cadence 生成并维护 `codegraph.json` 的 `exclude`（根相对 gitignore 模式），至少覆盖：非成员目录、`**/.worktrees/`、`**/.aria/`、构建目录和其他聚合资产。对动态成员，须在成员增删后原子更新该文件并 `sync`/全量 `index`。
3. 每个成员仓仍应提交 `.gitignore`，覆盖本仓不应索引的派生产物；不应仅靠它保护聚合资产。
4. 在 index 后用 `codegraph files` 和唯一符号负查询验证边界；将“发现非成员路径”作为失败而非静默接受。
5. 若需要真正的正向 allowlist（而不是枚举 denylist），不要依赖 CodeGraph 1.5.0：使用隔离 staging/index 根，仅放置 manifest 允许成员的目录链接/副本，或改为每仓独立 `.codegraph` 索引并由 Cadence 在查询层聚合。

### 版本钉定

建议精确钉定 `@colbymchenry/codegraph@1.5.0`（而非浮动 `latest` 或仅 `^1.5.0`），并在升级时重新跑本 spike 的四类边界回归：非 Git 目录、非成员目录、`.worktrees`/`.aria`、跨仓调用边。理由是本方案依赖 1.5.0 的 `codegraph.json.exclude`、内建 `build` 忽略与跨路径解析的实测行为。

## 8. 可复现命令摘要

```sh
# 首次统一索引
cd /tmp/spike-codegraph-20260808-202824/parent
codegraph init . --verbose
codegraph files
codegraph callers crossRepoGreeting
codegraph callees callSharedService

# 用根层排除契约重建
codegraph index . --verbose
codegraph query SHOULD_NOT_INDEX_NONMEMBER --json
codegraph query SHOULD_NOT_INDEX_WORKTREE --json
codegraph query SHOULD_NOT_INDEX_ARIA --json
codegraph query SHOULD_NOT_INDEX_BUILD --json
codegraph status .
```

所有命令只操作 `/tmp` fixture；worktree 在写报告前的已有 OpenSpec 修改保持原样，未执行 `git add`。
