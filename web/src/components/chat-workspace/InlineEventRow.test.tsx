import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ChatEntry } from "../../state/chat-entries";
import { InlineEventRow } from "./InlineEventRow";

describe("InlineEventRow", () => {
  it("renders collapsed by default and toggles command output details", () => {
    render(
      <InlineEventRow
        entry={{
          id: "event-1",
          type: "execution_event",
          role: "system",
          content: "读取认证模块",
          timestamp: "2026-05-26T10:00:00Z",
          metadata: {
            command: "sed -n '1,120p' src/auth.rs",
            output: "ok",
          },
        }}
      />,
    );

    expect(screen.getByText("读取认证模块")).toBeInTheDocument();
    expect(screen.queryByText("sed -n '1,120p' src/auth.rs")).not.toBeInTheDocument();
    expect(screen.queryByText("ok")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /读取认证模块/ }));

    expect(screen.getByText("sed -n '1,120p' src/auth.rs")).toBeInTheDocument();
    expect(screen.getByText("ok")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /读取认证模块/ }));

    expect(screen.queryByText("sed -n '1,120p' src/auth.rs")).not.toBeInTheDocument();
    expect(screen.queryByText("ok")).not.toBeInTheDocument();
  });
});
