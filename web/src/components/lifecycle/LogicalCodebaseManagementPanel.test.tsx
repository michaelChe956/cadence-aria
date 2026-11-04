// 票3 片6：逻辑代码库管理面板渲染批量核验条（与能力卡并列）。
import { render, screen } from "@testing-library/react";
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

/** 面板子组件（能力卡）挂载即 GET capabilities：给空投影即可。 */
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

function renderPanel() {
  return render(
    <LogicalCodebaseManagementPanel
      logicalCodebases={logicalCodebases}
      projectId={PROJECT_ID}
      bootstrapProjection={null}
      onBootstrapChanged={() => {}}
      activeLogicalCodebaseId={LC_ID}
      onSelectLogicalCodebase={() => {}}
      onOpenRegistration={() => {}}
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

describe("LogicalCodebaseManagementPanel 批量核验入口", () => {
  it("「一键核验全部」按钮在能力卡头部（上方），不再有面板底部的独立批量条", async () => {
    stubFetch();
    renderPanel();

    // 按钮挂在能力卡里（不是面板底部另起一条）。
    const card = await screen.findByTestId("provider-capability-card");
    const startButton = screen.getByRole("button", { name: "一键核验全部" });
    expect(card).toContainElement(startButton);
    expect(
      screen.queryByTestId("provider-capability-batch-bar"),
    ).not.toBeInTheDocument();

    // 按钮在卡片内位于四家 provider 行之前（即「上面」）。
    const firstProviderRow = await screen.findByTestId(
      "provider-capability-provider-claude_code",
    );
    expect(
      startButton.compareDocumentPosition(firstProviderRow) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    // 按钮区域仍说明作用域与成本（REQ-PCR-06）。
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "逐 LC 核验",
    );
  });
});
