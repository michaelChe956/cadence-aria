import { render, screen } from "@testing-library/react";
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
  it("announces the first three findings and keeps remaining findings in details", () => {
    render(<DraftValidationFailureNotice findings={findingsOfFour} />);

    expect(screen.getByRole("alert")).toHaveTextContent("Draft 校验失败，暂不能接受（4 项）");
    expect(screen.getAllByText(/unknown_done_when_ref|missing_required_verification_command/)).toHaveLength(3);
    expect(screen.getByText("查看全部 4 项错误")).toBeInTheDocument();
    expect(screen.getByText("missing_handoff")).toBeInTheDocument();
  });

  it("announces the fallback guidance when findings are unavailable", () => {
    render(<DraftValidationFailureNotice findings={[]} />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "校验详情暂不可用，请根据 Draft 内容重写",
    );
  });
});
