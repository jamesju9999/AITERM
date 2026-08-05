import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { MailView } from "./MailView";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations, LOCALE_STORAGE_KEY } from "../../lib/i18n";

// Captures the most recently registered callback so tests can drive a
// mail-sync-event through the component. MailView re-registers whenever the
// selected account changes, so "most recent" is always the live listener.
let syncListener: ((event: { payload: unknown }) => void) | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((_eventName: string, cb: (event: { payload: unknown }) => void) => {
    syncListener = cb;
    return Promise.resolve(() => { syncListener = null; });
  }),
}));

vi.mock("../../ipc/mail", () => ({
  MAIL_SYNC_EVENT: "mail-sync-event",
  mailListAccounts: vi.fn(),
  mailListMessages: vi.fn(),
  mailMarkRead: vi.fn(),
  mailDeleteMessage: vi.fn(),
}));

import { mailDeleteMessage, mailListAccounts, mailListMessages, mailMarkRead } from "../../ipc/mail";

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

// The click-to-mark-read row, matched on the sender rather than the subject:
// the row now has a delete button beside it whose accessible name names the
// subject, so a /Quarterly report/ query would match both.
const ROW_NAME = /sender@example\.com/;

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
    vi.mocked(mailDeleteMessage).mockResolvedValue(undefined as never);
    syncListener = null;
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

      const row = await screen.findByRole("button", { name: ROW_NAME });
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
      expect(screen.queryByRole("button", { name: ROW_NAME })).toBeNull();

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

      const row = await screen.findByRole("button", { name: ROW_NAME });
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

      fireEvent.click(await screen.findByRole("button", { name: ROW_NAME }));

      await waitFor(() => expect(onMessageRead).toHaveBeenCalledTimes(1));
    });

    it("does not notify the parent when marking read fails", async () => {
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);
      vi.mocked(mailMarkRead).mockRejectedValue(new Error("write failed") as never);
      const onMessageRead = vi.fn();

      renderView(onMessageRead);

      fireEvent.click(await screen.findByRole("button", { name: ROW_NAME }));

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

  // Moving a message to the server's Trash folder is the only thing this app
  // ever writes to IMAP, so the UI's job is to make it impossible to trigger by
  // accident and impossible to *believe* happened when it didn't.
  describe("delete a message", () => {
    beforeEach(() => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);
    });

    const findDeleteButton = () =>
      screen.findByRole("button", { name: t.mail_delete_aria("Quarterly report") });

    it("does not touch the server on the first click, only arms the confirmation", async () => {
      renderView();

      fireEvent.click(await findDeleteButton());

      expect(mailDeleteMessage).not.toHaveBeenCalled();
      expect(screen.getByText(t.mail_delete_confirm)).toBeTruthy();
    });

    it("deletes on the second click and drops the row", async () => {
      renderView();

      fireEvent.click(await findDeleteButton());
      fireEvent.click(screen.getByText(t.mail_delete_confirm));

      expect(mailDeleteMessage).toHaveBeenCalledWith("msg-1");
      await waitFor(() => expect(screen.queryByText("Quarterly report")).toBeNull());
    });

    it("disarms the confirmation when the cancel control is used", async () => {
      renderView();

      fireEvent.click(await findDeleteButton());
      fireEvent.click(screen.getByLabelText(t.mail_delete_cancel));

      expect(screen.queryByText(t.mail_delete_confirm)).toBeNull();
      expect(mailDeleteMessage).not.toHaveBeenCalled();
    });

    it("disarms the confirmation when the pointer leaves the row", async () => {
      const { container } = renderView();

      fireEvent.click(await findDeleteButton());
      fireEvent.mouseLeave(container.querySelector(".mail-view__item")!);

      expect(screen.queryByText(t.mail_delete_confirm)).toBeNull();
      expect(mailDeleteMessage).not.toHaveBeenCalled();
    });

    // The worst thing this feature could do is tell the user their mail is gone
    // while it is still sitting in their inbox.
    it("keeps the message in the list and shows the reason when the move fails", async () => {
      vi.mocked(mailDeleteMessage).mockRejectedValue(
        new Error("could not find a Trash folder on this server") as never
      );

      renderView();

      fireEvent.click(await findDeleteButton());
      fireEvent.click(screen.getByText(t.mail_delete_confirm));

      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain(t.mail_delete_failed);
      // The server's own reason, not a generic "delete failed" — a user whose
      // provider supports neither MOVE nor UIDPLUS can act on this one.
      expect(alert.textContent).toContain("could not find a Trash folder on this server");
      expect(screen.getByText("Quarterly report")).toBeTruthy();
    });

    // The row is a click-to-mark-read target and the delete control lives in
    // it. Deleting a message must not also flip it to read behind the user's
    // back — and if the delete then fails, the row would be left silently
    // marked read for a message that never went anywhere.
    it("does not mark the message read while deleting it", async () => {
      renderView();

      fireEvent.click(await findDeleteButton());
      fireEvent.click(screen.getByText(t.mail_delete_confirm));

      await waitFor(() => expect(mailDeleteMessage).toHaveBeenCalled());
      expect(mailMarkRead).not.toHaveBeenCalled();
    });

    it("blocks a second confirm click while the first is still in flight", async () => {
      // Never resolves: the delete is still on the wire when the second click
      // lands, which is exactly the window an impatient user clicks in.
      vi.mocked(mailDeleteMessage).mockReturnValue(new Promise(() => {}) as never);

      renderView();

      fireEvent.click(await findDeleteButton());
      const confirm = screen.getByText(t.mail_delete_confirm);
      fireEvent.click(confirm);
      await waitFor(() => expect(confirm).toBeDisabled());
      fireEvent.click(confirm);

      expect(mailDeleteMessage).toHaveBeenCalledTimes(1);
    });

    // A read message has no button role on its row, but it must still be
    // deletable — otherwise reading a message traps it in the list forever.
    it("offers the delete control on an already-read message too", async () => {
      vi.mocked(mailListMessages).mockResolvedValue(
        [makeMessage({ is_read_locally: true })] as never
      );

      renderView();

      fireEvent.click(await findDeleteButton());
      fireEvent.click(screen.getByText(t.mail_delete_confirm));

      expect(mailDeleteMessage).toHaveBeenCalledWith("msg-1");
    });
  });

  // Mail deleted or archived on the server is removed from the local cache by
  // the poller, which emits a `removed` sync event. The list has to refetch on
  // it, or the Mail tab keeps showing messages that no longer exist.
  describe("server-side deletions", () => {
    it("drops a message from the list when a removal event arrives", async () => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);
      vi.mocked(mailListMessages).mockResolvedValueOnce([makeMessage()] as never);

      renderView();
      expect(await screen.findByText("Quarterly report")).toBeTruthy();

      vi.mocked(mailListMessages).mockResolvedValue([] as never);
      await waitFor(() => expect(syncListener).not.toBeNull());
      syncListener!({ payload: { kind: "removed", account_id: "acc-1", removed_count: 1 } });

      await waitFor(() => expect(screen.queryByText("Quarterly report")).toBeNull());
    });

    it("ignores a removal event for an account other than the selected one", async () => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);

      renderView();
      expect(await screen.findByText("Quarterly report")).toBeTruthy();
      const callsBefore = vi.mocked(mailListMessages).mock.calls.length;

      await waitFor(() => expect(syncListener).not.toBeNull());
      syncListener!({ payload: { kind: "removed", account_id: "some-other-account", removed_count: 1 } });

      await new Promise((r) => setTimeout(r, 0));
      expect(vi.mocked(mailListMessages).mock.calls.length).toBe(callsBefore);
    });
  });

  // A wrong password, a revoked Gmail App Password or a network outage used to
  // reach the user as nothing but a log line, in a log already full of
  // unrelated warnings — the Mail tab simply stopped updating with no
  // explanation at all.
  describe("connection failures", () => {
    const FAILED = {
      kind: "connection_failed",
      account_id: "acc-1",
      message: "login failed: [AUTHENTICATIONFAILED] Invalid credentials",
    };

    const driveEvent = async (payload: unknown) => {
      await waitFor(() => expect(syncListener).not.toBeNull());
      syncListener!({ payload });
    };

    beforeEach(() => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);
      vi.mocked(mailListMessages).mockResolvedValue([makeMessage()] as never);
    });

    it("surfaces the server's own reason, named against the account", async () => {
      renderView();
      expect(await screen.findByText("Quarterly report")).toBeTruthy();

      await driveEvent(FAILED);

      const alert = await screen.findByRole("alert");
      // The account, so a user with several knows which one to go and fix.
      expect(alert.textContent).toContain(ACCOUNT.email);
      // And the backend's wording verbatim: "login failed" sends the user to
      // their App Password, "connection error" to their network.
      expect(alert.textContent).toContain("[AUTHENTICATIONFAILED] Invalid credentials");
    });

    it("clears the banner when the account reconnects", async () => {
      renderView();
      await driveEvent(FAILED);
      expect(await screen.findByRole("alert")).toBeTruthy();

      await driveEvent({ kind: "connection_restored", account_id: "acc-1" });

      await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    });

    // Both are red text in the same corner of the same tab, and they mean
    // completely different things: one row failed to move vs. nothing in this
    // tab is up to date.
    it("is visually distinct from a transient delete failure", async () => {
      vi.mocked(mailDeleteMessage).mockRejectedValue(new Error("no Trash folder") as never);

      const { container } = renderView();
      await driveEvent(FAILED);
      await screen.findByRole("alert");

      const connectionBanner = container.querySelector(".mail-view__error--connection");
      expect(connectionBanner).toBeTruthy();
      expect(connectionBanner!.textContent).toContain(ACCOUNT.email);

      // Now provoke a delete failure alongside it.
      fireEvent.click(await screen.findByRole("button", { name: t.mail_delete_aria("Quarterly report") }));
      fireEvent.click(screen.getByText(t.mail_delete_confirm));

      // Barrier: the delete rejection has landed and rendered. Without it
      // `getAllByRole` below would see only the connection banner and the
      // length assertion would fail for the wrong reason.
      const deleteBanner = await screen.findByText(new RegExp(t.mail_delete_failed));
      const banners = screen.getAllByRole("alert");
      expect(banners).toHaveLength(2);
      // The delete failure must NOT carry the connection modifier, or the two
      // would render identically and the user could not tell a one-off from a
      // standing outage.
      // (The length assertion above is also what catches a delete failure
      // clearing the standing banner, so there is no separate check for it.)
      expect(deleteBanner.classList.contains("mail-view__error--connection")).toBe(false);
    });

    it("does not refetch the message list on a connection event", async () => {
      renderView();
      expect(await screen.findByText("Quarterly report")).toBeTruthy();
      const callsBefore = vi.mocked(mailListMessages).mock.calls.length;

      await driveEvent(FAILED);

      // The banner is up, so the event was definitely processed.
      expect(await screen.findByRole("alert")).toBeTruthy();
      expect(vi.mocked(mailListMessages).mock.calls.length).toBe(callsBefore);
    });

    it("remembers a failure raised while another account was selected", async () => {
      const ACCOUNT_2 = { ...ACCOUNT, id: "acc-2", email: "second@example.com" };
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT, ACCOUNT_2] as never);

      renderView();
      const select = await screen.findByLabelText(t.mail_select_account);
      fireEvent.change(select, { target: { value: "acc-2" } });
      await waitFor(() => expect(mailListMessages).toHaveBeenLastCalledWith("acc-2"));

      // acc-1 breaks while acc-2 is on screen.
      await driveEvent(FAILED);
      expect(screen.queryByRole("alert")).toBeNull();

      fireEvent.change(select, { target: { value: "acc-1" } });

      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain(ACCOUNT.email);
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
