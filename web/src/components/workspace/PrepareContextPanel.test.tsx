import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { PrepareContextPanel } from "./PrepareContextPanel";

describe("PrepareContextPanel", () => {
  it("sends a trimmed context note on submit", () => {
    const onSendContextNote = vi.fn();
    render(
      <PrepareContextPanel
        contextNotes={[]}
        onSendContextNote={onSendContextNote}
        onStartGeneration={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByTestId("context-note-input"), {
      target: { value: " 需要支持空查询参数 " },
    });
    fireEvent.click(screen.getByTestId("send-context-note"));

    expect(onSendContextNote).toHaveBeenCalledWith("需要支持空查询参数");
    expect(screen.getByTestId("context-note-input")).toHaveValue("");
  });

  it("does not send empty context notes", () => {
    const onSendContextNote = vi.fn();
    render(
      <PrepareContextPanel
        contextNotes={[]}
        onSendContextNote={onSendContextNote}
        onStartGeneration={vi.fn()}
      />,
    );

    expect(screen.getByTestId("send-context-note")).toBeDisabled();
    fireEvent.click(screen.getByTestId("send-context-note"));

    expect(onSendContextNote).not.toHaveBeenCalled();
  });

  it("sends start generation when the CTA is clicked", () => {
    const onStartGeneration = vi.fn();
    render(
      <PrepareContextPanel
        contextNotes={[]}
        onSendContextNote={vi.fn()}
        onStartGeneration={onStartGeneration}
      />,
    );

    fireEvent.click(screen.getByTestId("start-generation"));

    expect(onStartGeneration).toHaveBeenCalledTimes(1);
  });

  it("shows context notes from the backend-derived list", () => {
    render(
      <PrepareContextPanel
        contextNotes={["第一条", "第二条"]}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
      />,
    );

    expect(screen.getByTestId("prepare-context-panel")).toBeInTheDocument();
    expect(screen.getByText("已补充上下文 2 条")).toBeInTheDocument();
    expect(screen.getByText("第一条")).toBeInTheDocument();
    expect(screen.getByText("第二条")).toBeInTheDocument();
  });

  it("disables controls when disabled", () => {
    render(
      <PrepareContextPanel
        disabled={true}
        contextNotes={[]}
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
      />,
    );

    expect(screen.getByTestId("context-note-input")).toBeDisabled();
    expect(screen.getByTestId("send-context-note")).toBeDisabled();
    expect(screen.getByTestId("start-generation")).toBeDisabled();
  });
});
