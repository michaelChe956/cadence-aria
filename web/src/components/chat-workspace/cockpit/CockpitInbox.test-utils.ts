import { vi } from "vitest";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";

// 从 CockpitInbox.test.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 收件箱测试共用的 facade mock 工厂。
export function mockActions(): CockpitActionFacade {
  return {
    confirm: vi.fn(),
    confirmReview: vi.fn(),
    feedback: vi.fn(),
    terminate: vi.fn(),
    advance: vi.fn(),
    adoptReview: vi.fn(),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    resumeRepositoryInitialization: vi.fn(async () => undefined),
    sendBootstrapAction: vi.fn(async () => undefined),
  };
}
