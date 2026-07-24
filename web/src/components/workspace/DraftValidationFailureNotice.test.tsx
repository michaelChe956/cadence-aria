import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { DraftValidationFailureNotice } from "./DraftValidationFailureNotice";

const findingsOfFour = [
  {
    finding_id: "finding_1",
    level: "error",
    code: "unknown_done_when_ref",
    message: "完成条件引用不存在",
    affected_scopes: [],
  },
  {
    finding_id: "finding_2",
    level: "error",
    code: "missing_required_verification_command",
    message: "缺少必需验证命令",
    affected_scopes: [],
  },
  {
    finding_id: "finding_3",
    level: "error",
    code: "unknown_done_when_ref",
    message: "另一个完成条件引用不存在",
    affected_scopes: [],
  },
  {
    finding_id: "finding_4",
    level: "error",
    code: "missing_handoff",
    message: "缺少交接说明",
    affected_scopes: [],
  },
];

describe("DraftValidationFailureNotice", () => {
  it("announces the first three findings and expands a complete findings list", async () => {
    render(<DraftValidationFailureNotice findings={findingsOfFour} />);

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Draft 校验失败，暂不能接受（4 项）");
    const [summaryList] = within(alert).getAllByRole("list");
    const summaryItems = within(summaryList).getAllByRole("listitem");
    expect(summaryItems).toHaveLength(3);
    expect(summaryItems[0]).toHaveTextContent("unknown_done_when_ref");
    expect(summaryItems[1]).toHaveTextContent("missing_required_verification_command");
    expect(summaryItems[2]).toHaveTextContent("unknown_done_when_ref");

    await userEvent.click(within(alert).getByText("查看全部 4 项错误"));

    const lists = within(alert).getAllByRole("list");
    expect(lists).toHaveLength(2);
    const fullFindingsList = lists[1];
    const fullFindingItems = within(fullFindingsList).getAllByRole("listitem");
    expect(fullFindingItems).toHaveLength(4);
    expect(fullFindingItems[0]).toHaveTextContent("unknown_done_when_ref");
    expect(fullFindingItems[1]).toHaveTextContent("missing_required_verification_command");
    expect(fullFindingItems[2]).toHaveTextContent("unknown_done_when_ref");
    expect(fullFindingItems[3]).toHaveTextContent("missing_handoff");
  });

  it("announces the fallback guidance when findings are unavailable", () => {
    render(<DraftValidationFailureNotice findings={[]} />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "校验详情暂不可用，请根据 Draft 内容重写",
    );
  });
});
