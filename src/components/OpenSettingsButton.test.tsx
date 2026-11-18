import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";
import { OpenSettingsButton } from "./OpenSettingsButton";
import { LocaleProvider } from "../contexts/LocaleContext";
import { translations } from "../lib/i18n";

const t = translations["zh-TW"];

function Probe() {
  const loc = useLocation();
  return <div data-testid="at">{loc.pathname}|{(loc.state as { tab?: string } | null)?.tab}</div>;
}

describe("OpenSettingsButton", () => {
  it("navigates to /settings with the requested tab and runs the pre-navigate hook", () => {
    const before = vi.fn();
    render(
      <MemoryRouter initialEntries={["/"]}>
        <LocaleProvider>
          <Routes>
            <Route path="/" element={<OpenSettingsButton tab="mail" onBeforeNavigate={before} />} />
            <Route path="/settings" element={<Probe />} />
          </Routes>
        </LocaleProvider>
      </MemoryRouter>,
    );
    fireEvent.click(screen.getByRole("button", { name: t.open_settings_mail }));
    expect(before).toHaveBeenCalledOnce();
    expect(screen.getByTestId("at").textContent).toBe("/settings|mail");
  });

  it("renders nothing outside a Router instead of throwing", () => {
    const { container } = render(<LocaleProvider><OpenSettingsButton tab="providers" /></LocaleProvider>);
    expect(container.querySelector("button")).toBeNull();
  });
});
