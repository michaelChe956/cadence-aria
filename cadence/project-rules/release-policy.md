# 版本发布策略规则（默认不发预发布 · 合并后再发版）

## 适用范围

本仓库的一切**版本发布**动作：打版本 tag、触发发布流水线（`.github/workflows/release.yml`）、`npm publish`、创建 GitHub Release，以及面向客户/用户的版本通告。

适用于所有会话、所有 subagent 派工。

## 强制要求

### 1. 默认不发 rc / 预发布版本

- 🔴 **禁止**在未获 owner 明确指示时，创建带 `-rc` / `-beta` / `-alpha` 等预发布后缀的版本号或 tag。
- 默认只有**一种**发版形态：**改动合并进 `main` 之后**发布正式版。

### 2. 必须先合并，再发版

- 发版前提：该版本对应的改动**已在 `origin/main`**（对应 PR 已合并）。
- 🔴 **禁止**从尚未合并的功能分支发布任何版本（正式版与预发布 alike）——正式版产物必须可从 `main` 复现，否则版本谱系与仓库提交对不上。
- 固定顺序：**① PR 合并进 `main` → ② 在合并提交上打版本 tag → ③ 推 tag 触发发布流水线**。
- 需要给外部/客户先试用时，走**正式版的预发渠道**（如把正式版产物交给对方试装），而不是另发一个版本号。

### 3. 发版后必须核对发布态（不核对 = 发版未完成）

流水线成功后**立刻**核对三处，任一不符即视为发版未完成：

```bash
# ① npm 默认标签必须指向本次正式版
npm view @cadence-aria/cli dist-tags        # 期望 latest == <本次正式版>
# ② GitHub Release 的预发布标记必须正确
gh release view <tag> --json isPrerelease    # 正式版期望 false
# ③ 产物内嵌版本与构建 commit 必须对得上
strings -a <解包出的 aria> | grep -E "<版本号>|<commit 前 12 位>"
```

若 `latest` 指向了**非正式版**（例如误发的 rc），必须立即用 `npm dist-tag add @cadence-aria/cli@<正式版> latest` 归位并复核；该操作需要 npm 发布权。

同时检查**面向用户的文档口径**（如客户机测试指引）里写的「当前 latest 指向哪个版本 / 安装命令写什么版本号」是否与实际一致——发布一旦完成，文档里的版本描述即过期，必须同步订正。

### 4. 确需预发布时的附加约束（仅当 owner 明确指示）

- 版本号必须带预发布后缀（`-rc1` 等）；
- 发布必须走 `next` 标签（`release.yml` 的「Resolve version and dist-tag from tag」已按后缀自动判定）；
- GitHub Release 必须标 `prerelease=true`（同一步骤已按 tag 后缀自动判定）；
- 发布后按第 3 条核对发布态，并确认 `latest` **未**被预发布版占用。

## 本次实例（2026-11-14，v0.1.6）

- 背景：修复需先在客户的 M 芯片 Mac 上实测，而当时改动**尚未合并**（`main` 仍是 v0.1.5）。
- 做法：打了 `v0.1.6-rc1`，发布命令未带 `--tag` ⇒ npm 按默认把预发布版挂到 `latest` ⇒ **候选版成了所有人的默认安装版**；同时因预发布包先于正式版发布，客户指引里「latest 仍指向稳定版 0.1.5」的描述当场过期。
- 处置：正式版 `v0.1.6` 发布后 `latest` 自动归位；`release.yml` 补上 dist-tag 解析与 `prerelease` 标记（预发布挂 `next`、按 tag 后缀标预发布）；已发布的 rc 补标 `prerelease=true`。
- 结论：预发布本身不是问题，**未合并就发版 + 机制未对齐（缺 `--tag`）+ 发版后未核对发布态**三者叠加才造成事故。

## 关联

- 分支与合并纪律：`cadence/project-rules/branch-protection.md`
- 交付完整性：`cadence/project-rules/delivery-completeness.md`
