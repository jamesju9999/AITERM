import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../contexts/LocaleContext";
import { StreamingIndicator } from "./StreamingIndicator";

const renderIndicator = (onStop?: () => void) =>
  render(
    <LocaleProvider>
      <StreamingIndicator visible text="" onStop={onStop} />
    </LocaleProvider>,
  );

describe("StreamingIndicator stop", () => {
  it("fires onStop from the button", async () => {
    const onStop = vi.fn();
    renderIndicator(onStop);
    await userEvent.click(screen.getByRole("button", { name: /停止/ }));
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("fires onStop on Escape", async () => {
    const onStop = vi.fn();
    renderIndicator(onStop);
    await userEvent.keyboard("{Escape}");
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("renders no button without onStop", () => {
    renderIndicator();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
