// 票3 片6 → 2026-11-04 迁移：逻辑代码库管理面板**不再**承载 provider 能力
// 核验区（owner 裁定能力核验已全局化 ⇒ provider 管理搬去独立的「Provider 管理」
// 面板）。本文件断言：
// - LC 面板【不渲染】provider 核验区（能力卡 / 行级列表 / 「一键核验全部」按钮 /
//   作用域说明一个都不在）；
// - 原位置留一行只读说明 + 「去 Provider 管理」入口，点击即打开独立面板；
// - 「一键核验全部」按钮在能力卡头部（上方）+ 全局核验口径说明的这组断言迁到
//   ProviderManagementPanel.test.tsx（宿主换成 Provider 管理面板，断言不弱化）。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LogicalCodebaseManagementPanel } from "./LogicalCodebaseManagementPanel";
import type { CodebaseSummaryDto } from "../../api/types";

const PROJECT_ID = "project_0001";
const LC_ID = "logical_codebase_0001";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** 面板子组件挂载即 GET：给空投影即可（本面板已不含能力卡）。 */
function stubFetch(): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input.toString();
      if (url.endsWith("/capabilities")) {
        return jsonResponse({
          providers: [
            {
              provider_type: "claude_code",
              cli_program: "claude",
              version: null,
              probed_at: null,
              probe_artifact_ref: null,
              rows: [],
            },
          ],
        });
      }
      if (url === "/api/providers/install-targets") {
        return jsonResponse({ targets: [] });
      }
      return jsonResponse(
        { code: "not_found", message: `no route for ${url}` },
        404,
      );
    }),
  );
}

const logicalCodebases: CodebaseSummaryDto[] = [
  {
    id: LC_ID,
    name: "lc-one",
    kind: "logical",
    repository_id: null,
    logical_codebase_id: LC_ID,
    member_count: 1,
  },
];

function renderPanel(onOpenProviderManagement: () => void = () => {}) {
  return render(
    <LogicalCodebaseManagementPanel
      logicalCodebases={logicalCodebases}
      projectId={PROJECT_ID}
      bootstrapProjection={null}
      onBootstrapChanged={() => {}}
      activeLogicalCodebaseId={LC_ID}
      onSelectLogicalCodebase={() => {}}
      onOpenRegistration={() => {}}
      onOpenProviderManagement={onOpenProviderManagement}
      logicalCodebaseMembers={[]}
      aggregateInitialization={null}
      aggregateInitializationBusy={false}
      onStartAggregateInitialization={() => {}}
      onCancelAggregateInitialization={() => {}}
      onReinitializeAggregateInitialization={() => {}}
      aggregateIndex={null}
      aggregateIndexRebuilding={false}
      onRebuildAggregateIndex={() => {}}
      latestPointerPublication={null}
      pointerPublicationBusy={false}
      showIncrementalHint={false}
      onPublishFull={() => {}}
      onPublishIncremental={() => {}}
      onRetryRepo={() => {}}
      onRevoke={() => {}}
    />,
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  window.localStorage.clear();
});

describe("LogicalCodebaseManagementPanel 不再承载 provider 能力核验区", () => {
  it("面板内不再渲染 provider 核验区（能力卡 / 行级列表 / 批量按钮 / 作用域说明）", () => {
    stubFetch();
    renderPanel();

    // 能力核验区整体搬去独立的「Provider 管理」面板 ⇒ 这里一个都不渲染。
    expect(screen.queryByTestId("provider-capability-card")).toBeNull();
    expect(screen.queryByTestId("provider-capability-list")).toBeNull();
    expect(
      screen.queryByTestId("provider-capability-provider-claude_code"),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "一键核验全部" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("capability-batch-scope")).toBeNull();
    expect(screen.queryByTestId("capability-batch-start")).toBeNull();
  });

  it("原位置留一行只读说明 + 「去 Provider 管理」入口，点击即打开独立面板", async () => {
    stubFetch();
    const onOpenProviderManagement = vi.fn();
    const user = userEvent.setup();
    renderPanel(onOpenProviderManagement);

    // 只读说明（不含任何自造的统计数字）。
    const summary = screen.getByTestId("lc-provider-management-summary");
    expect(summary).toHaveTextContent("Provider");
    expect(summary).not.toHaveTextContent("已确认");

    // 「去 Provider 管理」入口可点击，触发打开独立面板。
    const link = screen.getByTestId("provider-management-link");
    expect(link).toHaveTextContent("去 Provider 管理");
    await user.click(link);
    expect(onOpenProviderManagement).toHaveBeenCalledTimes(1);
  });
});
