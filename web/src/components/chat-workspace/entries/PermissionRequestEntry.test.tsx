import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import { PermissionRequestEntry } from "./PermissionRequestEntry";

describe("PermissionRequestEntry", () => {
  it("disables buttons immediately after a response is clicked", () => {
    const onRespond = vi.fn();
    const entry = makeEntry();

    render(<PermissionRequestEntry entry={entry} onRespond={onRespond} />);

    fireEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(screen.getByRole("button", { name: "允许" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "拒绝" })).toBeDisabled();
    expect(onRespond).toHaveBeenCalledWith(entry, true);
  });

  it("shows the resolved result and hides buttons", () => {
    render(
      <PermissionRequestEntry
        entry={{
          ...makeEntry(),
          resolved: true,
          metadata: {
            request_id: "permission-1",
            tool_name: "shell",
            description: "cargo test",
            approved: true,
          },
        }}
        onRespond={vi.fn()}
      />,
    );

    expect(screen.getByText("已允许")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "允许" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "拒绝" })).not.toBeInTheDocument();
  });
});

function makeEntry(): ChatEntry {
  return {
    id: "permission-request-1",
    type: "permission_request",
    role: "system",
    content: "shell · cargo test",
    timestamp: "2026-05-26T10:00:00Z",
    metadata: {
      request_id: "permission-1",
      tool_name: "shell",
      description: "cargo test",
      risk_level: "medium",
    },
  };
}
