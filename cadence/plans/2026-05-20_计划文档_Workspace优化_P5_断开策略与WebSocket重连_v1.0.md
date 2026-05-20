# P5: 断开策略 + WebSocket 重连 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 运行阶段刷新/关 Tab 时拦截并写入 `aborted_by_disconnect` Timeline 节点；网络抖动时自动重连，snapshot 全量替换；首次重连不闪 banner，多次失败后显示进度。

**Architecture:** 前端 `useUnloadGuard`（beforeunload + useBlocker）+ `useWorkspaceWsReconnect`（退避重连 + jitter + hidden 暂停）；后端 socket close handler 写入 `aborted_by_disconnect` 节点 + ping/pong/hello 处理。

**Tech Stack:** React + TypeScript + Rust (axum WebSocket + tokio)

**前置依赖:** P1（协议类型 `WsInMessage::Hello` / `WsOutMessage::Pong` / `WsOutMessage::SessionState` + `aborted_by_disconnect` 节点类型）

**后续 plan 消费点:**
- P7 E2E 消费断开拦截 + 重连恢复用例（E1-E5 + F1-F5）

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `web/src/hooks/useUnloadGuard.ts` | 新建 | beforeunload 拦截 + useBlocker |
| `web/src/hooks/useWorkspaceWsReconnect.ts` | 新建 | 退避重连 + jitter + hidden 暂停 |
| `web/src/components/workspace/DisconnectBanner.tsx` | 新建 | 重连 banner / 断开提示 banner |
| `src/web/workspace_ws_handler.rs` | 修改 | socket close handler 写 aborted_by_disconnect + hello/ping/pong |
| `src/product/workspace_engine.rs` | 修改 | 暴露 `append_aborted_by_disconnect` / `transition_to_prepare_context_after_disconnect` API |
| `web/src/hooks/useWorkspaceWs.ts` | 修改 | 接入重连 hook、发 hello/ping |
| `web/src/state/workspace-ws-store.ts` | 修改 | reconnect banner 状态 |

---

## 修订约束（必须优先遵守）

1. `ActiveRun` 的 abort handle 当前只存在于 `workspace_ws_handler.rs` 局部 `current_run`；P5 socket close 清理必须从该局部变量 `take()`，不得调用 engine 上不存在的 active-run API。
2. `WorkspaceEngine` 只新增断开审计和状态切回 API：`append_aborted_by_disconnect(last_active_run_id)`、`transition_to_prepare_context_after_disconnect()`；后者内部复用现有 stage/session 持久化方法。
3. 服务端 idle timeout 不发送业务 sentinel 给前端 store；send loop 内部用本地 enum 区分 `Text(String)` 与 `CloseDueToIdleTimeout`，收到 close 控制消息后调用 `ws_sender.close().await`。

### Task 1: useUnloadGuard hook

**Files:**
- 新建: `web/src/hooks/useUnloadGuard.ts`
- 测试: `web/src/hooks/useUnloadGuard.test.ts`

- [ ] **Step 1: 写 failing 测试**

```typescript
import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useUnloadGuard } from "./useUnloadGuard";

describe("useUnloadGuard", () => {
  it("registers beforeunload when enabled", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");
    renderHook(() =>
      useUnloadGuard({
        enabled: true,
        message: "运行中，离开将中止",
      })
    );
    expect(addEventListener).toHaveBeenCalledWith(
      "beforeunload",
      expect.any(Function)
    );
    addEventListener.mockRestore();
  });

  it("does not register when disabled", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");
    renderHook(() =>
      useUnloadGuard({
        enabled: false,
        message: "运行中，离开将中止",
      })
    );
    expect(addEventListener).not.toHaveBeenCalled();
    addEventListener.mockRestore();
  });
});
```

Run: `pnpm --filter web test -- useUnloadGuard`
Expected: 编译失败 — useUnloadGuard 未定义

- [ ] **Step 2: 实现 useUnloadGuard**

```typescript
import { useEffect, useRef } from "react";
import { useBlocker } from "@tanstack/react-router";

interface UseUnloadGuardOptions {
  enabled: boolean;
  message: string;
}

export function useUnloadGuard({ enabled, message }: UseUnloadGuardOptions) {
  const messageRef = useRef(message);
  messageRef.current = message;

  // 浏览器原生 beforeunload（刷新/关 Tab）
  useEffect(() => {
    if (!enabled) return;

    function handleBeforeUnload(e: BeforeUnloadEvent) {
      e.preventDefault();
      // 现代浏览器需要 returnValue
      e.returnValue = messageRef.current;
      return messageRef.current;
    }

    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
    };
  }, [enabled]);

  // 程序化导航拦截（React Router）
  useBlocker({
    condition: enabled,
    blockerFn: () => {
      const confirm = window.confirm(messageRef.current);
      return confirm;
    },
  });
}
```

注意：`useBlocker` 的 API 需要根据 `@tanstack/react-router` 版本调整。如果版本不支持 `blockerFn`，改用：

```typescript
  const blocker = useBlocker(enabled);
  useEffect(() => {
    if (blocker.state === "blocked") {
      const confirm = window.confirm(messageRef.current);
      if (confirm) {
        blocker.proceed();
      } else {
        blocker.reset();
      }
    }
  }, [blocker]);
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- useUnloadGuard`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/hooks/useUnloadGuard.ts web/src/hooks/useUnloadGuard.test.ts
git commit -m "feat(ui): add useUnloadGuard for beforeunload + programmatic nav blocker"
```

---

### Task 2: useWorkspaceWsReconnect hook

**Files:**
- 新建: `web/src/hooks/useWorkspaceWsReconnect.ts`
- 测试: `web/src/hooks/useWorkspaceWsReconnect.test.ts`

- [ ] **Step 1: 写 failing 测试**

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";

describe("useWorkspaceWsReconnect", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts with initial delay after close", () => {
    const onReconnect = vi.fn();
    renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1006,
      })
    );
    act(() => {
      vi.advanceTimersByTime(1100);
    });
    expect(onReconnect).toHaveBeenCalled();
  });

  it("pauses when document.hidden", () => {
    const onReconnect = vi.fn();
    renderHook(() =>
      useWorkspaceWsReconnect({
        enabled: true,
        onReconnect,
        closeCode: 1006,
      })
    );
    Object.defineProperty(document, "hidden", { value: true, writable: true });
    act(() => {
      document.dispatchEvent(new Event("visibilitychange"));
      vi.advanceTimersByTime(5000);
    });
    expect(onReconnect).not.toHaveBeenCalled();
  });
});
```

Run: `pnpm --filter web test -- useWorkspaceWsReconnect`
Expected: 编译失败 — useWorkspaceWsReconnect 未定义

- [ ] **Step 2: 实现 useWorkspaceWsReconnect**

```typescript
import { useEffect, useRef, useState, useCallback } from "react";

interface UseWorkspaceWsReconnectOptions {
  enabled: boolean;
  onReconnect: () => void;
  closeCode?: number;
}

const INITIAL_DELAY_MS = 1000;
const MAX_DELAY_MS = 16000;
const JITTER_PCT = 0.2;

function nextDelay(prevDelay: number): number {
  const next = Math.min(prevDelay * 2, MAX_DELAY_MS);
  const jitter = next * JITTER_PCT * (Math.random() * 2 - 1);
  return Math.max(INITIAL_DELAY_MS, Math.round(next + jitter));
}

export function useWorkspaceWsReconnect({
  enabled,
  onReconnect,
  closeCode,
}: UseWorkspaceWsReconnectOptions) {
  const [attemptCount, setAttemptCount] = useState(0);
  const [isReconnecting, setIsReconnecting] = useState(false);
  const delayRef = useRef(INITIAL_DELAY_MS);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pausedRef = useRef(false);
  const onReconnectRef = useRef(onReconnect);
  onReconnectRef.current = onReconnect;

  const clearReconnectTimeout = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const scheduleReconnect = useCallback(() => {
    clearReconnectTimeout();
    if (!enabled || pausedRef.current) return;

    setIsReconnecting(true);
    const delay = delayRef.current;
    timeoutRef.current = setTimeout(() => {
      setAttemptCount((c) => c + 1);
      onReconnectRef.current();
      delayRef.current = nextDelay(delayRef.current);
    }, delay);
  }, [enabled, clearReconnectTimeout]);

  // 触发重连
  useEffect(() => {
    if (!enabled) {
      clearReconnectTimeout();
      setIsReconnecting(false);
      return;
    }
    // 非用户主动关闭（1000 = 正常关闭）
    if (closeCode !== undefined && closeCode !== 1000) {
      scheduleReconnect();
    }
    return () => clearReconnectTimeout();
  }, [enabled, closeCode, scheduleReconnect, clearReconnectTimeout]);

  // document.hidden 暂停
  useEffect(() => {
    function handleVisibilityChange() {
      if (document.hidden) {
        pausedRef.current = true;
        clearReconnectTimeout();
      } else {
        pausedRef.current = false;
        if (enabled) {
          // 恢复时立即触发一次
          delayRef.current = INITIAL_DELAY_MS;
          scheduleReconnect();
        }
      }
    }
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () =>
      document.removeEventListener("visibilitychange", handleVisibilityChange);
  }, [enabled, scheduleReconnect, clearReconnectTimeout]);

  const reset = useCallback(() => {
    delayRef.current = INITIAL_DELAY_MS;
    setAttemptCount(0);
    setIsReconnecting(false);
    clearReconnectTimeout();
  }, [clearReconnectTimeout]);

  const retryNow = useCallback(() => {
    delayRef.current = INITIAL_DELAY_MS;
    scheduleReconnect();
  }, [scheduleReconnect]);

  return {
    isReconnecting,
    attemptCount,
    retryNow,
    reset,
  };
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- useWorkspaceWsReconnect`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/hooks/useWorkspaceWsReconnect.ts web/src/hooks/useWorkspaceWsReconnect.test.ts
git commit -m "feat(ws): add useWorkspaceWsReconnect with backoff + jitter + hidden pause"
```

---

### Task 3: DisconnectBanner 组件

**Files:**
- 新建: `web/src/components/workspace/DisconnectBanner.tsx`
- 测试: `web/src/components/workspace/DisconnectBanner.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DisconnectBanner } from "./DisconnectBanner";

describe("DisconnectBanner", () => {
  it("shows reconnect banner after multiple attempts", () => {
    render(
      <DisconnectBanner
        isReconnecting={true}
        attemptCount={2}
        onManualReconnect={vi.fn()}
      />
    );
    expect(screen.getByText(/重连中/i)).toBeInTheDocument();
    expect(screen.getByText(/手动重连/i)).toBeInTheDocument();
  });

  it("shows aborted banner when disconnected", () => {
    const onAck = vi.fn();
    render(
      <DisconnectBanner
        abortedByDisconnect={{ ts: "2026-05-20T14:32:00Z" }}
        onAcknowledge={onAck}
      />
    );
    expect(screen.getByText(/上次运行因断开被中止/i)).toBeInTheDocument();
    fireEvent.click(screen.getByText(/我知道了/i));
    expect(onAck).toHaveBeenCalled();
  });
});
```

Run: `pnpm --filter web test -- DisconnectBanner`
Expected: 编译失败 — DisconnectBanner 未定义

- [ ] **Step 2: 实现 DisconnectBanner**

```tsx
interface DisconnectBannerProps {
  isReconnecting?: boolean;
  attemptCount?: number;
  onManualReconnect?: () => void;
  abortedByDisconnect?: { ts: string } | null;
  onAcknowledge?: () => void;
  onViewTimeline?: () => void;
}

export function DisconnectBanner({
  isReconnecting,
  attemptCount,
  onManualReconnect,
  abortedByDisconnect,
  onAcknowledge,
  onViewTimeline,
}: DisconnectBannerProps) {
  if (isReconnecting && (attemptCount ?? 0) > 1) {
    return (
      <div className="flex items-center justify-between bg-amber-50 px-4 py-2 text-sm text-amber-700">
        <span>⚠️ 连接断开，正在重连...（尝试 {attemptCount} 次）</span>
        {onManualReconnect && (
          <button
            onClick={onManualReconnect}
            className="rounded bg-amber-100 px-2 py-0.5 text-xs hover:bg-amber-200"
          >
            手动重连
          </button>
        )}
      </div>
    );
  }

  if (abortedByDisconnect) {
    return (
      <div className="flex items-center justify-between bg-red-50 px-4 py-2 text-sm text-red-700">
        <span>
          ⚠️ 上次运行因断开被中止（
          {new Date(abortedByDisconnect.ts).toLocaleTimeString()}）
        </span>
        <div className="flex gap-2">
          {onViewTimeline && (
            <button
              onClick={onViewTimeline}
              className="rounded bg-red-100 px-2 py-0.5 text-xs hover:bg-red-200"
            >
              查看 Timeline
            </button>
          )}
          {onAcknowledge && (
            <button
              onClick={onAcknowledge}
              className="rounded bg-red-100 px-2 py-0.5 text-xs hover:bg-red-200"
            >
              我知道了
            </button>
          )}
        </div>
      </div>
    );
  }

  return null;
}
```

`onAcknowledge` 必须写入 localStorage，key 固定为 `aria.workspace.aborted_ack_nodes`，避免刷新后重复弹出同一条断开中止提示：

```typescript
export function loadAcknowledgedAbortedNodes(): string[] {
  try {
    const raw = window.localStorage.getItem("aria.workspace.aborted_ack_nodes");
    return raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    return [];
  }
}

export function saveAcknowledgedAbortedNode(nodeId: string) {
  const next = Array.from(new Set([...loadAcknowledgedAbortedNodes(), nodeId]));
  window.localStorage.setItem("aria.workspace.aborted_ack_nodes", JSON.stringify(next));
  return next;
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- DisconnectBanner`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/DisconnectBanner.tsx web/src/components/workspace/DisconnectBanner.test.tsx
git commit -m "feat(ui): add DisconnectBanner for reconnect progress + aborted notice"
```

---

### Task 4: 后端 socket close handler 写入 aborted_by_disconnect

**Files:**
- 修改: `src/web/workspace_ws_handler.rs`
- 修改: `src/product/workspace_engine.rs`

- [ ] **Step 1: 写 failing 测试**

```rust
#[tokio::test]
async fn socket_close_writes_aborted_by_disconnect() {
    let engine = create_test_engine().await;
    let session_id = "sess-1";
    // 模拟 active run
    engine.set_active_run("run-1").await;
    
    // 模拟 socket close
    engine.handle_socket_close(session_id).await;
    
    let nodes = engine.lifecycle_store.load_timeline_nodes(session_id).unwrap();
    let last = nodes.last().unwrap();
    assert_eq!(last.node_type, TimelineNodeType::AbortedByDisconnect);
}
```

Run: `cargo test socket_close_writes_aborted_by_disconnect -- --nocapture`
Expected: 编译失败 — handle_socket_close / set_active_run 未定义

- [ ] **Step 2: 在 workspace_engine.rs 暴露 API**

```rust
    pub async fn append_aborted_by_disconnect(
        &mut self,
        last_active_run_id: String,
    ) -> Result<TimelineNode, WorkspaceEngineError> {
        let node = TimelineNode {
            node_id: generate_id(),
            node_type: TimelineNodeType::AbortedByDisconnect,
            agent: None,
            stage: self.session.stage.clone(),
            round: None,
            status: TimelineNodeStatus::Failed,
            title: "运行因断开中止".to_string(),
            summary: Some(format!("last_active_run_id: {}", last_active_run_id)),
            started_at: now_iso(),
            completed_at: Some(now_iso()),
            duration_ms: Some(0),
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: self.session.author_provider.clone(),
                reviewer: self.session.reviewer_provider.clone(),
                review_rounds: self.session.review_rounds.unwrap_or(0),
            },
        };
        self.timeline_nodes.push(node.clone());
        if let Some(store) = &self.lifecycle_store {
            store.save_timeline_nodes(
                &self.session.session_id,
                &self.timeline_nodes,
            )?;
        }
        Ok(node)
    }

    pub async fn transition_to_prepare_context_after_disconnect(&mut self) {
        self.transition_stage(WorkspaceStage::PrepareContext).await;
        self.clear_active_run_id();
    }
```

- [ ] **Step 3: 在 workspace_ws_handler.rs 修改 socket close 处理**

当前 `handle_workspace_socket` 末尾已经有局部 `current_run` 清理逻辑。将该清理扩展为断开审计，不新增 engine 上的 active-run take API：

```rust
    // handle_workspace_socket 末尾：while receive loop 退出后
    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let last_active_run_id = run.id.to_string();
        let _ = run.command_tx.send(ProviderCommand::Abort).await;
        run.cancel.cancel();

        let mut engine = engine.lock().await;
        let _ = engine.append_aborted_by_disconnect(last_active_run_id).await;
        engine.transition_to_prepare_context_after_disconnect().await;

        let state_msg = engine.build_session_state();
        if let Ok(json) = serde_json::to_string(&state_msg) {
            let _ = outbound_tx.send(json).await;
        }
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test socket_close_writes_aborted_by_disconnect -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/web/workspace_ws_handler.rs src/product/workspace_engine.rs
git commit -m "feat(ws): write aborted_by_disconnect node on socket close"
```

---

### Task 5: 后端 hello / ping / pong 处理

**Files:**
- 修改: `src/web/workspace_ws_handler.rs`

- [ ] **Step 1: 在 handle_message 中追加 hello/ping 处理**

```rust
            WsInMessage::Hello { session_id, last_seen_node_id } => {
                let state = engine.build_session_state();
                let _ = socket.send(Message::Text(serde_json::to_string(&state).unwrap())).await;
            }
            WsInMessage::Ping => {
                let _ = socket.send(Message::Text(serde_json::to_string(&WsOutMessage::Pong).unwrap())).await;
            }
```

- [ ] **Step 1.5: 服务端 90s 无客户端消息主动 close**

在 socket receive loop 中维护 `last_client_message_at`，每次收到任意合法客户端消息都刷新；另起 task 每 5s 检查一次，超过 90s 后发送 close，触发 §6 的 socket close 清理路径：

```rust
enum OutboundControl {
    Text(String),
    CloseDueToIdleTimeout,
}

let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
let last_seen_for_timeout = last_client_message_at.clone();
let close_tx = outbound_tx.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        let last_seen = *last_seen_for_timeout.lock().await;
        if last_seen.elapsed() > std::time::Duration::from_secs(90) {
            let _ = close_tx.send(OutboundControl::CloseDueToIdleTimeout).await;
            break;
        }
    }
});
```

发送任务收到 `OutboundControl::CloseDueToIdleTimeout` 后调用 `ws_sender.close().await`，不要序列化为业务消息。普通业务消息走 `OutboundControl::Text(json)`。收到 `Ping` 时刷新 `last_client_message_at` 并回 `Pong`。

- [ ] **Step 2: 跑测试确认通过**

Run: `cargo test --locked -j 1`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(ws): handle hello and ping inbound messages"
```

---

### Task 6: 前端接入重连 + 断开拦截

**Files:**
- 修改: `web/src/hooks/useWorkspaceWs.ts`
- 修改: `web/src/pages/WorkspacePage.tsx`
- 修改: `web/src/state/workspace-ws-store.ts`

- [ ] **Step 1: 在 useWorkspaceWs.ts 接入重连**

```typescript
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const lastMessageAtRef = useRef(Date.now());
  const [closeCode, setCloseCode] = useState<number | undefined>();
  // ...

  const { isReconnecting, attemptCount, retryNow, reset: resetReconnect } = useWorkspaceWsReconnect({
    enabled: connectionStatus === "disconnected" && !!sessionId,
    onReconnect: () => {
      connect(); // 重新连接
    },
    closeCode,
  });

  function connect() {
    if (!sessionId) return;
    const ws = new WebSocket(url);
    wsRef.current = ws;
    setCloseCode(undefined);

    ws.onopen = () => {
      store.setConnectionStatus("connected");
      lastMessageAtRef.current = Date.now();
      resetReconnect();
      // 发送 hello
      ws.send(JSON.stringify({
        type: "hello",
        session_id: sessionId,
      }));
    };

    ws.onmessage = (event) => {
      lastMessageAtRef.current = Date.now();
      handleRawMessage(event);
    };

    ws.onclose = (e) => {
      setCloseCode(e.code);
      store.setConnectionStatus("disconnected");
    };
    // ...
  }

  // 心跳
  useEffect(() => {
    if (connectionStatus !== "connected") return;
    const interval = setInterval(() => {
      if (Date.now() - lastMessageAtRef.current > 60000) {
        wsRef.current?.close();
        return;
      }
      sendPing();
    }, 25000);
    return () => clearInterval(interval);
  }, [connectionStatus]);

  // ...
	  return {
	    // ...
	    isReconnecting,
	    reconnectAttemptCount: attemptCount,
	    retryNow,
	  };
}
```

- [ ] **Step 2: 在 WorkspacePage.tsx 接入 useUnloadGuard + DisconnectBanner**

```tsx
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { DisconnectBanner } from "../components/workspace/DisconnectBanner";

function WorkspacePage({ sessionId }: WorkspacePageProps) {
  const store = useWorkspaceStore();
	  const { isReconnecting, reconnectAttemptCount, retryNow } = useWorkspaceWs(sessionId);

  useUnloadGuard({
    enabled: ["running", "cross_review", "revision"].includes(store.stage),
    message: "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？",
  });

  // 检查最后一个节点是否是 aborted_by_disconnect
  const lastNode = store.timelineNodes[store.timelineNodes.length - 1];
  const showAbortedBanner =
    lastNode?.node_type === "aborted_by_disconnect" &&
    !store.acknowledgedAbortedNodes?.includes(lastNode.node_id);

  return (
    <div className="flex h-full flex-col">
      <DisconnectBanner
        isReconnecting={isReconnecting}
        attemptCount={reconnectAttemptCount}
	        onManualReconnect={retryNow}
        abortedByDisconnect={
          showAbortedBanner
            ? { ts: lastNode.started_at }
            : undefined
        }
        onAcknowledge={() => {
	          const acknowledged = saveAcknowledgedAbortedNode(lastNode.node_id);
	          useWorkspaceStore.setState({ acknowledgedAbortedNodes: acknowledged });
	        }}
      />
      {/* ... 其余布局 ... */}
    </div>
  );
}
```

- [ ] **Step 3: 在 store 中追加 acknowledgedAbortedNodes**

```typescript
export interface WorkspaceWsState {
  // ... 现有 ...
  acknowledgedAbortedNodes: string[];
}
```

初始值：`acknowledgedAbortedNodes: []`

- [ ] **Step 4: 跑前端测试确认通过**

Run: `pnpm --filter web test -- WorkspacePage`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add web/src/hooks/useWorkspaceWs.ts web/src/pages/WorkspacePage.tsx web/src/state/workspace-ws-store.ts
git commit -m "feat(ui): integrate unload guard + reconnect + disconnect banners"
```

---

### Task 7: 全量回归测试

- [ ] **Step 1: 跑后端测试**

Run: `cargo test --locked -j 1`
Expected: PASS

- [ ] **Step 2: 跑前端测试**

Run: `pnpm --filter web test`
Expected: PASS

- [ ] **Step 3: Commit（如有修复）**

```bash
git commit -am "fix: adjust tests for disconnect + reconnect features"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 § | 实现位置 |
|---|---|
| §7.1 端到端事件流 | Task 4 (后端) + Task 6 (前端 banner) |
| §7.2 beforeunload 拦截 | Task 1 (useUnloadGuard) |
| §7.3 后端 aborted_by_disconnect | Task 4 (socket close handler) |
| §7.4 重连后明示 | Task 3 (DisconnectBanner) + Task 6 (store ack) |
| §7.5 主动中止 vs 断开中止 | Task 4 (仅 socket close 触发) |
| §8.2 重连策略 | Task 2 (useWorkspaceWsReconnect) |
| §8.3 UI 反馈 | Task 3 (banner) |
| §8.4 snapshot 全量替换 | Task 5 (hello 回送 SessionState) |
| §8.6 心跳与超时 | Task 6 (25s ping) |

**2. Implementation constraints:**
- 没有待定占位项

**3. Type consistency:**
- `acknowledgedAbortedNodes` 在 store 中用 string[]，并同步到 localStorage key `aria.workspace.aborted_ack_nodes`

---

## 本 plan 验收清单

- [ ] Running 阶段刷新页面：beforeunload 拦截弹出确认
- [ ] 用户确认离开：后端写入 `aborted_by_disconnect` 节点
- [ ] 重连后 UI 顶部 banner："上次运行因断开被中止"
- [ ] 点击"我知道了"后 localStorage 标记，再刷新不再弹
- [ ] PrepareContext 刷新不拦截、不写节点
- [ ] WebSocket 断开后 1s 开始自动重连，退避递增
- [ ] 重连失败 >1 次显示进度 banner + 手动重连按钮
- [ ] 心跳每 25s 发送 ping
- [ ] 客户端 60s 无消息主动 close 并进入重连
- [ ] 服务端 90s 无客户端消息主动 close 并触发断开中止路径
- [ ] `cargo test --locked -j 1` PASS
- [ ] `pnpm --filter web test` PASS
