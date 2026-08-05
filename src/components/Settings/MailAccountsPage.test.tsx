import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { MailAccountsPage } from "./MailAccountsPage";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations, LOCALE_STORAGE_KEY } from "../../lib/i18n";

vi.mock("../../ipc/mail", () => ({
  mailListAccounts: vi.fn(),
  mailAddAccount: vi.fn(),
  mailRemoveAccount: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
}));

import { mailListAccounts, mailAddAccount, mailRemoveAccount } from "../../ipc/mail";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";

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

function renderPage() {
  return render(<LocaleProvider><MailAccountsPage /></LocaleProvider>);
}

/** The fields the save guard requires; the first three are trim-checked. */
const REQUIRED_VALUES = {
  mail_email: "new@example.com",
  mail_imap_host: "imap.new.com",
  mail_username: "newuser",
  mail_password: "s3cret",
} as const;
const REQUIRED_FIELDS = Object.keys(REQUIRED_VALUES) as (keyof typeof REQUIRED_VALUES)[];
const TRIMMED_FIELDS = REQUIRED_FIELDS.filter((f) => f !== "mail_password");

/** Fills the guard's required fields, optionally leaving one of them blank. */
function fillRequired(omit?: keyof typeof REQUIRED_VALUES) {
  for (const field of REQUIRED_FIELDS) {
    if (field === omit) continue;
    fireEvent.change(screen.getByLabelText(t[field]), { target: { value: REQUIRED_VALUES[field] } });
  }
}

/** Opens the add form and fills every field with known values. */
async function fillForm() {
  fireEvent.click(await screen.findByRole("button", { name: t.mail_add }));
  fireEvent.change(screen.getByLabelText(t.mail_email), { target: { value: "new@example.com" } });
  fireEvent.change(screen.getByLabelText(t.mail_imap_host), { target: { value: "imap.new.com" } });
  fireEvent.change(screen.getByLabelText(t.mail_imap_port), { target: { value: "1993" } });
  fireEvent.change(screen.getByLabelText(t.mail_smtp_host), { target: { value: "smtp.new.com" } });
  fireEvent.change(screen.getByLabelText(t.mail_smtp_port), { target: { value: "1587" } });
  fireEvent.change(screen.getByLabelText(t.mail_username), { target: { value: "newuser" } });
  fireEvent.change(screen.getByLabelText(t.mail_password), { target: { value: "s3cret" } });
  fireEvent.change(screen.getByLabelText(t.mail_poll_interval), { target: { value: "120" } });
}

/** Clicks Save (the in-form submit button, distinct from the header's add button). */
function clickSave() {
  fireEvent.click(screen.getByRole("button", { name: t.save }));
}

describe("MailAccountsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.setItem(LOCALE_STORAGE_KEY, LOCALE);
    // The error-path tests log intentionally; keep the suite output clean.
    vi.spyOn(console, "error").mockImplementation(() => {});
    vi.mocked(mailListAccounts).mockResolvedValue([] as never);
    vi.mocked(mailAddAccount).mockResolvedValue(ACCOUNT as never);
    vi.mocked(mailRemoveAccount).mockResolvedValue(undefined as never);
    vi.mocked(isPermissionGranted).mockResolvedValue(true as never);
    vi.mocked(requestPermission).mockResolvedValue("granted" as never);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("listing", () => {
    it("lists existing accounts on mount", async () => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);

      renderPage();

      expect(await screen.findByText(ACCOUNT.email)).toBeTruthy();
      expect(screen.getByText(/imap\.example\.com:993/)).toBeTruthy();
    });

    it("does not flash the empty state while the accounts fetch is pending", () => {
      // Never resolves — the page must stay in its loading state.
      vi.mocked(mailListAccounts).mockReturnValue(new Promise(() => {}) as never);

      renderPage();

      expect(screen.queryByText(t.mail_accounts_empty)).toBeNull();
      expect(screen.queryByText(t.mail_load_failed)).toBeNull();
    });

    it("shows a distinct error when loading accounts fails", async () => {
      vi.mocked(mailListAccounts).mockRejectedValue(new Error("ipc exploded") as never);

      renderPage();

      expect(await screen.findByText(t.mail_load_failed)).toBeTruthy();
      // Must NOT be mistaken for "you have no accounts" — that would send the
      // user off to re-add an account they already have.
      expect(screen.queryByText(t.mail_accounts_empty)).toBeNull();
    });

    it("shows the empty state when there genuinely are no accounts", async () => {
      renderPage();

      expect(await screen.findByText(t.mail_accounts_empty)).toBeTruthy();
      expect(screen.queryByText(t.mail_load_failed)).toBeNull();
    });
  });

  describe("removing an account", () => {
    beforeEach(() => {
      vi.mocked(mailListAccounts).mockResolvedValue([ACCOUNT] as never);
    });

    it("removes the account by id and refetches the list", async () => {
      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_remove }));
      // Destructive action is confirm-gated, like its Settings siblings.
      fireEvent.click(screen.getByRole("button", { name: `${t.mail_remove}?` }));

      await waitFor(() => expect(mailRemoveAccount).toHaveBeenCalledWith("acc-1"));
      // 1 = initial mount, 2 = post-remove refetch.
      await waitFor(() => expect(mailListAccounts).toHaveBeenCalledTimes(2));
    });

    it("surfaces a removal failure in the UI instead of failing silently", async () => {
      vi.mocked(mailRemoveAccount).mockRejectedValue(new Error("keyring locked") as never);

      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_remove }));
      fireEvent.click(screen.getByRole("button", { name: `${t.mail_remove}?` }));

      expect(await screen.findByText(/keyring locked/)).toBeTruthy();
    });
  });

  describe("adding an account", () => {
    it("calls mailAddAccount with the entered values and refetches the list", async () => {
      renderPage();
      await fillForm();

      clickSave();

      await waitFor(() => expect(mailAddAccount).toHaveBeenCalledWith({
        email: "new@example.com",
        imap_host: "imap.new.com",
        imap_port: 1993,
        smtp_host: "smtp.new.com",
        smtp_port: 1587,
        username: "newuser",
        password: "s3cret",
        poll_interval_secs: 120,
      }));
      // 1 = initial mount, 2 = post-add refetch.
      await waitFor(() => expect(mailListAccounts).toHaveBeenCalledTimes(2));
    });

    // `Number("")` is 0, so backspacing the field clean used to send
    // poll_interval_secs: 0 — the backend only defaults on None, so Some(0)
    // reaches poller.rs's sleep(Duration::from_secs(0)) and becomes an
    // unthrottled IMAP reconnect loop. With no edit command, the only recovery
    // would be remove-and-re-add.
    it("clamps a cleared poll interval up to the 60s floor", async () => {
      renderPage();
      await fillForm();

      fireEvent.change(screen.getByLabelText(t.mail_poll_interval), { target: { value: "" } });
      clickSave();

      await waitFor(() => expect(mailAddAccount).toHaveBeenCalledWith(
        expect.objectContaining({ poll_interval_secs: 60 })
      ));
    });

    it("clamps an explicit 0 poll interval up to the 60s floor", async () => {
      renderPage();
      await fillForm();

      fireEvent.change(screen.getByLabelText(t.mail_poll_interval), { target: { value: "0" } });
      clickSave();

      await waitFor(() => expect(mailAddAccount).toHaveBeenCalledWith(
        expect.objectContaining({ poll_interval_secs: 60 })
      ));
    });

    // Saving a blank email/host/credential writes the account to config AND the
    // keychain and starts the poller, which then fails every cycle with only a
    // log::warn! and no UI feedback at all.
    //
    // One case per field rather than one cumulative test: a test that fills
    // three fields before asserting "still disabled" only ever witnesses the
    // fourth, so dropping any of the other three from the guard would go
    // undetected.
    it("enables save once every essential field is filled", async () => {
      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_add }));
      expect(screen.getByRole("button", { name: t.save })).toBeDisabled();

      fillRequired();

      expect(screen.getByRole("button", { name: t.save })).toBeEnabled();
    });

    it.each(REQUIRED_FIELDS)("keeps save disabled when %s is the only blank field", async (field) => {
      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_add }));
      fillRequired(field);

      expect(screen.getByRole("button", { name: t.save })).toBeDisabled();
    });

    it.each(REQUIRED_FIELDS)("does not submit when %s is the only blank field", async (field) => {
      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_add }));
      fillRequired(field);

      clickSave();

      expect(mailAddAccount).not.toHaveBeenCalled();
    });

    // Boolean(" ") is true, so an untrimmed guard would let a stray space
    // through and produce exactly the failure the guard exists to prevent.
    it.each(TRIMMED_FIELDS)("keeps save disabled when %s holds only whitespace", async (field) => {
      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_add }));
      fillRequired();
      fireEvent.change(screen.getByLabelText(t[field]), { target: { value: "   " } });

      expect(screen.getByRole("button", { name: t.save })).toBeDisabled();
    });

    // Deliberately asymmetric: leading/trailing whitespace can be legitimate in
    // a password, so it must not be trimmed away when deciding validity.
    it("still allows save when the password is entirely whitespace", async () => {
      renderPage();

      fireEvent.click(await screen.findByRole("button", { name: t.mail_add }));
      fillRequired();
      fireEvent.change(screen.getByLabelText(t.mail_password), { target: { value: "   " } });

      expect(screen.getByRole("button", { name: t.save })).toBeEnabled();
    });

    it("clears the entered password when the form is cancelled", async () => {
      renderPage();
      await fillForm();

      fireEvent.click(screen.getByRole("button", { name: t.cancel }));
      // A live mailbox credential must not sit in component state for as long
      // as SettingsView stays mounted.
      fireEvent.click(screen.getByRole("button", { name: t.mail_add }));

      expect(screen.getByLabelText(t.mail_password)).toHaveValue("");
      expect(screen.getByLabelText(t.mail_email)).toHaveValue("");
    });

    it("clears a previous failure banner when the form is cancelled", async () => {
      vi.mocked(mailAddAccount).mockRejectedValue(new Error("imap unreachable") as never);

      renderPage();
      await fillForm();
      clickSave();
      expect(await screen.findByText(/imap unreachable/)).toBeTruthy();

      fireEvent.click(screen.getByRole("button", { name: t.cancel }));

      expect(screen.queryByText(/imap unreachable/)).toBeNull();
    });

    it("disables the save button while the add is in flight", async () => {
      // Never resolves — the add stays in flight.
      vi.mocked(mailAddAccount).mockReturnValue(new Promise(() => {}) as never);

      renderPage();
      await fillForm();

      clickSave();

      // A second submit would create a duplicate account, so the button must
      // stop accepting clicks until the first one settles.
      const saving = await screen.findByRole("button", { name: t.mail_saving });
      expect(saving).toBeDisabled();
      fireEvent.click(saving);
      expect(mailAddAccount).toHaveBeenCalledTimes(1);
    });

    it("surfaces an add failure and keeps the form open with the entered values", async () => {
      vi.mocked(mailAddAccount).mockRejectedValue(new Error("imap unreachable") as never);

      renderPage();
      await fillForm();

      clickSave();

      expect(await screen.findByText(/imap unreachable/)).toBeTruthy();
      // Form still open, values preserved — retyping eight fields after a
      // transient failure would be miserable.
      expect(screen.getByLabelText(t.mail_email)).toHaveValue("new@example.com");
    });
  });

  describe("notification permission prompt", () => {
    it("requests OS notification permission after a successful add", async () => {
      // Not yet granted: the page must actually raise the prompt, at the moment
      // the user is provably looking at the app — not lazily from a background
      // poll timer, where macOS would present the dialog to an unfocused app.
      vi.mocked(isPermissionGranted).mockResolvedValue(false as never);

      renderPage();
      await fillForm();

      clickSave();

      await waitFor(() => expect(requestPermission).toHaveBeenCalled());
    });

    it("does not re-prompt when permission is already granted", async () => {
      vi.mocked(isPermissionGranted).mockResolvedValue(true as never);

      renderPage();
      await fillForm();

      clickSave();

      await waitFor(() => expect(isPermissionGranted).toHaveBeenCalled());
      expect(requestPermission).not.toHaveBeenCalled();
    });

    it("does not prompt when the add itself failed", async () => {
      vi.mocked(mailAddAccount).mockRejectedValue(new Error("imap unreachable") as never);

      renderPage();
      await fillForm();

      clickSave();

      // Barrier: wait until the failure has actually been rendered, otherwise
      // this would pass merely by asserting before the prompt could fire.
      expect(await screen.findByText(/imap unreachable/)).toBeTruthy();
      expect(isPermissionGranted).not.toHaveBeenCalled();
    });

    it("does not turn a permission failure into an add-account error", async () => {
      // The account WAS added; a permission problem must not be reported as if
      // the add had failed, nor reopen/block the form.
      vi.mocked(isPermissionGranted).mockRejectedValue(new Error("no notif plugin") as never);

      renderPage();
      await fillForm();

      clickSave();

      await waitFor(() => expect(isPermissionGranted).toHaveBeenCalled());
      // Barrier: the add's own success path must have completed (form closed,
      // list refetched) before we can meaningfully assert "no error shown".
      await waitFor(() => expect(mailListAccounts).toHaveBeenCalledTimes(2));
      expect(screen.queryByText(/no notif plugin/)).toBeNull();
      expect(screen.queryByText(new RegExp(t.mail_add_failed))).toBeNull();
    });
  });
});
