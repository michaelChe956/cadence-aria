# Aria npx 本地化分发 · P4：GitHub Actions 发布 workflow

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `v*` tag 触发的两阶段 release workflow——多平台原生构建（tar 保执行位）→ 归集组包（chmod +x + test -x + 体积 gate）→ 发布 npm（子包先、主包后）→ 创建 GitHub Release 附带三平台二进制与 SHA256SUMS。并汇总开发文档。

**Architecture:** GitHub Actions：`build` job 用 matrix 在 ubuntu/macos-13/macos-14 上各自 `pnpm -C web build` + `cargo build --release`，tar 打包二进制上传 artifact；`release` job `needs: build`，下载解包、复用 P3 的 `scripts/pack-npm.mjs` 组包、`npm publish`、`gh release create`。

**Tech Stack:** GitHub Actions（actions/checkout、setup-node、pnpm/action-setup、dtolnay/rust-toolchain、Swatinem/rust-cache、upload/download-artifact、softprops/action-gh-release）。

**对应设计：** v1.3 第 6 节全节；总览 P4 行 + 交付物清单第 5、7 项。

**前置：** P3 已交付（`scripts/pack-npm.mjs`、`npm/` 包结构）。workflow 仅编排，不改 P1-P3 代码。

---

## 文件结构

| 文件 | 职责 | 操作 |
|------|------|------|
| `.github/workflows/release.yml` | 两阶段发布 workflow | Create |
| `scripts/make-release-assets.mjs` | 生成 GitHub Release 附件（tar.gz + SHA256SUMS） | Create |
| `cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md` | 开发/发布文档（交付物第 7 项） | Create |

---

## Task 1：Release 附件生成脚本

**Files:**
- Create: `scripts/make-release-assets.mjs`

> 归集 job 拿到三平台二进制后，为 GitHub Release 生成 `aria-<platform>.tar.gz`（含二进制 + LICENSE + README）与 `SHA256SUMS`。与 npm 组包分离，面向直接下载用户。

- [ ] **Step 1: 写脚本**

创建 `scripts/make-release-assets.mjs`：

```js
#!/usr/bin/env node
"use strict";

// 用法：node scripts/make-release-assets.mjs --version 0.1.0 --out dist-release \
//        --binary linux-x64=path/to/aria --binary darwin-x64=... --binary darwin-arm64=...
//
// 为每个平台生成 aria-<platform>.tar.gz（含 aria + LICENSE + README.md），
// 并在 out 目录生成 SHA256SUMS。

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const crypto = require("node:crypto");

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");

function args() {
  const out = { binaries: {} };
  const a = process.argv.slice(2);
  for (let i = 0; i < a.length; i++) {
    if (a[i] === "--version") out.version = a[++i];
    else if (a[i] === "--out") out.out = a[++i];
    else if (a[i] === "--binary") {
      const [key, p] = a[++i].split("=");
      out.binaries[key] = p;
    }
  }
  return out;
}

function sha256(file) {
  const buf = fs.readFileSync(file);
  return crypto.createHash("sha256").update(buf).digest("hex");
}

function main() {
  const { version, out, binaries } = args();
  if (!version || !out) throw new Error("必须指定 --version 与 --out");
  fs.mkdirSync(out, { recursive: true });

  const sums = [];
  for (const [key, binPath] of Object.entries(binaries)) {
    if (!fs.existsSync(binPath)) throw new Error(`二进制不存在：${key}=${binPath}`);
    // 组装 staging 目录：aria + LICENSE + README
    const stage = fs.mkdtempSync(path.join(os.tmpdir(), `aria-rel-${key}-`));
    fs.copyFileSync(binPath, path.join(stage, "aria"));
    fs.chmodSync(path.join(stage, "aria"), 0o755);
    fs.copyFileSync(path.join(repoRoot, "LICENSE"), path.join(stage, "LICENSE"));
    fs.copyFileSync(path.join(repoRoot, "npm/cli/README.md"), path.join(stage, "README.md"));

    const tarName = `aria-${key}.tar.gz`;
    const tarPath = path.join(out, tarName);
    // tar 保留执行位
    execFileSync("tar", ["-czf", tarPath, "-C", stage, "aria", "LICENSE", "README.md"]);
    sums.push(`${sha256(tarPath)}  ${tarName}`);
    console.log(`生成 ${tarName}`);
  }

  fs.writeFileSync(path.join(out, "SHA256SUMS"), sums.join("\n") + "\n");
  console.log(`生成 SHA256SUMS（${sums.length} 项）`);
}

main();
```

- [ ] **Step 2: 校验语法**

Run: `node --check scripts/make-release-assets.mjs`
Expected: 无输出。

- [ ] **Step 3: 本地验证（用 release 二进制造单平台附件）**

Run:
```bash
pnpm -C web build && cargo build --release --locked
node scripts/make-release-assets.mjs --version 0.0.1-test --out /tmp/aria-rel --binary linux-x64=target/release/aria
ls -lh /tmp/aria-rel && cat /tmp/aria-rel/SHA256SUMS
tar -tzf /tmp/aria-rel/aria-linux-x64.tar.gz
```
Expected: 生成 `aria-linux-x64.tar.gz` 与 `SHA256SUMS`；tar 内含 `aria`、`LICENSE`、`README.md`。

- [ ] **Step 4: 提交**

```bash
git add scripts/make-release-assets.mjs
git commit -m "feat(release): GitHub Release 附件生成脚本(tar.gz + SHA256SUMS)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2：release workflow — build 阶段（多平台矩阵）

**Files:**
- Create: `.github/workflows/release.yml`（先写 build job）

- [ ] **Step 1: 写 workflow 头部与 build job**

创建 `.github/workflows/release.yml`：

```yaml
name: Release npx

on:
  push:
    tags:
      - "v*"

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: build ${{ matrix.target_key }}
    strategy:
      fail-fast: true
      matrix:
        include:
          - os: ubuntu-latest
            target_key: linux-x64
            rust_target: x86_64-unknown-linux-gnu
          # macos-13 为 Intel runner，存在退役风险（设计 v1.3 §6.1）；
          # 若未来下线，改用 arm64 runner 交叉编译 x86_64-apple-darwin（需额外验证 git 重依赖交叉编译）。
          - os: macos-13
            target_key: darwin-x64
            rust_target: x86_64-apple-darwin
          - os: macos-14
            target_key: darwin-arm64
            rust_target: aarch64-apple-darwin
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: "20"

      - name: Setup pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 10

      - name: Rust toolchain
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: 1.95.0
          targets: ${{ matrix.rust_target }}

      - name: Cargo cache
        uses: Swatinem/rust-cache@v2

      # 强制顺序：前端先于二进制（rust-embed 嵌入 web/dist；build.rs 硬校验兜底）
      - name: Build frontend
        run: pnpm -C web install --frozen-lockfile && pnpm -C web build

      - name: Build release binary
        run: cargo build --release --locked --target ${{ matrix.rust_target }}

      # tar 打包保执行位（actions/upload-artifact 以 zip 存储会丢 +x，设计 v1.3 §6.1）
      - name: Package binary (tar preserves +x)
        run: |
          mkdir -p out
          tar -czf "out/aria-${{ matrix.target_key }}.tar.gz" \
            -C "target/${{ matrix.rust_target }}/release" aria

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: aria-${{ matrix.target_key }}
          path: out/aria-${{ matrix.target_key }}.tar.gz
          retention-days: 1
```

- [ ] **Step 2: 校验 YAML 语法**

Run:
```bash
node -e "const fs=require('node:fs'); const s=fs.readFileSync('.github/workflows/release.yml','utf8'); if(!s.includes('jobs:')||!s.includes('build:')) throw new Error('结构缺失'); console.log('结构 ok')"
```
Expected: `结构 ok`。如本机有 `yamllint`/`actionlint` 更佳：`actionlint .github/workflows/release.yml`（可选）。

- [ ] **Step 3: 提交**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): build 阶段多平台原生构建 + tar 保执行位上传

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3：release workflow — release 阶段（归集发布）

**Files:**
- Modify: `.github/workflows/release.yml`（追加 release job）

- [ ] **Step 1: 追加 release job**

在 `.github/workflows/release.yml` 末尾（与 `build` job 同级）追加：

```yaml
  release:
    name: publish npm + github release
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write   # 创建 GitHub Release 需要（默认 GITHUB_TOKEN 即可）
    steps:
      - uses: actions/checkout@v4

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: "20"
          registry-url: "https://registry.npmjs.org"

      - name: Resolve version from tag
        id: ver
        run: echo "version=${GITHUB_REF_NAME#v}" >> "$GITHUB_OUTPUT"

      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts

      # 解包三平台 tar（恢复执行位），整理出各平台二进制路径
      - name: Extract binaries
        run: |
          set -euo pipefail
          mkdir -p bins
          for key in linux-x64 darwin-x64 darwin-arm64; do
            tar -xzf "artifacts/aria-${key}/aria-${key}.tar.gz" -C bins
            mv bins/aria "bins/aria-${key}"
            chmod +x "bins/aria-${key}"   # 双保险（设计 §6.2）
            test -x "bins/aria-${key}"    # gate：缺执行位则失败
          done
          ls -l bins

      # 复用 P3 组包脚本：主包版本 + 三平台子包（chmod +x + test -x 在脚本内）
      - name: Assemble npm packages
        run: |
          set -euo pipefail
          V="${{ steps.ver.outputs.version }}"
          node scripts/pack-npm.mjs --version "$V" --main-only
          node scripts/pack-npm.mjs --version "$V" --platform linux-x64    --binary bins/aria-linux-x64
          node scripts/pack-npm.mjs --version "$V" --platform darwin-x64   --binary bins/aria-darwin-x64
          node scripts/pack-npm.mjs --version "$V" --platform darwin-arm64 --binary bins/aria-darwin-arm64

      # 体积 gate（设计 §7.3）：超阈即失败（阈值首次实测后校准）
      - name: Size gate
        run: |
          set -euo pipefail
          MAX_BIN=$((90 * 1024 * 1024))   # 90MB 上限（初值，按实测调整）
          for key in linux-x64 darwin-x64 darwin-arm64; do
            sz=$(stat -c%s "npm/cli-${key}/bin/aria")
            echo "cli-${key} 二进制大小: $sz bytes"
            if [ "$sz" -gt "$MAX_BIN" ]; then
              echo "::error::cli-${key} 二进制超过体积上限 ($sz > $MAX_BIN)"; exit 1
            fi
          done

      # 发布顺序：子包先、主包后（设计 §6.3）；任一失败整体失败
      - name: Publish subpackages then main
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
        run: |
          set -euo pipefail
          for key in linux-x64 darwin-x64 darwin-arm64; do
            npm publish "npm/cli-${key}" --access public
          done
          npm publish "npm/cli" --access public

      # GitHub Release 附件（设计 §6.4）
      - name: Make release assets
        run: |
          set -euo pipefail
          node scripts/make-release-assets.mjs \
            --version "${{ steps.ver.outputs.version }}" --out dist-release \
            --binary linux-x64=bins/aria-linux-x64 \
            --binary darwin-x64=bins/aria-darwin-x64 \
            --binary darwin-arm64=bins/aria-darwin-arm64

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          name: ${{ github.ref_name }}
          generate_release_notes: true
          files: |
            dist-release/aria-linux-x64.tar.gz
            dist-release/aria-darwin-x64.tar.gz
            dist-release/aria-darwin-arm64.tar.gz
            dist-release/SHA256SUMS
```

> **publish 非原子性提醒**（设计 §6.3）：若 publish 中途失败（如 2/3 子包已发），**不要**覆盖同版本——npm 不允许。改用新 patch tag（如 `v0.1.1`）整体重发。该说明写入 Task 5 文档。

- [ ] **Step 2: 校验 YAML 结构**

Run:
```bash
node -e "const s=require('node:fs').readFileSync('.github/workflows/release.yml','utf8'); for(const k of ['build:','release:','needs: build','contents: write','npm publish','action-gh-release']){ if(!s.includes(k)) throw new Error('缺 '+k);} console.log('结构 ok')"
```
Expected: `结构 ok`。

- [ ] **Step 3: 提交**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): release 阶段归集组包+体积gate+npm发布+GitHub Release

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4：组包脚本与 workflow 的契约对齐验证

**Files:**（无新增，验证 P3 脚本与 P4 workflow 字段一致）

> 防止 P3 的 `pack-npm.mjs` 接口与 P4 workflow 调用漂移（writing-plans 自检：类型/接口一致性）。

- [ ] **Step 1: 验证 pack-npm.mjs 接口与 workflow 调用一致**

Run:
```bash
grep -n "platform\|--binary\|--main-only\|--version" scripts/pack-npm.mjs | head
grep -n "pack-npm.mjs" .github/workflows/release.yml
```
Expected: workflow 调用的 `--version` / `--main-only` / `--platform <key>` / `--binary <path>` 四种形态均被 `pack-npm.mjs` 的 `arg()` 解析支持；平台 key（`linux-x64`/`darwin-x64`/`darwin-arm64`）与 `pack-npm.mjs` 的 `SUPPORTED` 数组完全一致。

- [ ] **Step 2: 本地端到端串跑（模拟 release job 的组包+附件，单平台）**

Run:
```bash
pnpm -C web build && cargo build --release --locked
mkdir -p bins && cp target/release/aria bins/aria-linux-x64 && chmod +x bins/aria-linux-x64 && test -x bins/aria-linux-x64
node scripts/pack-npm.mjs --version 0.0.1-test --main-only
node scripts/pack-npm.mjs --version 0.0.1-test --platform linux-x64 --binary bins/aria-linux-x64
test -x npm/cli-linux-x64/bin/aria && echo "✓ 子包二进制可执行"
node scripts/make-release-assets.mjs --version 0.0.1-test --out /tmp/aria-rel-e2e --binary linux-x64=bins/aria-linux-x64
ls /tmp/aria-rel-e2e
```
Expected: `✓ 子包二进制可执行`；`/tmp/aria-rel-e2e` 含 `aria-linux-x64.tar.gz` 与 `SHA256SUMS`。

- [ ] **Step 3: 清理本地产物（不入库）**

Run:
```bash
git checkout npm/cli/package.json 2>/dev/null || true
rm -rf npm/cli-*/bin npm/cli-*/package.json bins /tmp/aria-rel-e2e
git status --short
```
Expected: 工作区干净（组包产物已被 P3 的 .gitignore 覆盖）。

> 本 Task 无代码改动，无需提交。

---

## Task 5：开发与发布指南文档

**Files:**
- Create: `cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md`

> 交付物清单第 7 项：汇总 `ARIA_WEB_DIST` 逃生口、构建顺序契约、Rust 测试前置、发布前置条件、版本管理、publish 非原子处理。

- [ ] **Step 1: 写文档**

创建 `cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md`：

````markdown
# README：Aria npx 分发开发与发布指南

- 文档类型：开发文档（README）
- 版本：v1.0
- 创建日期：2026-05-31
- 对应设计：`cadence/designs/2026-05-31_技术方案_Aria_npx本地化分发_v1.3.md`

## 1. 构建顺序契约（必读）

`web/dist` 被 `web/.gitignore` 忽略、不在版本控制中。`aria` 二进制通过 `rust-embed` 在**编译期**嵌入 `web/dist`，且 `build.rs` 会在其缺失时**硬失败**。因此：

> **任何 `cargo build` / `cargo test` / `cargo check` 前，必须先 `pnpm -C web build`。**

干净 checkout、切分支、CI 首次构建均适用。

## 2. 本地开发逃生口 ARIA_WEB_DIST

运行时默认从嵌入资源服务前端。开发/调试时若想改前端而不重编 Rust：

```bash
ARIA_WEB_DIST=$(pwd)/web/dist cargo run -- web --workspace .
```

设置后改从该磁盘目录读取（未设置或目录不存在则走嵌入）。playwright e2e（`web/e2e/start-api.mjs`）已自动注入该变量。

## 3. Rust 验证命令

遵循 `cadence/project-rules/build-test-commands.md`：

```bash
pnpm -C web build   # 前置，必须
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

🔴 禁止给 cargo 加 `-j 1`。

## 4. npm 包结构与本地冒烟

- 主包 `npm/cli`（仅 JS launcher，声明 `bin: aria` 与 optionalDependencies）。
- 平台子包 `npm/cli-<platform>`（含预编译二进制 `bin/aria`，**不声明 bin**）。
- 组包：`node scripts/pack-npm.mjs --version <v> --main-only` + `--platform <key> --binary <path>`。
- 本地冒烟：`node scripts/smoke-npx.mjs`（需先 `pnpm -C web build && cargo build --release`）。

launcher 单测：`node --test npm/cli/test/`。

## 5. 发布（Tag 触发）

推送 `v<x.y.z>` tag 触发 `.github/workflows/release.yml`：

1. build：ubuntu/macos-13/macos-14 各自原生编译，tar 打包上传 artifact。
2. release：下载解包 → chmod +x + test -x → 组包 → 体积 gate → npm publish（子包先、主包后）→ GitHub Release（附 tar.gz + SHA256SUMS）。

### 发布前置条件（维护者提供）

- npm `@cadence-aria` scope 已创建。
- 仓库 secret `NPM_TOKEN`（具 `@cadence-aria` 发布权限）。
- GitHub Release 用默认 `GITHUB_TOKEN` + `permissions: contents: write`，无需额外 PAT。
- 版本来源：tag `v0.1.0` → 包版本 `0.1.0`。

### publish 失败处理（非原子）

多包 publish 无事务。若中途失败（如 3 个子包发了 2 个），**不要覆盖同版本**（npm 不允许）。改用新 patch 版本（如 `v0.1.1`）整体重发。

## 6. 平台支持与风险

- 预编译矩阵：linux-x64、darwin-x64、darwin-arm64。
- `macos-13`（Intel runner）有退役风险；下线后改用 arm64 runner 交叉编译 x86_64-apple-darwin（需验证 git 重依赖交叉编译）。
- linux-arm64、Windows 原生为后续增量；非支持平台 launcher 给出清晰错误。

## 7. 待发布前确认（开放事项）

- 首个版本号（建议 `0.1.0`）。
- 启动检测提示中 codex/claude 安装指引文案/链接。
- release profile `opt-level`（`s` vs `z`）、`panic = "abort"` 最终取值。
- 体积 gate 阈值（workflow 中初值 90MB，按实测校准）。
````

- [ ] **Step 2: 校验文档存在且格式**

Run: `head -6 "cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md"`
Expected: 显示文档头部元信息。

- [ ] **Step 3: 提交**

```bash
git add "cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md"
git commit -m "docs: npx 分发开发与发布指南(构建契约/逃生口/发布流程/前置条件)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## P4 自检（完成所有 Task 后执行）

- [ ] **附件脚本**：`make-release-assets.mjs` 本地生成 tar.gz + SHA256SUMS，tar 内含 aria/LICENSE/README。
- [ ] **workflow 结构**：`release.yml` 含 build matrix（3 平台）+ release job（needs build、contents: write、tar 上传、download、chmod+test、组包、体积 gate、npm publish 子包先主包后、GitHub Release）。
- [ ] **契约对齐**：workflow 对 `pack-npm.mjs` 的调用参数与平台 key 同脚本 SUPPORTED 一致。
- [ ] **本地端到端串跑**：单平台组包 + 附件生成跑通，二进制执行位保留。
- [ ] **文档齐全**：开发/发布指南覆盖构建契约、逃生口、发布前置、publish 非原子处理。
- [ ] **工作区干净**：组包/附件产物未误入库。

完成后：四个分册全部交付，回到总览执行「完成定义（DoD）」终检。
