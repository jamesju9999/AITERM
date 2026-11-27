import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const remote = vi.hoisted(() => ({ value: { connected: true, role: "admin" } as { connected: boolean; role: string } }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => remote.value }));
vi.mock("../../ipc/serverQuestions", () => ({ getAskNotifyEmail: vi.fn(), setAskNotifyEmail: vi.fn() }));

import { getAskNotifyEmail, setAskNotifyEmail } from "../../ipc/serverQuestions";
import { AskNotifySettings } from "./AskNotifySettings";

const mount = () => render(<LocaleProvider><AskNotifySettings /></LocaleProvider>);

beforeEach(() => {
  remote.value = { connected: true, role: "admin" };
  vi.mocked(getAskNotifyEmail).mockResolvedValue("boss@example.com");
  vi.mocked(setAskNotifyEmail).mockResolvedValue(undefined);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("AskNotifySettings", () => {
  it("成員或沒連線時不顯示、也不去問 server", () => {
    remote.value = { connected: true, role: "member" };
    const { container } = mount();
    expect(container).toBeEmptyDOMElement();
    expect(getAskNotifyEmail).not.toHaveBeenCalled();
  });

  it("管理員看到目前的信箱；改了才能存，存的是修剪後的值", async () => {
    mount();
    const input = await screen.findByDisplayValue("boss@example.com");
    const save = screen.getByRole("button", { name: "送出" });
    expect(save).toBeDisabled();
    fireEvent.change(input, { target: { value: " new@example.com " } });
    fireEvent.click(save);
    await waitFor(() => expect(setAskNotifyEmail).toHaveBeenCalledWith("new@example.com"));
  });

  it("server 拒絕時顯示原因", async () => {
    vi.mocked(setAskNotifyEmail).mockRejectedValue("Email 格式不對");
    mount();
    fireEvent.change(await screen.findByDisplayValue("boss@example.com"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "送出" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Email 格式不對");
  });
});
