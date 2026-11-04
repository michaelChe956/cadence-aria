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
        return jsonResponse({ providers: [] });
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

describe("LogicalCodebaseManagementPanel 批量核验条", () => {
  it("面板渲染「一键核验全部」批量条（与能力卡并列）", () => {
    stubFetch();
    renderPanel();

    expect(
      screen.getByTestId("provider-capability-batch-bar"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "一键核验全部" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("capability-batch-scope")).toHaveTextContent(
      "逐 LC 核验",
    );
  });
});
