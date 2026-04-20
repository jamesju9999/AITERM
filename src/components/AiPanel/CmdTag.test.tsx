import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CmdTag } from "./CmdTag";

describe("CmdTag", () => {
  it("renders the command text", () => {
    render(<CmdTag command="ls -la" multiline={false} onExec={vi.fn()} />);
    expect(screen.getByText("ls -la")).toBeInTheDocument();
  });

  it("single-line click calls onExec immediately", async () => {
    const onExec = vi.fn();
    render(<CmdTag command="ls" multiline={false} onExec={onExec} />);

    await userEvent.click(screen.getByRole("button"));
    expect(onExec).toHaveBeenCalledWith("ls");
  });

  it("multi-line first click shows inline confirm, does not exec", async () => {
    const onExec = vi.fn();
    render(
      <CmdTag command={"cd /tmp\nls -la"} multiline={true} onExec={onExec} />,
    );

    await userEvent.click(screen.getByRole("button", { name: /cd \/tmp/ }));
    expect(onExec).not.toHaveBeenCalled();
    // Cancel button should appear
    expect(screen.getByRole("button", { name: "取消" })).toBeInTheDocument();
  });

  it("multi-line confirm: second click on command button executes", async () => {
    const onExec = vi.fn();
    render(
      <CmdTag command={"cd /tmp\nls -la"} multiline={true} onExec={onExec} />,
    );

    const cmdButton = screen.getByRole("button", { name: /cd \/tmp/ });
    await userEvent.click(cmdButton); // first click → show confirm
    await userEvent.click(cmdButton); // second click → exec
    expect(onExec).toHaveBeenCalledWith("cd /tmp\nls -la");
  });

  it("multi-line cancel: clicking 取消 does not exec", async () => {
    const onExec = vi.fn();
    render(
      <CmdTag command={"cd /tmp\nls -la"} multiline={true} onExec={onExec} />,
    );

    await userEvent.click(screen.getByRole("button", { name: /cd \/tmp/ })); // show confirm
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onExec).not.toHaveBeenCalled();
  });
});
