import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { MailView } from "./MailView";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations, LOCALE_STORAGE_KEY } from "../../lib/i18n";

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

// Pinned explicitly rather than relying on LocaleProvider's fallback: if the
// default ever changed, the negative assertions below (queryByText(...) is
// null) would silently pass for the wrong reason.
const LOCALE = "zh-TW";
const t = translations[LOCALE];

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

function renderView(onMessageRead?: () => void) {
  return render(
    <LocaleProvider>
      <MailView isActive onMessageRead={onMessageRead} />
    </LocaleProvider>
  );
}

describe("MailView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.setItem(LOCALE_STORAGE_KEY, LOCALE);
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

      // Must click the inner div, not the <li>: the handlers live on the inner
      // element, and events bubble up rather than down — clicking the <li>
      // could never reach them, making this assertion vacuously true.
      fireEvent.click(container.querySelector(".mail-view__item > div")!);

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

    // The global unread badge lives in TabBar, fed by useMailSync. Marking a
    // message read here only touches the backing store and emits no sync
    // event, so without this callback the badge would stay stale for up to
    // poll_interval_secs (default 300s) after the user clears a row.
    it("notifies the parent after a successful mark-read so the unread badge can refresh", async () => {
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);
      const onMessageRead = vi.fn();

      renderView(onMessageRead);

      fireEvent.click(await screen.findByRole("button", { name: /Quarterly report/ }));

      await waitFor(() => expect(onMessageRead).toHaveBeenCalledTimes(1));
    });

    it("does not notify the parent when marking read fails", async () => {
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);
      vi.mocked(mailMarkRead).mockRejectedValue(new Error("write failed") as never);
      const onMessageRead = vi.fn();

      renderView(onMessageRead);

      fireEvent.click(await screen.findByRole("button", { name: /Quarterly report/ }));

      // Barrier: wait until the rejection has actually been handled (the catch
      // logs), otherwise this would pass simply by asserting too early — the
      // unread class alone is a weak signal, since it is also present before
      // the optimistic update lands.
      await waitFor(() => expect(console.error).toHaveBeenCalledWith(
        "[mail] failed to mark message read:", expect.anything()
      ));
      expect(onMessageRead).not.toHaveBeenCalled();
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

  // App.tsx keeps TerminalApp — and therefore this component — permanently
  // mounted behind the Settings overlay, so an account added in Settings can
  // only reach an already-open Mail tab by refetching when the tab is shown
  // again. Without it the tab reads "no accounts" forever while the TabBar
  // badge (fed by mail_count_unread) already shows unread counts.
  describe("refetch on tab reactivation", () => {
    const ACCOUNT_2 = { ...ACCOUNT, id: "acc-2", email: "second@example.com" };
    const ACCOUNT_3 = { ...ACCOUNT, id: "acc-3", email: "third@example.com" };

    const tree = (isActive: boolean) => (
      <LocaleProvider>
        <MailView isActive={isActive} />
      </LocaleProvider>
    );

    it("refetches and shows an account added while the tab was hidden", async () => {
      vi.mocked(mailListAccounts)
        .mockResolvedValueOnce([] as never)
        .mockResolvedValueOnce([ACCOUNT] as never);

      const { rerender } = render(tree(true));
      expect(await screen.findByText(t.mail_no_accounts)).toBeTruthy();

      // Switch to another tab (still mounted, just hidden) and back.
      rerender(tree(false));
      rerender(tree(true));

      await waitFor(() => expect(mailListAccounts).toHaveBeenCalledTimes(2));
      expect(await screen.findByText(ACCOUNT.email)).toBeTruthy();
    });

    it("does not refetch on re-renders while the tab stays active", async () => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);

      const { rerender } = render(tree(true));
      // Barrier: the initial fetch has landed, so the count below is measured
      // against a settled component rather than a still-loading one.
      expect(await screen.findByText(ACCOUNT.email)).toBeTruthy();

      // Unrelated re-renders: isActive is unchanged, only the callback identity
      // differs. An IMAP-account fetch per parent render would be wasteful.
      rerender(<LocaleProvider><MailView isActive onMessageRead={() => {}} /></LocaleProvider>);
      rerender(<LocaleProvider><MailView isActive onMessageRead={() => {}} /></LocaleProvider>);

      expect(mailListAccounts).toHaveBeenCalledTimes(1);
    });

    it("keeps the selected account across a reactivation refetch", async () => {
      vi.mocked(mailListAccounts)
        .mockResolvedValueOnce([ACCOUNT, ACCOUNT_2] as never)
        .mockResolvedValueOnce([ACCOUNT, ACCOUNT_2, ACCOUNT_3] as never);

      const { rerender } = render(tree(true));
      const select = await screen.findByLabelText(t.mail_select_account);
      fireEvent.change(select, { target: { value: "acc-2" } });

      rerender(tree(false));
      rerender(tree(true));

      // Barrier: the refetched list has actually landed in state, so the
      // selection assertion below cannot pass merely by running too early.
      expect(await screen.findByText(ACCOUNT_3.email)).toBeTruthy();
      expect(select).toHaveValue("acc-2");
    });

    it("falls back to the first account when the selected one was removed", async () => {
      vi.mocked(mailListAccounts)
        .mockResolvedValueOnce([ACCOUNT, ACCOUNT_2] as never)
        .mockResolvedValueOnce([ACCOUNT] as never);

      const { rerender } = render(tree(true));
      const select = await screen.findByLabelText(t.mail_select_account);
      fireEvent.change(select, { target: { value: "acc-2" } });
      await waitFor(() => expect(mailListMessages).toHaveBeenLastCalledWith("acc-2"));

      rerender(tree(false));
      rerender(tree(true));

      // Barrier: the removed account is gone from the list.
      await waitFor(() => expect(screen.queryByText(ACCOUNT_2.email)).toBeNull());
      // Asserted through the message fetch rather than the <select> value: a
      // <select> whose value matches no option falls back to showing the first
      // option anyway, so the DOM alone cannot tell a real reselection from a
      // stale selectedAccountId still pointing at the deleted account — which
      // is what actually decides whose mail is listed.
      expect(mailListMessages).toHaveBeenLastCalledWith("acc-1");
    });
  });
});
