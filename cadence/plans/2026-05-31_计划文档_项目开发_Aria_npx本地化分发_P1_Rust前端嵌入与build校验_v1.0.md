# Aria npx 本地化分发 · P1：Rust 前端嵌入与 build 校验

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让前端 `web/dist` 在编译期嵌入 `aria` 二进制，运行时统一从嵌入资源服务前端，仅当显式设置 `ARIA_WEB_DIST` 时改读磁盘目录；新增 `build.rs` 在 `web/dist` 缺失/为空时硬失败，杜绝静默白屏。

**Architecture:** 用 `rust-embed`（启用 `debug-embed` feature，debug/release 行为一致）嵌入 `web/dist`。`src/web/static_assets.rs` 改为返回一个统一的 axum fallback `Service`：未设 `ARIA_WEB_DIST` 时走嵌入资源（命中返回文件 + 正确 Content-Type，未命中 fallback 到嵌入 `index.html`）；设了则走 `ServeDir`。`build.rs` 编译期校验 `web/dist/index.html` 存在。

**Tech Stack:** Rust（axum 0.8、rust-embed、mime_guess、tower）。

**对应设计：** v1.3 第 4.1、4.2 节；总览 P1 行。

**前置：** 本分册所有 `cargo` 命令前必须先 `pnpm -C web build` 生成 `web/dist`（否则 Task 3 起 build.rs 会硬失败）。

---

## 文件结构

| 文件 | 职责 | 操作 |
|------|------|------|
| `Cargo.toml` | 新增 `rust-embed` 依赖 | Modify |
| `build.rs` | 编译期校验 `web/dist` 存在且非空 | Create |
| `src/web/static_assets.rs` | 嵌入资源服务 + `ARIA_WEB_DIST` 逃生口 | Modify（重写） |
| `src/web/app.rs` | 挂载点适配（`serve_web` 调用新接口） | Modify（`:221` 附近） |
| `tests/it_web/web_static_assets.rs` | 嵌入/fallback/逃生口集成测试 | Create |
| `tests/it_web.rs` | 注册新测试模块 | Modify |
| `web/e2e/start-api.mjs` | 注入 `ARIA_WEB_DIST` 适配嵌入 | Modify |

---

## Task 1：引入 rust-embed 依赖

**Files:**
- Modify: `Cargo.toml`（`[dependencies]` 区）

- [ ] **Step 1: 添加依赖**

在 `Cargo.toml` 的 `[dependencies]` 区，按字母序在 `pulldown-cmark` 之后、`serde` 之前插入：

```toml
rust-embed = { version = "8", features = ["debug-embed"] }
```

> `debug-embed` feature 使 debug 构建也嵌入资源（默认 debug 是磁盘动态读取）。本方案要 debug/release 统一走嵌入，故必须开启。`mime_guess` 已在依赖中（rust-embed 内部也用它，不冲突）。

- [ ] **Step 2: 验证依赖解析**

Run: `pnpm -C web build && cargo check --locked`
Expected: 编译通过（此时 static_assets 尚未改，rust-embed 仅引入未用，可能有 unused 警告，下一 Task 消除）。若 `Cargo.lock` 更新，属正常。

- [ ] **Step 3: 提交**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: 引入 rust-embed(debug-embed) 依赖用于前端嵌入

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2：重写 static_assets 为嵌入服务 + 逃生口

**Files:**
- Modify: `src/web/static_assets.rs`（整体重写）

本 Task 先实现，测试在 Task 4 补（Task 3 的 build.rs 是测试运行的前置）。本 Task 末尾用 `cargo check` 确认编译。

- [ ] **Step 1: 重写 static_assets.rs**

将 `src/web/static_assets.rs` 全文替换为：

```rust
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use tower_http::services::{ServeDir, ServeFile};

/// 编译期嵌入的前端构建产物。`web/dist` 必须先由 `pnpm -C web build` 生成
/// （build.rs 会在缺失时硬失败）。
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct WebAssets;

const INDEX_HTML: &str = "index.html";

/// 统一的前端静态资源 fallback 服务。
///
/// - 未设置 `ARIA_WEB_DIST`：从嵌入资源服务（命中返回文件，未命中 SPA fallback 到 index.html）。
/// - 设置了 `ARIA_WEB_DIST` 且目录存在：改用 ServeDir 从该磁盘目录读取（开发/e2e 逃生口）。
pub fn static_dist_service() -> StaticDistService {
    match std::env::var_os("ARIA_WEB_DIST") {
        Some(dir) if !dir.is_empty() && PathBuf::from(&dir).is_dir() => {
            let dir = PathBuf::from(dir);
            let index = dir.join(INDEX_HTML);
            StaticDistService::Disk(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        _ => StaticDistService::Embedded,
    }
}

/// fallback 服务的两种形态：嵌入资源 或 磁盘目录（ServeDir）。
#[derive(Clone)]
pub enum StaticDistService {
    Embedded,
    Disk(ServeDir<ServeFile>),
}

impl StaticDistService {
    /// 从嵌入资源构造响应：精确命中返回该文件，未命中返回 index.html（SPA fallback）。
    fn embedded_response(uri: &Uri) -> Response {
        let path = uri.path().trim_start_matches('/');
        let candidate = if path.is_empty() { INDEX_HTML } else { path };

        if let Some(content) = WebAssets::get(candidate) {
            return embedded_file_response(candidate, content.data.into_owned());
        }
        // SPA fallback：未命中一律返回 index.html
        match WebAssets::get(INDEX_HTML) {
            Some(content) => embedded_file_response(INDEX_HTML, content.data.into_owned()),
            None => (StatusCode::NOT_FOUND, "index.html not embedded").into_response(),
        }
    }
}

fn embedded_file_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Body::from(bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response
}
```

> 注：`StaticDistService` 作为 axum fallback 需实现 `Service`。下一步用一个适配 handler 把它接入 router，避免手写 `tower::Service`。

- [ ] **Step 2: 在 static_assets.rs 末尾追加 axum 接入 handler**

在同文件末尾追加：

```rust
use axum::extract::Request;
use std::convert::Infallible;
use tower::ServiceExt; // for ServeDir oneshot

/// 作为 axum fallback 的统一 handler。根据服务形态分派到嵌入资源或磁盘 ServeDir。
pub async fn serve_static(service: StaticDistService, req: Request) -> Result<Response, Infallible> {
    match service {
        StaticDistService::Embedded => Ok(StaticDistService::embedded_response(req.uri())),
        StaticDistService::Disk(serve_dir) => {
            // ServeDir 的 fallback(ServeFile) 已覆盖未命中→index.html
            match serve_dir.oneshot(req).await {
                Ok(resp) => Ok(resp.into_response()),
                Err(err) => match err {},
            }
        }
    }
}
```

- [ ] **Step 3: 验证编译**

Run: `pnpm -C web build && cargo check --locked`
Expected: 编译通过（`serve_static`/`static_dist_service` 暂未被 app.rs 调用，可能 unused 警告，Task 5 消除）。

- [ ] **Step 4: 提交**

```bash
git add src/web/static_assets.rs
git commit -m "feat: static_assets 改为嵌入资源服务 + ARIA_WEB_DIST 逃生口

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3：新增 build.rs 硬校验

**Files:**
- Create: `build.rs`（仓库根）

- [ ] **Step 1: 创建 build.rs**

在仓库根创建 `build.rs`：

```rust
use std::path::Path;

fn main() {
    let dist = Path::new("web/dist");
    let index = dist.join("index.html");

    println!("cargo:rerun-if-changed=web/dist");
    println!("cargo:rerun-if-changed=web/dist/index.html");

    if !index.is_file() {
        panic!(
            "web/dist/index.html 不存在——前端产物未构建。\n\
             请先运行：pnpm -C web install && pnpm -C web build\n\
             （web/dist 被 web/.gitignore 忽略、不在版本控制中，干净 checkout 后需手动构建。\n\
             aria 二进制通过 rust-embed 在编译期嵌入 web/dist，缺失会导致运行时全站白屏。）"
        );
    }

    // 校验非空：assets 目录或至少一个资源存在
    let has_assets = std::fs::read_dir(dist)
        .map(|mut entries| entries.any(|e| e.is_ok()))
        .unwrap_or(false);
    if !has_assets {
        panic!("web/dist 为空目录——请重新运行 pnpm -C web build 生成完整前端产物。");
    }
}
```

- [ ] **Step 2: 验证 dist 存在时编译通过**

Run: `pnpm -C web build && cargo check --locked`
Expected: 编译通过（build.rs 校验 `web/dist/index.html` 存在）。

- [ ] **Step 3: 验证 dist 缺失时硬失败**

Run:
```bash
mv web/dist /tmp/aria-dist-backup && cargo check --locked 2>&1 | grep -A2 "web/dist/index.html 不存在"; mv /tmp/aria-dist-backup web/dist
```
Expected: 输出包含 panic 信息「web/dist/index.html 不存在」，`cargo check` 失败。恢复后再次 `cargo check --locked` 应通过。

> ⚠️ 该步骤临时移走 dist 验证硬失败，命令末尾已恢复。务必确认 `web/dist` 已恢复再继续。

- [ ] **Step 4: 提交**

```bash
git add build.rs
git commit -m "build: 新增 build.rs 硬校验 web/dist 存在,杜绝嵌入空目录白屏

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4：嵌入资源服务集成测试（TDD）

**Files:**
- Create: `tests/it_web/web_static_assets.rs`
- Modify: `tests/it_web.rs`（注册模块）

> 测试策略：嵌入资源在编译期固化，集成测试通过一个挂载了 fallback 的最小 router 发请求验证。先写测试看其失败（Task 5 接入 app.rs 后转绿），符合 TDD。本 Task 测试直接调用 `static_assets` 的公开接口构造 router，不依赖 app.rs 改动，故可独立先行。

- [ ] **Step 1: 注册测试模块**

在 `tests/it_web.rs` 中（与其它 `mod` 并列）加入一行：

```rust
mod web_static_assets;
```

- [ ] **Step 2: 写失败测试**

创建 `tests/it_web/web_static_assets.rs`：

```rust
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use cadence_aria::web::static_assets::{serve_static, static_dist_service};
use tower::ServiceExt;

/// 构造一个仅挂载静态 fallback 的最小 router（模拟 serve_web 的挂载方式）。
fn static_router() -> Router {
    let service = static_dist_service();
    Router::new().fallback(move |req: Request| {
        let service = service.clone();
        async move { serve_static(service, req).await }
    })
}

#[tokio::test]
async fn embedded_index_served_at_root() {
    // 确保走嵌入分支
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    let app = static_router();
    let resp = app
        .oneshot(Request::builder().method(Method::GET).uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ctype = resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
    assert!(ctype.starts_with("text/html"), "root 应返回 html，实际 {ctype}");
}

#[tokio::test]
async fn embedded_spa_fallback_for_unknown_path() {
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    let app = static_router();
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/some/spa/route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // 未命中静态文件 → SPA fallback 到 index.html
    assert_eq!(resp.status(), StatusCode::OK);
    let ctype = resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
    assert!(ctype.starts_with("text/html"), "SPA fallback 应返回 html，实际 {ctype}");
}

#[tokio::test]
async fn embedded_asset_has_correct_content_type() {
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    // index.html 中引用的 assets/*.js 必然存在；直接请求 assets 目录下任一 js
    // 通过读取嵌入清单找到一个 .js 资源路径
    let app = static_router();
    // 已知 vite 产物在 assets/ 下，请求一个不存在的 .js 仍 SPA fallback 到 html；
    // 因此本用例验证「精确命中真实存在的 index.html 返回 html」这一最强可断言点已由上面覆盖，
    // 这里改为验证显式 index.html 路径命中：
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/index.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ctype = resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
    assert!(ctype.starts_with("text/html"));
}

#[tokio::test]
async fn aria_web_dist_override_reads_from_disk() {
    use std::fs;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    fs::write(dir.path().join("index.html"), "<html>disk-override</html>").unwrap();

    unsafe { std::env::set_var("ARIA_WEB_DIST", dir.path()) };
    let app = static_router();
    let resp = app
        .oneshot(Request::builder().method(Method::GET).uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains("disk-override"),
        "设置 ARIA_WEB_DIST 后应从磁盘目录读取"
    );
}
```

> 注：测试用例间共享进程环境变量 `ARIA_WEB_DIST`，每个用例显式 set/remove；因 `cargo test` 默认多线程并发，环境变量用例（`aria_web_dist_override_reads_from_disk`）与嵌入用例可能互相干扰。**为消除竞态，Step 2 之后将这些用例合并为单个串行测试函数**（见 Step 3）。

- [ ] **Step 3: 合并为单一串行测试消除环境变量竞态**

将上面 4 个 `#[tokio::test]` 改写为**一个**测试函数，顺序执行各断言，避免 `ARIA_WEB_DIST` 跨线程污染：

```rust
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use cadence_aria::web::static_assets::{serve_static, static_dist_service};
use tower::ServiceExt;

fn static_router() -> Router {
    let service = static_dist_service();
    Router::new().fallback(move |req: Request| {
        let service = service.clone();
        async move { serve_static(service, req).await }
    })
}

async fn html_get(app: Router, uri: &str) -> axum::response::Response {
    app.oneshot(Request::builder().method(Method::GET).uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn static_assets_embedded_and_override_behavior() {
    // 1) 嵌入：根路径返回 index.html(html)
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    let resp = html_get(static_router(), "/").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/html"));

    // 2) 嵌入：未知路径 SPA fallback 到 html
    let resp = html_get(static_router(), "/some/spa/route").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/html"));

    // 3) 嵌入：显式 /index.html 命中
    let resp = html_get(static_router(), "/index.html").await;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4) 逃生口：ARIA_WEB_DIST 指向磁盘目录时改读磁盘
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>disk-override</html>").unwrap();
    unsafe { std::env::set_var("ARIA_WEB_DIST", dir.path()) };
    let resp = html_get(static_router(), "/").await;
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("disk-override"));
}
```

用此单函数版本替换 Step 2 写入的内容（删除 Step 2 的多函数版本）。

- [ ] **Step 4: 运行测试确认通过**

Run: `pnpm -C web build && cargo test --locked --test it_web static_assets_embedded_and_override_behavior`
Expected: PASS（static_assets 已在 Task 2 实现，测试应直接绿）。

> 若因 axum fallback 闭包签名不匹配而编译失败，按编译器提示微调闭包形态（fallback 接受 `Handler`）。核心断言不变。

- [ ] **Step 5: 提交**

```bash
git add tests/it_web.rs tests/it_web/web_static_assets.rs
git commit -m "test: 嵌入静态资源服务与 ARIA_WEB_DIST 逃生口集成测试

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5：app.rs 挂载点适配

**Files:**
- Modify: `src/web/app.rs`（`serve_web` 内 `:221` 附近）

- [ ] **Step 1: 改写挂载点**

在 `src/web/app.rs` 的 `serve_web` 中，将：

```rust
    let app =
        build_web_router(state).fallback_service(crate::web::static_assets::static_dist_service());
```

替换为：

```rust
    let static_service = crate::web::static_assets::static_dist_service();
    let app = build_web_router(state).fallback(move |req: axum::extract::Request| {
        let static_service = static_service.clone();
        async move { crate::web::static_assets::serve_static(static_service, req).await }
    });
```

> 从 `.fallback_service(...)`（接受 `Service`）改为 `.fallback(...)`（接受 `Handler` 闭包），以复用 Task 2 的 `serve_static` 统一分派逻辑。

- [ ] **Step 2: 全量验证**

Run:
```bash
pnpm -C web build
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
```
Expected: 全部通过；`static_assets_embedded_and_override_behavior` 绿；无 unused 警告（static_assets 现已被 app.rs 引用）。

- [ ] **Step 3: 提交**

```bash
git add src/web/app.rs
git commit -m "feat: serve_web 挂载统一嵌入静态服务(fallback handler)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6：e2e 适配 ARIA_WEB_DIST

**Files:**
- Modify: `web/e2e/start-api.mjs`

> e2e 用 `cargo run` 起后端、cwd 在仓库根。嵌入改造后，e2e 仍想用源码树的 `web/dist`（前端改动无需重编 Rust），故注入 `ARIA_WEB_DIST` 指向 `<repo>/web/dist`。

- [ ] **Step 1: 注入环境变量**

在 `web/e2e/start-api.mjs` 中，找到设置 provider 环境变量的位置：

```js
process.env.ARIA_PROVIDER_MODE = "fake";
process.env.ARIA_E2E_TEST_CONTROLS = "1";
```

在其后追加（`new URL("..", import.meta.url)` 指向 `web/` 上级即仓库根，其下 `web/dist`）：

```js
// 嵌入改造后，e2e 通过 ARIA_WEB_DIST 指向源码树 web/dist，
// 使前端改动无需重编 Rust（与 dev 工作流一致）。
process.env.ARIA_WEB_DIST = new URL("../web/dist", import.meta.url).pathname;
```

> `start-api.mjs` 位于 `web/e2e/`，`new URL("../web/dist", import.meta.url)` 解析为 `web/web/dist` 是错的——`import.meta.url` 是 `web/e2e/start-api.mjs`，`..` 到 `web/e2e/` 的上级 `web/`，需再进 `dist`。正确写法：`new URL("../dist", import.meta.url).pathname`（从 `web/e2e/` 上溯到 `web/`，再进 `dist`）。**采用 `../dist`。**

修正为：

```js
process.env.ARIA_WEB_DIST = new URL("../dist", import.meta.url).pathname;
```

- [ ] **Step 2: 验证 e2e 配置可解析（不强制跑全套 e2e）**

Run:
```bash
node -e "console.log(new URL('../dist', 'file:///repo/web/e2e/start-api.mjs').pathname)"
```
Expected: 输出 `/repo/web/dist`，确认路径解析正确。

> 若环境具备 playwright 依赖，可跑 `pnpm -C web build && pnpm -C web test:e2e` 中的一条 spec 验证仍绿（非必需，CI 会覆盖）。

- [ ] **Step 3: 提交**

```bash
git add web/e2e/start-api.mjs
git commit -m "test(e2e): 注入 ARIA_WEB_DIST 指向源码树 web/dist 适配嵌入改造

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## P1 自检（完成所有 Task 后执行）

- [ ] **嵌入服务**：`cargo test --locked --test it_web static_assets_embedded_and_override_behavior` 绿。
- [ ] **build.rs 硬失败**：临时移走 `web/dist` 后 `cargo check` 失败并提示「请先运行 pnpm -C web build」，恢复后通过。
- [ ] **全量门禁**：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked` 全绿（前置已 `pnpm -C web build`）。
- [ ] **逃生口**：`ARIA_WEB_DIST` 指向磁盘目录时改读磁盘（已由测试用例 4 覆盖）。
- [ ] **挂载点唯一**：`grep -rn "static_dist_service\|serve_static" src/` 仅 `static_assets.rs` 定义 + `app.rs` 调用。
- [ ] **e2e 路径**：`start-api.mjs` 注入 `ARIA_WEB_DIST=<repo>/web/dist`。

完成后进入 **P2**。
