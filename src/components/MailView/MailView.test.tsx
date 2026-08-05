import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { MailView } from "./MailView";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("../../ipc/mail", () => ({
  MAIL_SYNC_EVENT: "mail-sync-event",
  mailListAccounts: vi.fn(),
  mailListMessages: vi.fn(),
  mailMarkRead: vi.fn(),
}));

import { mailListAccounts, mailListMessages, mailMarkRead } from "../../ipc/mail";

// LocaleProvider defaults to zh-TW when nothing is stored, so assert against
// that table rather than hard-coding the strings.
const t = translations["zh-TW"];

const ACCOUNT = {
  id: "acc-1",
  email: "me@example.com",
  imap_host: "imap.example.com",
  imap_port: 993,
  smtp_host: "smtp.example.com",
  smtp_port: 465,
  username: "me",
  poll_interval_secs: 300,
};

function makeMessage(over: Partial<Record<string, unknown>> = {}) {
  return {
    id: "msg-1",
    account_id: "acc-1",
    uid: 1,
    sender: "sender@example.com",
    subject: "Quarterly report",
    date: "2026-08-04T10:00:00Z",
    body_text: "body",
    ai_summary: "A short summary",
    is_important: false,
    is_promotional: false,
    is_read_locally: false,
    fetched_at: "2026-08-04T10:00:00Z",
    ...over,
  };
}

function renderView() {
  return render(
    <LocaleProvider>
      <MailView isActive />
    </LocaleProvider>
  );
}

describe("MailView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The error-path tests log intentionally; keep the suite output clean.
    vi.spyOn(console, "error").mockImplementation(() => {});
    vi.mocked(mailListMessages).mockResolvedValue([] as never);
    vi.mocked(mailMarkRead).mockResolvedValue(undefined as never);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("initial load states", () => {
    it("does not flash the empty state while the accounts fetch is pending", () => {
      // Never resolves — the component must stay in its loading state.
      vi.mocked(mailListAccounts).mockReturnValue(new Promise(() => {}) as never);

      renderView();

      expect(screen.queryByText(t.mail_no_accounts)).toBeNull();
      expect(screen.queryByText(t.mail_load_failed)).toBeNull();
    });

    it("shows a distinct error message when loading accounts fails", async () => {
      vi.mocked(mailListAccounts).mockRejectedValue(new Error("ipc exploded") as never);

      renderView();

      expect(await screen.findByText(t.mail_load_failed)).toBeTruthy();
      // Must NOT be mistaken for "you have no accounts" — that would send the
      // user off to re-add an account they already have.
      expect(screen.queryByText(t.mail_no_accounts)).toBeNull();
    });

    it("shows the empty state when there genuinely are no accounts", async () => {
      vi.mocked(mailListAccounts).mockResolvedValue([] as never);

      renderView();

      expect(await screen.findByText(t.mail_no_accounts)).toBeTruthy();
      expect(screen.queryByText(t.mail_load_failed)).toBeNull();
    });
  });

  describe("message list", () => {
    beforeEach(() => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);
    });

    it("marks an unread message read on click and clears its unread styling", async () => {
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);

      const { container } = renderView();

      const row = await screen.findByRole("button", { name: /Quarterly report/ });
      expect(container.querySelector(".mail-view__item--unread")).toBeTruthy();

      fireEvent.click(row);

      expect(mailMarkRead).toHaveBeenCalledWith("msg-1");
      await waitFor(() => {
        expect(container.querySelector(".mail-view__item--unread")).toBeNull();
      });
    });

    it("does not call mailMarkRead for an already-read message", async () => {
      vi.mocked(mailListMessages).mockResolvedValue(
        [makeMessage({ is_read_locally: true })] as never
      );

      const { container } = renderView();

      await waitFor(() => {
        expect(container.querySelector(".mail-view__item")).toBeTruthy();
      });
      // A read row is inert: no button role, so no dead tab stop either.
      expect(screen.queryByRole("button", { name: /Quarterly report/ })).toBeNull();

      fireEvent.click(container.querySelector(".mail-view__item")!);

      expect(mailMarkRead).not.toHaveBeenCalled();
    });

    it("rolls the row back to unread when marking read fails", async () => {
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);
      vi.mocked(mailMarkRead).mockRejectedValue(new Error("write failed") as never);

      const { container } = renderView();

      const row = await screen.findByRole("button", { name: /Quarterly report/ });
      fireEvent.click(row);

      // Optimistically cleared, then restored once the backend call rejects.
      await waitFor(() => {
        expect(container.querySelector(".mail-view__item--unread")).toBeTruthy();
      });
    });

    it("omits the summary element when a message has no ai_summary", async () => {
      vi.mocked(mailListMessages).mockResolvedValue(
        [makeMessage({ ai_summary: null })] as never
      );

      const { container } = renderView();

      await waitFor(() => {
        expect(container.querySelector(".mail-view__item")).toBeTruthy();
      });
      expect(container.querySelector(".mail-view__item-summary")).toBeNull();
    });

    it("keeps the list semantics intact so the row role does not replace listitem", async () => {
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);

      renderView();

      expect(await screen.findByRole("listitem")).toBeTruthy();
    });
  });
});
