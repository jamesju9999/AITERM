import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CmdTag } from "./CmdTag";

describe("CmdTag", () => {
  beforeEach(() => {
    vi.spyOn(window, "confirm").mockImplementation(() => true);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the command text", () => {
    render(<CmdTag command="ls -la" multiline={false} onExec={vi.fn()} />);
    expect(screen.getByText("ls -la")).toBeInTheDocument();
  });

  it("single-line click calls onExec without confirmation", async () => {
    const onExec = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm");
    render(<CmdTag command="ls" multiline={false} onExec={onExec} />);

    await userEvent.click(screen.getByRole("button"));
    expect(onExec).toHaveBeenCalledWith("ls");
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it("multi-line click shows confirm and runs on approval", async () => {
    const onExec = vi.fn();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(
      <CmdTag
        command={"cd /tmp\nls -la"}
        multiline={true}
        onExec={onExec}
      />,
    );

    await userEvent.click(screen.getByRole("button"));
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(onExec).toHaveBeenCalledWith("cd /tmp\nls -la");
  });

  it("multi-line click does not run when confirm cancelled", async () => {
    const onExec = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(
      <CmdTag
        command={"cd /tmp\nls -la"}
        multiline={true}
        onExec={onExec}
      />,
    );

    await userEvent.click(screen.getByRole("button"));
    expect(onExec).not.toHaveBeenCalled();
  });
});
