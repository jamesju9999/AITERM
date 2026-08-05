import { invoke } from "@tauri-apps/api/core";

export interface MailAccount {
  id: string;
  email: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  username: string;
  poll_interval_secs: number;
}

export interface MailAccountInput {
  email: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  username: string;
  password: string;
  poll_interval_secs?: number;
}

export interface MailMessage {
  id: string;
  account_id: string;
  uid: number;
  sender: string;
  subject: string;
  date: string | null;
  body_text: string;
  ai_summary: string | null;
  is_important: boolean;
  is_promotional: boolean;
  is_read_locally: boolean;
  fetched_at: string;
}

export type MailSyncEvent =
  | { kind: "summary"; account_id: string; message_id: string }
  | { kind: "important"; account_id: string; message_id: string; subject: string; summary: string }
  // Cached messages disappeared — deleted or archived on the server, or dropped
  // wholesale because the mailbox's UIDVALIDITY changed. Carries `account_id`
  // like the others, which is all MailView's refetch and useMailSync's badge
  // refresh need to reflect a removal.
  | { kind: "removed"; account_id: string; removed_count: number };

export const MAIL_SYNC_EVENT = "mail-sync-event";

/** IMAP LOGIN + SELECT INBOX round-trip. SMTP is not exercised. */
export function mailTestConnection(input: MailAccountInput): Promise<void> {
  return invoke<void>("mail_test_connection", { input });
}

export function mailAddAccount(input: MailAccountInput): Promise<MailAccount> {
  return invoke<MailAccount>("mail_add_account", { input });
}

export function mailRemoveAccount(id: string): Promise<void> {
  return invoke<void>("mail_remove_account", { id });
}

export function mailListAccounts(): Promise<MailAccount[]> {
  return invoke<MailAccount[]>("mail_list_accounts");
}

export function mailListMessages(accountId: string): Promise<MailMessage[]> {
  return invoke<MailMessage[]>("mail_list_messages", { accountId });
}

export function mailMarkRead(messageId: string): Promise<void> {
  return invoke<void>("mail_mark_read", { messageId });
}

export function mailCountUnread(): Promise<number> {
  return invoke<number>("mail_count_unread");
}
