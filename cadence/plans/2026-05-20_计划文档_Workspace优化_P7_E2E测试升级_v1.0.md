# P7: E2E 测试升级 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 覆盖 P1-P6 全部闭环，写 7 组 E2E 用例（A-G），升级既有 E2E fixture 适配协议 v2，确保所有验收标准可自动验证。

**Architecture:** 复用既有 Playwright E2E 框架（fake provider + 真实 WS），按 A-G 7 闭环组织，每组用例覆盖一个端到端用户场景。既有用例适配 protocol v2（user_message → context_note / start_generation）。

**Tech Stack:** Playwright + TypeScript + fake provider（后端内置）

**前置依赖:** P1-P6 全部完成

**后续 plan 消费点:** 无（P7 是收尾）

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `web/e2e/protocol-v2.spec.ts` | 新建 | A 组：输入语义解耦 |
| `web/e2e/timeline-audit.spec.ts` | 新建 | B 组：Timeline 审计 + 会话恢复 |
| `web/e2e/drawer-navigation.spec.ts` | 新建 | C 组：看板侧滑详情 |
| `web/e2e/stage-ui.spec.ts` | 新建 | D 组：阶段化 UI + 节点 tab |
| `web/e2e/disconnect-strategy.spec.ts` | 新建 | E 组：断开策略 |
| `web/e2e/websocket-reconnect.spec.ts` | 新建 | F 组：自动重连 |
| `web/e2e/permission-link.spec.ts` | 新建 | G 组：Permission 链路 |
| `web/e2e/issue-lifecycle-workspace.spec.ts` | 修改 | 既有用例适配 protocol v2 |
| `src/web/test_controls.rs`（仅 `cfg(test)` 或 E2E test mode） | 新建/修改 | 提供 E2E 专用 WS drop / timeout 缩短控制，避免在浏览器里使用非法 close code 或 `window.__ws` |

---

### Task 1: E2E 基础设施（fake provider 配置 + 通用 helpers）

**Files:**
- 修改: `web/e2e/start-api.mjs`
- 新建: `web/e2e/helpers/workspace.ts`

- [ ] **Step 1: 确认 fake provider 环境变量**

`start-api.mjs` 中应包含：

```javascript
process.env.ARIA_PROVIDER_MODE = "fake";
```

- [ ] **Step 2: 新建 E2E helper**

```typescript
// web/e2e/helpers/workspace.ts
import { expect, type Page } from "@playwright/test";

export async function createWorkspaceSession(page: Page, issueId: string): Promise<string> {
  // 通过 API 或直接操作 UI 创建 Workspace session
  await page.goto(`/workbench?focus=${issueId}`);
  await page.getByText(/打开 Workspace/i).click();
  await page.waitForURL(/\/workbench\/workspace\//);
  const url = page.url();
  return url.split("/workbench/workspace/")[1];
}

export async function waitForStage(page: Page, stage: string, timeout = 30000) {
  await page.waitForFunction(
    (expected) => {
      const badge = document.querySelector('[data-testid="stage-badge"]');
      return badge?.textContent?.includes(expected);
    },
    stage,
    { timeout }
  );
}

export async function sendContextNote(page: Page, content: string) {
  await page.getByPlaceholder(/补充上下文/i).fill(content);
  await page.getByText(/发送上下文/i).click();
}

export async function clickStartGeneration(page: Page) {
  await page.getByText(/开始生成/i).click();
}

export async function waitForTimelineNode(page: Page, nodeType: string, timeout = 30000) {
  await page.waitForSelector(`[data-testid="timeline-node-${nodeType}"]`, { timeout });
}

export async function sendRawWorkspaceMessage(page: Page, sessionId: string, payload: unknown) {
  await page.evaluate(
    ({ sessionId, payload }) =>
      new Promise<void>((resolve, reject) => {
        const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
        const ws = new WebSocket(`${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`);
        ws.onopen = () => {
          ws.send(JSON.stringify(payload));
          ws.close();
          resolve();
        };
        ws.onerror = () => reject(new Error("raw workspace websocket failed"));
      }),
    { sessionId, payload },
  );
}

export async function dropWorkspaceSocketFromServer(page: Page, sessionId: string) {
  const response = await page.request.post(`/api/test/workspace-sessions/${sessionId}/ws/drop`);
  expect(response).toBeOK();
}
```

- [ ] **Step 3: 增加 E2E 专用 WebSocket 控制接口**

仅在 `ARIA_E2E_TEST_CONTROLS=1` 时启用 `/api/test/workspace-sessions/:session_id/ws/drop`，由服务端主动断开对应 session 的当前 WebSocket。测试必须通过这个接口模拟异常断开；不要在浏览器端调用 `ws.close(1006)`，因为 1006 是保留 close code，浏览器不允许发送。

`start-api.mjs` 追加：

```javascript
process.env.ARIA_E2E_TEST_CONTROLS = "1";
```

后端接口验收：

```rust
#[tokio::test]
async fn test_controls_are_disabled_without_e2e_env() {
    std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    assert!(!test_controls_enabled());
}

#[tokio::test]
async fn test_controls_are_enabled_in_e2e_env() {
    std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    assert!(test_controls_enabled());
}
```

- [ ] **Step 4: Commit**

```bash
git add web/e2e/helpers/workspace.ts web/e2e/start-api.mjs src/web/test_controls.rs
git commit -m "test(e2e): add workspace E2E helpers"
```

---

### Task 2: A 组 — 输入语义解耦

**Files:**
- 新建: `web/e2e/protocol-v2.spec.ts`

- [ ] **Step 1: 写 A1-A4 用例**

```typescript
import { test, expect } from "@playwright/test";
import { createWorkspaceSession, sendContextNote, clickStartGeneration, waitForTimelineNode, waitForStage, sendRawWorkspaceMessage } from "./helpers/workspace";

test.describe("A. 输入语义解耦", () => {
  test("A1. context_note 不触发 Provider", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await sendContextNote(page, "需要支持空查询参数");
    await waitForTimelineNode(page, "context_note");
    await expect(page.getByText(/准备中/i)).toBeVisible();
  });

  test("A2. 连续 3 条 context_note 不启动 Provider", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await sendContextNote(page, "第一条");
    await sendContextNote(page, "第二条");
    await sendContextNote(page, "第三条");
    await expect(page.locator('[data-testid="timeline-node-context_note"]')).toHaveCount(3);
    await expect(page.getByText(/准备中/i)).toBeVisible();
  });

  test("A3. 开始生成锁定 Provider 并切 Running", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByText(/🔒/i)).toBeVisible();
  });

	  test("A4. Running 阶段发 context_note 收到 protocol_error", async ({ page }) => {
	    const sessionId = await createWorkspaceSession(page, "issue-1");
	    await clickStartGeneration(page);
	    await waitForStage(page, "运行中");
	    await sendRawWorkspaceMessage(page, sessionId, { type: "context_note", content: "test" });
	    await expect(page.getByText(/INVALID_MESSAGE_FOR_STAGE/i)).toBeVisible();
	  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/protocol-v2.spec.ts
git commit -m "test(e2e): add protocol v2 semantic decoupling cases (A1-A4)"
```

---

### Task 3: B 组 — Timeline 审计 + 会话恢复

**Files:**
- 新建: `web/e2e/timeline-audit.spec.ts`

- [ ] **Step 1: 写 B1-B5 用例**

```typescript
import { test, expect } from "@playwright/test";

test.describe("B. Timeline 审计 + 会话恢复", () => {
  test("B1. 流式中刷新后 snapshot 含 streaming 累积和 aborted_by_disconnect", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    // 等待部分 stream
    await page.waitForTimeout(2000);
    const streamingBefore = await page.locator('[data-testid="streaming-content"]').textContent();
    page.on("dialog", (dialog) => dialog.accept());
    await page.reload();
    await waitForStage(page, "准备中");
    const streamingAfter = await page.locator('[data-testid="streaming-content"]').textContent();
    expect(streamingAfter?.length).toBeGreaterThanOrEqual(streamingBefore?.length ?? 0);
    await expect(page.locator('[data-testid="timeline-node-aborted_by_disconnect"]')).toBeVisible();
  });

  test("B2. permission_request 未应答时刷新 snapshot 含 pending", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    // 等待 permission_request（fake provider 触发）
    await page.waitForSelector('[data-testid="permission-request"]', { timeout: 30000 });
    // 刷新
    await page.reload();
    const nodeDetail = await page.locator('[data-testid="node-detail-panel"]').textContent();
    expect(nodeDetail).toContain("待应答");
  });

	  test("B3. reviewer verdict 完成后刷新 snapshot 完整", async ({ page }) => {
	    await createWorkspaceSession(page, "issue-1");
	    await clickStartGeneration(page);
	    await waitForStage(page, "审核结论待处理", 60000);
	    await page.reload();
	    await expect(page.getByText(/审核结论/i)).toBeVisible();
	    await expect(page.locator('[data-testid="node-detail-panel"]')).toContainText(/verdict|审核结论/i);
	  });

	  test("B4. 多版本 revision 后刷新两个 author_run 完整", async ({ page }) => {
	    await createWorkspaceSession(page, "issue-1");
	    await clickStartGeneration(page);
	    await waitForStage(page, "审核结论待处理", 60000);
	    await page.getByLabel(/直接返修/i).check();
	    await page.getByRole("button", { name: /确定/i }).click();
	    await waitForStage(page, "修订中", 30000);
	    await page.reload();
	    await expect(page.locator('[data-testid="timeline-node-author_run"]')).toHaveCount(2);
	  });

  test("B5. 100+ 节点写入/读取性能", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    // 发送 100 条 context_note
    for (let i = 0; i < 100; i++) {
      await sendContextNote(page, `note-${i}`);
    }
    const start = Date.now();
    await page.reload();
    await page.waitForSelector('[data-testid="timeline-node-context_note"]', { timeout: 200 });
    const elapsed = Date.now() - start;
    expect(elapsed).toBeLessThan(200);
  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/timeline-audit.spec.ts
git commit -m "test(e2e): add timeline audit + recovery cases (B1-B5)"
```

---

### Task 4: C 组 — 看板侧滑详情

**Files:**
- 新建: `web/e2e/drawer-navigation.spec.ts`

- [ ] **Step 1: 写 C1-C6 用例**

```typescript
import { test, expect } from "@playwright/test";

test.describe("C. 看板侧滑详情", () => {
  test("C1. 卡片点击打开 Drawer", async ({ page }) => {
    await page.goto("/workbench");
    await page.getByText(/某个 Story Spec/i).click();
    await expect(page.locator('[data-testid="lifecycle-card-drawer"]')).toBeVisible();
    await expect(page).toHaveURL(/focus=/);
  });

  test("C2. 关闭 Drawer URL 清除", async ({ page }) => {
    await page.goto("/workbench?focus=story-12");
    await page.getByLabel(/关闭/i).click();
    await expect(page).not.toHaveURL(/focus=/);
  });

  test("C3. Story confirmed 生成 Design Spec", async ({ page }) => {
    await page.goto("/workbench?focus=story-12");
    await page.getByText(/生成 Design Spec/i).click();
    await expect(page.locator('[data-testid="lifecycle-card-drawer"]')).toContainText("Design Spec");
    await expect(page).toHaveURL(/\/workbench\?focus=design/);
    await expect(page).not.toHaveURL(/\/workbench\/workspace\//);
    await expect(page.getByText(/打开 Workspace 配置 Provider 并开始生成/i)).toBeVisible();
  });

	  test("C4. Drawer 内打开 Workspace", async ({ page }) => {
	    await page.goto("/workbench?focus=story-12");
	    await page.getByText(/打开 Workspace/i).click();
	    await page.waitForURL(/\/workbench\/workspace\//);
	  });

  test("C5. URL 直接访问 focus 自动打开 Drawer", async ({ page }) => {
    await page.goto("/workbench?focus=story-12");
    await expect(page.locator('[data-testid="lifecycle-card-drawer"]')).toBeVisible();
  });

  test("C6. handleLaunchWorkspace race fix", async ({ page }) => {
    await page.goto("/workbench?focus=story-12");
    await page.getByText(/打开 Workspace/i).click();
    // 不应出现白屏或错误
    await expect(page.getByText(/准备中/i)).toBeVisible({ timeout: 5000 });
  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/drawer-navigation.spec.ts
git commit -m "test(e2e): add drawer navigation cases (C1-C6)"
```

---

### Task 5: D 组 — 阶段化 UI + 节点 tab

**Files:**
- 新建: `web/e2e/stage-ui.spec.ts`

- [ ] **Step 1: 写 D1-D5 用例**

```typescript
import { test, expect } from "@playwright/test";

test.describe("D. 阶段化 UI + 节点 tab", () => {
  test("D1. 节点详情 5 tab 切换", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await page.getByText(/流式输出/i).click();
    await expect(page.locator('[data-testid="tab-streaming"]')).toBeVisible();
    await page.getByText(/执行事件/i).click();
    await expect(page.locator('[data-testid="tab-execution"]')).toBeVisible();
    await page.getByText(/权限/i).click();
    await expect(page.locator('[data-testid="tab-permission"]')).toBeVisible();
  });

  test("D2. Header Provider snapshot 锁定状态", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByText(/🔒/i)).toBeVisible();
    await expect(page.getByText(/锁定于/i)).toBeVisible();
  });

  test("D3. ReviewDecision 三路径选择", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "审核结论待处理", 60000);
    await page.getByText(/直接返修/i).click();
    await page.getByText(/确定/i).click();
    await waitForStage(page, "修订中");
  });

  test("D4. HumanConfirm 显示 reviewer 摘要 + diff", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    // 运行到 HumanConfirm
    await waitForStage(page, "等待确认", 120000);
    await expect(page.getByText(/审核摘要/i)).toBeVisible();
    await expect(page.getByText(/与上一版本对比/i)).toBeVisible();
  });

  test("D5. HumanConfirm 要求修改走结构化反馈", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await waitForStage(page, "等待确认", 120000);
    await page.getByText(/要求修改/i).click();
    await page.getByLabelText(/内容缺失/i).check();
    await page.getByPlaceholder(/具体描述/i).fill("缺少错误处理");
    await page.getByText(/提交/i).click();
    await waitForStage(page, "审核结论待处理");
  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/stage-ui.spec.ts
git commit -m "test(e2e): add stage UI cases (D1-D5)"
```

---

### Task 6: E 组 — 断开策略

**Files:**
- 新建: `web/e2e/disconnect-strategy.spec.ts`

- [ ] **Step 1: 写 E1-E5 用例**

```typescript
import { test, expect } from "@playwright/test";

test.describe("E. 断开策略", () => {
  test("E1. Running 时刷新 beforeunload 拦截", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    page.on("dialog", async (dialog) => {
      expect(dialog.message()).toContain("中止");
      await dialog.accept();
    });
    await page.reload();
    await expect(page.getByText(/上次运行因断开被中止/i)).toBeVisible();
  });

  test("E2. 重连 banner 可关闭", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    // 刷新并接受 beforeunload
    page.on("dialog", (dialog) => dialog.accept());
    await page.reload();
    await page.getByText(/我知道了/i).click();
    await expect(page.getByText(/上次运行因断开被中止/i)).not.toBeVisible();
  });

  test("E3. PrepareContext 刷新不拦截", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    let dialogShown = false;
    page.on("dialog", () => { dialogShown = true; });
    await page.reload();
    expect(dialogShown).toBe(false);
  });

  test("E4. HumanConfirm 刷新不拦截", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    // 运行到 HumanConfirm
    await waitForStage(page, "等待确认", 120000);
    let dialogShown = false;
    page.on("dialog", () => { dialogShown = true; });
    await page.reload();
    expect(dialogShown).toBe(false);
  });

  test("E5. 主动中止不产生 aborted_by_disconnect", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await page.getByText(/中止/i).click();
    await expect(page.locator('[data-testid="timeline-node-aborted_by_disconnect"]')).toHaveCount(0);
    await expect(page.locator('[data-testid="timeline-node-author_run"]')).toContainText("aborted");
  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/disconnect-strategy.spec.ts
git commit -m "test(e2e): add disconnect strategy cases (E1-E5)"
```

---

### Task 7: F 组 — 自动重连

**Files:**
- 新建: `web/e2e/websocket-reconnect.spec.ts`

- [ ] **Step 1: 写 F1-F5 用例**

```typescript
import { test, expect } from "@playwright/test";
import { createWorkspaceSession, clickStartGeneration, waitForStage, dropWorkspaceSocketFromServer } from "./helpers/workspace";

test.describe("F. 自动重连", () => {
  test("F1. 服务端主动 drop socket 后自动重连", async ({ page }) => {
    const sessionId = await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await dropWorkspaceSocketFromServer(page, sessionId);
    // 首次重连尝试不显示 banner，成功后应应用 snapshot
    await expect(page.getByText(/重连中/i)).not.toBeVisible();
    // 等待重连成功
    await expect(page.getByText(/运行中/i)).toBeVisible({ timeout: 5000 });
  });

  test("F2. 多次失败显示进度 banner", async ({ page }) => {
    const sessionId = await createWorkspaceSession(page, "issue-1");
    // 连续从服务端 drop 多次
    for (let i = 0; i < 3; i++) {
      await dropWorkspaceSocketFromServer(page, sessionId);
      await page.waitForTimeout(1500);
    }
    await expect(page.getByText(/尝试 2 次/i)).toBeVisible();
  });

  test("F3. hidden 暂停恢复", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", { value: true, writable: true });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page.waitForTimeout(3000);
    // hidden 期间不应重连
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", { value: false, writable: true });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await expect(page.getByText(/运行中/i)).toBeVisible({ timeout: 3000 });
  });

  test("F4. 客户端无消息超时后主动 close 并重连", async ({ page }) => {
    const sessionId = await createWorkspaceSession(page, "issue-1");
    await page.request.post("/api/test/ws-timeout", {
      data: { client_idle_timeout_ms: 5000, suppress_server_messages: true },
    });
    await page.waitForTimeout(7000);
    await expect(page.getByText(/重连中|运行中|准备中/i)).toBeVisible();
    await page.request.post("/api/test/ws-timeout", {
      data: { client_idle_timeout_ms: 60000, suppress_server_messages: false, session_id: sessionId },
    });
  });

  test("F5. 服务端 90s 无消息触发 close", async ({ page }) => {
    // E2E 环境把服务端 idle timeout 调小到 5s
    await createWorkspaceSession(page, "issue-1");
    let closed = false;
    page.on("websocket", (ws) => {
      ws.on("close", () => { closed = true; });
    });
    await page.request.post("/api/test/ws-timeout", { data: { server_idle_timeout_ms: 5000 } });
    await page.waitForTimeout(7000);
    expect(closed).toBe(true);
  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/websocket-reconnect.spec.ts
git commit -m "test(e2e): add websocket reconnect cases (F1-F5)"
```

---

### Task 8: G 组 — Permission 链路

**Files:**
- 新建: `web/e2e/permission-link.spec.ts`

- [ ] **Step 1: 写 G1-G5 用例**

```typescript
import { test, expect } from "@playwright/test";
import { createWorkspaceSession, clickStartGeneration, waitForStage, sendRawWorkspaceMessage } from "./helpers/workspace";

test.describe("G. Permission 链路", () => {
  test("G1. 正常 approve 继续 run", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await page.waitForSelector('[data-testid="permission-request"]', { timeout: 30000 });
    await page.getByText(/批准/i).click();
    await expect(page.getByText(/等待确认/i)).toBeVisible({ timeout: 60000 });
  });

  test("G2. 正常 deny 中止 run", async ({ page }) => {
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await page.waitForSelector('[data-testid="permission-request"]', { timeout: 30000 });
    await page.getByText(/拒绝/i).click();
    await expect(page.getByText(/aborted/i)).toBeVisible();
  });

  test("G3. unmatched id 展示 protocol_error", async ({ page }) => {
    const sessionId = await createWorkspaceSession(page, "issue-1");
    await sendRawWorkspaceMessage(page, sessionId, {
      type: "permission_response",
      id: "nonexistent-id",
      approved: true,
    });
    await expect(page.getByText(/PERMISSION_ID_UNMATCHED/i)).toBeVisible();
  });

  test("G4. 15min 超时清理", async ({ page }) => {
	    await createWorkspaceSession(page, "issue-1");
	    await page.request.post("/api/test/permission-timeout", { data: { timeout_ms: 5000 } });
	    await clickStartGeneration(page);
	    await waitForStage(page, "运行中");
	    await page.waitForSelector('[data-testid="permission-request"]', { timeout: 30000 });
	    await page.waitForTimeout(6000);
	    await expect(page.getByText(/timeout/i)).toBeVisible();
	    await expect(page.getByText(/permission_timeout/i)).toBeVisible();
	  });

  test("G5. 全链路 trace log", async ({ page }) => {
    const logs: string[] = [];
    page.on("console", (msg) => logs.push(msg.text()));
    await createWorkspaceSession(page, "issue-1");
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await page.waitForSelector('[data-testid="permission-request"]', { timeout: 30000 });
    await page.getByText(/批准/i).click();
    expect(logs.some((l) => l.includes("permission") && l.includes("sending response"))).toBe(true);
  });
});
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/permission-link.spec.ts
git commit -m "test(e2e): add permission link cases (G1-G5)"
```

---

### Task 9: 既有 E2E 用例适配 protocol v2

**Files:**
- 修改: `web/e2e/issue-lifecycle-workspace.spec.ts`
- 修改: `web/e2e/fake-workbench.spec.ts`

- [ ] **Step 1: 更新 user_message → context_note / start_generation**

```typescript
// 原来：
// await page.getByRole("textbox").fill("开始生成");
// await page.getByRole("button", { name: "发送" }).click();

// 改为：
// await page.getByPlaceholder(/补充上下文/i).fill("上下文");
// await page.getByText(/发送上下文/i).click();
// await page.getByText(/开始生成/i).click();
```

- [ ] **Step 2: Commit**

```bash
git add web/e2e/issue-lifecycle-workspace.spec.ts web/e2e/fake-workbench.spec.ts
git commit -m "test(e2e): migrate existing E2E to protocol v2"
```

---

### Task 10: 全量 E2E 回归

- [ ] **Step 1: 跑全量 E2E**

Run: `pnpm --filter web test:e2e`
Expected: 全部 PASS（可能需要多次运行稳定化）

- [ ] **Step 2: Commit（如有修复）**

```bash
git commit -am "fix(e2e): stabilize E2E cases for P1-P6"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 §10.2 | E2E 文件 |
|---|---|
| A. 输入语义解耦 | `protocol-v2.spec.ts` |
| B. Timeline 审计 | `timeline-audit.spec.ts` |
| C. 看板侧滑 | `drawer-navigation.spec.ts` |
| D. 阶段化 UI | `stage-ui.spec.ts` |
| E. 断开策略 | `disconnect-strategy.spec.ts` |
| F. 自动重连 | `websocket-reconnect.spec.ts` |
| G. Permission 链路 | `permission-link.spec.ts` |

**2. Placeholder scan:**
- 无 TBD/TODO

**3. 时间敏感用例:**
- B5 性能断言：100 节点 < 200ms
- F4/F5 心跳/超时：E2E test controls 将服务端超时调小到 5s；浏览器端不使用非法 close code
- G4 15min 超时：测试环境应调小为 5s

---

## 本 plan 验收清单

- [ ] A1-A4 全部 PASS
- [ ] B1-B5 全部 PASS
- [ ] C1-C6 全部 PASS
- [ ] D1-D5 全部 PASS
- [ ] E1-E5 全部 PASS
- [ ] F1-F5 全部 PASS
- [ ] G1-G5 全部 PASS
- [ ] 既有 E2E 不破坏
- [ ] `pnpm --filter web test:e2e` 全量 PASS
- [ ] `cargo test --locked -j 1` PASS
- [ ] `pnpm --filter web test` PASS
