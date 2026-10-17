import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

beforeEach(() => {
  localStorage.clear();
});

describe("LocaleContext", () => {
  it("熱更新重新載入模組後，新模組的 useLocale 仍接得到舊模組的 Provider", async () => {
    // main.tsx 不會被熱更新，手上握的是舊模組的 Provider；設定頁跟著重新載入，用的是新模組的 useLocale。
    const before = await import("./LocaleContext");
    vi.resetModules();
    const after = await import("./LocaleContext");
    expect(after).not.toBe(before);

    let switchTo: (l: "en" | "zh-TW") => void = () => {};
    function Probe() {
      const { locale, setLocale } = after.useLocale();
      switchTo = setLocale;
      return <span>{locale}</span>;
    }
    render(
      <before.LocaleProvider>
        <Probe />
      </before.LocaleProvider>,
    );
    expect(screen.getByText("zh-TW")).toBeInTheDocument();
    act(() => switchTo("en"));
    expect(screen.getByText("en")).toBeInTheDocument();
  });
});
