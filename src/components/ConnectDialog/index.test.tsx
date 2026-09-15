import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("../../ipc/shareViewer", () => ({
  shareViewerConnect: vi.fn(),
}));

vi.mock("../../ipc/share", () => ({
  shareDiscover: vi.fn(),
}));

vi.mock("../../ipc/remoteHosts", () => ({
  // **兩個常數都要列。** vi.mock 的 factory 會整個取代模組，漏掉的那個在測試裡
  // 會是 undefined，`msg.includes(undefined)` 永遠不成立——那條錯誤分支在測試
  // 環境裡等於不存在，而且不會有任何錯誤訊息。
  ERR_SAVED_KEY_MISSING: "remote_host_key_missing",
  ERR_KEYCHAIN_UNAVAILABLE: "remote_host_keychain_unavailable",
  remoteHostsList: vi.fn(async () => []),
  remoteHostsAdd: vi.fn(async () => "new-id"),
  remoteHostsUpdate: vi.fn(async () => undefined),
  remoteHostsRemove: vi.fn(async () => undefined),
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { shareViewerConnect } from "../../ipc/shareViewer";
import { shareDiscover } from "../../ipc/share";
import {
  remoteHostsList,
  remoteHostsAdd,
  remoteHostsUpdate,
  remoteHostsRemove,
} from "../../ipc/remoteHosts";
import { ConnectDialog } from "./index";

const onConnected = vi.fn();
const onCancel = vi.fn();

beforeEach(() => {
  vi.mocked(shareViewerConnect).mockReset().mockResolvedValue({ connId: "conn-1", sas: "4917" });
  vi.mocked(shareDiscover).mockReset().mockResolvedValue({ kind: "notFound" });
  vi.mocked(remoteHostsList).mockReset().mockResolvedValue([]);
  vi.mocked(remoteHostsAdd).mockReset().mockResolvedValue("new-id");
  vi.mocked(remoteHostsUpdate).mockReset().mockResolvedValue(undefined);
  vi.mocked(remoteHostsRemove).mockReset().mockResolvedValue(undefined);
  onConnected.mockReset();
  onCancel.mockReset();
});

describe("ConnectDialog", () => {
  it("keeps the manual address field out of the way at first", () => {
    // 平常乾淨；出事時才把退路攤開（見 spec 的「觀看端」）。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    expect(screen.queryByPlaceholderText(/192\.168/)).not.toBeInTheDocument();
  });

  it("reveals the manual address field on demand", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
  });

  it("connects with an address and a key but no short code", async () => {
    // 金鑰模式**一定**沒有短碼——身分完全由金鑰決定，觀看端連送出去的
    // `code` 都是空字串。實機回報的 bug：送出鈕的啟用條件寫死成「短碼剛好
    // 6 位」，所以金鑰填好了按鈕還是灰的，整條 CLI host 的路完全走不通。
    //
    // 既有的每一條對話框測試都會順手打一組 6 位短碼，所以它們全綠也抓不到
    // 這件事——那正是這條測試存在的理由。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "127.0.0.1:18022");
    await userEvent.type(screen.getByLabelText(/你的名字/), "Bob");
    await userEvent.type(screen.getByLabelText(/金鑰/), "ab".repeat(32));

    const submit = screen.getByRole("button", { name: /^連線$/ });
    expect(submit).toBeEnabled();
    await userEvent.click(submit);

    expect(shareViewerConnect).toHaveBeenCalledWith({
      host: "127.0.0.1",
      port: 18022,
      code: "",
      displayName: "Bob",
      key: "ab".repeat(32),
      savedHostId: undefined,
    });
  });

  it("still requires a short code when no key is given", async () => {
    // 反面：沒有金鑰時，送出鈕仍然必須等到短碼滿 6 位才能按。放寬成「有位址
    // 就能按」會讓短碼模式在還沒填完碼時就送出去。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "127.0.0.1:18022");
    expect(screen.getByRole("button", { name: /^連線$/ })).toBeDisabled();
  });

  it("connects with a manually entered host and port", async () => {
    // 2C 的 mDNS 還沒上線，手動位址是這個階段唯一的路——也是永遠可用的
    // 主路徑（見 spec 的決策紀錄）。
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/你的名字/), "Bob");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(shareViewerConnect).toHaveBeenCalledWith({
      host: "192.168.1.33",
      port: 47823,
      code: "559207",
      displayName: "Bob",
      key: undefined,
      savedHostId: undefined,
    });
    expect(onConnected).toHaveBeenCalledWith("conn-1", "4917", "192.168.1.33:47823");
  });

  it("rejects an address that is not host:port", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "just-a-hostname");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/位址格式不對/)).toBeInTheDocument();
    expect(shareViewerConnect).not.toHaveBeenCalled();
  });

  it("shows why connecting failed instead of closing silently", async () => {
    vi.mocked(shareViewerConnect).mockRejectedValue("連不上 192.168.1.33:47823");
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/6 位數/), "559207");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/連不上/)).toBeInTheDocument();
    expect(onConnected).not.toHaveBeenCalled();
  });

  it("connects straight through when mDNS finds exactly one match", async () => {
    vi.mocked(shareDiscover).mockResolvedValue({ kind: "found", host: "192.168.1.50", port: 9000 });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(shareDiscover).toHaveBeenCalledWith("632706");
    expect(shareViewerConnect).toHaveBeenCalledWith({
      host: "192.168.1.50",
      port: 9000,
      code: "632706",
      displayName: "AITerm",
      key: undefined,
      savedHostId: undefined,
    });
    expect(onConnected).toHaveBeenCalledWith("conn-1", "4917", "192.168.1.50:9000");
  });

  it("falls back to the manual address field when mDNS finds nothing", async () => {
    vi.mocked(shareDiscover).mockResolvedValue({ kind: "notFound" });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/找不到這組編號/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
    expect(shareViewerConnect).not.toHaveBeenCalled();
  });

  it("shows a distinct message when mDNS finds more than one match", async () => {
    vi.mocked(shareDiscover).mockResolvedValue({ kind: "ambiguous" });
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(await screen.findByText(/不只一台機器/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/192\.168/)).toBeInTheDocument();
    expect(shareViewerConnect).not.toHaveBeenCalled();
  });

  it("skips mDNS entirely once the manual address field has something in it", async () => {
    render(<ConnectDialog onConnected={onConnected} onCancel={onCancel} />);
    await userEvent.click(screen.getByText(/直接輸入位址/));
    await userEvent.type(screen.getByPlaceholderText(/192\.168/), "192.168.1.33:47823");
    await userEvent.type(screen.getByLabelText(/6 位數/), "632706");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));

    expect(shareDiscover).not.toHaveBeenCalled();
    expect(shareViewerConnect).toHaveBeenCalledWith({
      host: "192.168.1.33",
      port: 47823,
      code: "632706",
      displayName: "AITerm",
      key: undefined,
      savedHostId: undefined,
    });
  });
});

describe("ConnectDialog 地址簿", () => {
  beforeEach(() => {
    vi.mocked(remoteHostsList).mockResolvedValue([
      { id: "a", name: "辦公室", host: "192.168.1.50", port: 8022, has_key: true },
    ]);
    vi.mocked(shareViewerConnect).mockResolvedValue({ connId: "c1", sas: "1234" });
  });

  it("點清單的一筆時只送 savedHostId，不送 key", async () => {
    // 金鑰不跨 IPC 是整個設計的核心。前端若自己帶 key，代表它某處拿得到
    // 已存的金鑰——那就是這個設計要避免的事。
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByText("辦公室"));
    expect(shareViewerConnect).toHaveBeenCalledTimes(1);
    const args = vi.mocked(shareViewerConnect).mock.calls[0][0];
    expect(args.savedHostId).toBe("a");
    expect(args.key).toBeUndefined();
  });

  it("從地址簿連上之後不再問要不要儲存", async () => {
    const onConnected = vi.fn();
    render(<ConnectDialog onConnected={onConnected} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByText("辦公室"));
    expect(screen.queryByText(/存進地址簿/)).toBeNull();
    expect(onConnected).toHaveBeenCalledWith("c1", "1234", "192.168.1.50:8022");
  });

  it("手動輸入金鑰連上之後會問要不要儲存，按儲存才寫入", async () => {
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(screen.getByText(/▸/));
    await userEvent.type(screen.getByLabelText(/位址|Address/), "10.0.0.9:9000");
    await userEvent.type(screen.getByLabelText(/金鑰|Key/), "deadbeef");
    await userEvent.click(screen.getByRole("button", { name: /連線|Connect$/ }));
    await screen.findByText(/存進地址簿/);
    await userEvent.type(screen.getByLabelText(/別名|^Name$/), "雲端");
    await userEvent.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    expect(remoteHostsAdd).toHaveBeenCalledWith({
      name: "雲端",
      host: "10.0.0.9",
      port: 9000,
      secret: "deadbeef",
    });
  });

  it("按「不用」仍然會開分頁", async () => {
    // 不存 ≠ 丟掉這條已經建立好的連線。
    const onConnected = vi.fn();
    render(<ConnectDialog onConnected={onConnected} onCancel={vi.fn()} />);
    await userEvent.click(screen.getByText(/▸/));
    await userEvent.type(screen.getByLabelText(/位址|Address/), "10.0.0.9:9000");
    await userEvent.type(screen.getByLabelText(/金鑰|Key/), "deadbeef");
    await userEvent.click(screen.getByRole("button", { name: /連線|Connect$/ }));
    await userEvent.click(await screen.findByRole("button", { name: /^不用$|Not now/ }));
    expect(remoteHostsAdd).not.toHaveBeenCalled();
    expect(onConnected).toHaveBeenCalledWith("c1", "1234", "10.0.0.9:9000");
  });

  it("金鑰不在這台電腦上時，展開手動欄位並帶入位址讓使用者重貼", async () => {
    // 這條路徑存在的理由：不處理的話後端的錯誤會原樣顯示，而且使用者不知道
    // 要做什麼。帶入位址是為了讓他只需要貼金鑰那一欄。
    vi.mocked(shareViewerConnect).mockRejectedValueOnce(new Error("remote_host_key_missing"));
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByText("辦公室"));
    expect(await screen.findByDisplayValue("192.168.1.50:8022")).toBeInTheDocument();
    expect(screen.getByText(/金鑰不在這台電腦上|not on this computer/)).toBeInTheDocument();
  });

  it("keychain 讀不到時不叫使用者重貼金鑰", async () => {
    // **跟上一條的處置相反。** 金鑰是好的，是金鑰圈打不開，重貼幾次都沒用，
    // 所以不該展開手動欄位。後端送的形狀是 `<常數>: <底層原因>`，所以比對
    // 必須是 includes/startsWith 而不是相等。
    vi.mocked(shareViewerConnect).mockRejectedValueOnce(
      new Error("remote_host_keychain_unavailable: keychain locked"),
    );
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByText("辦公室"));
    expect(
      await screen.findByText(/讀不到系統金鑰圈|Could not read the system keychain/),
    ).toBeInTheDocument();
    expect(screen.queryByDisplayValue("192.168.1.50:8022")).toBeNull();
  });

  it("刪除要先確認，按確認才真的刪", async () => {
    // 刪除會連 keychain 的金鑰一起刪掉，一按就生效太危險。
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByRole("button", { name: /^刪除$|^Delete$/ }));
    expect(remoteHostsRemove).not.toHaveBeenCalled();
    // 用確認句本身當同步點，不是純粹的「辦公室」——那個字同時出現在清單列
    // 跟確認句裡（確認句用 {name} 內插進去），拿它當 findByText 的目標會撞成
    // 「找到兩個元素」。
    await screen.findByText(/確定要從地址簿刪除/);
    const buttons = screen.getAllByRole("button", { name: /^刪除$|^Delete$/ });
    await userEvent.click(buttons[buttons.length - 1]);
    expect(remoteHostsRemove).toHaveBeenCalledWith("a");
  });

  it("短碼模式連上之後不問儲存", async () => {
    // 短碼每次都不一樣，存起來沒有意義。
    vi.mocked(shareDiscover).mockResolvedValue({
      kind: "found",
      host: "1.2.3.4",
      port: 8022,
    });
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    // 標籤實際文字是「6 位數」/「6-digit」，不是「短碼」——跟既有測試（如
    // "connects straight through when mDNS finds exactly one match"）用同一個
    // matcher，避免自己編了一個對不上真實文案的正則。
    await userEvent.type(screen.getByLabelText(/6 位數|6-digit/), "123456");
    await userEvent.click(screen.getByRole("button", { name: /連線|Connect$/ }));
    await waitFor(() => expect(shareViewerConnect).toHaveBeenCalled());
    expect(screen.queryByText(/存進地址簿/)).toBeNull();
  });

  it("編輯一筆時就算沒有重新輸入金鑰，送出鈕仍會啟用並呼叫 remoteHostsUpdate", async () => {
    // 這條測試只驗證「別名/位址修改可以送出並更新」這件事本身——不驗證
    // editingId 有沒有被清掉。清掉與否是另一條測試
    // （「不用」之後 editingId 不會殘留到下一次全新連線）的職責；混在同一條
    // 測試裡會讓名字承諾了測試沒有真的檢查的東西。
    //
    // 這是計畫本身沒蓋到的洞：editHost 只設定 editingId，不強迫使用者重打
    // 金鑰（後端把空字串當成「不改金鑰」）。如果送出鈕的啟用條件只看
    // 「有沒有打金鑰」，這種「只改別名/位址」的編輯會永遠停在灰色送不出去。
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByRole("button", { name: /^編輯$/ }));
    const submit = screen.getByRole("button", { name: /^連線$/ });
    expect(submit).toBeEnabled();
    await userEvent.click(submit);
    await screen.findByText(/存進地址簿/);
    await userEvent.click(screen.getByRole("button", { name: /^儲存$/ }));
    expect(remoteHostsUpdate).toHaveBeenCalledWith({
      id: "a",
      name: "192.168.1.50:8022",
      host: "192.168.1.50",
      port: 8022,
      secret: "",
    });
  });

  it("「不用」之後 editingId 不會殘留到下一次全新的手動連線", async () => {
    // 這是「編輯一筆時就算沒有重新輸入金鑰…」那條測試沒有蓋到的洞：那條只
    // 斷言 remoteHostsUpdate 被呼叫，從來沒有做任何後續動作去暴露
    // editingId 有沒有真的被清掉——把 finishPending 裡的 setEditingId(null)
    // 拿掉，那條測試依然全線通過。
    //
    // 真正會分勝負的動作序列是：先走一次編輯＋連線＋「不用」（跳過存檔，
    // 最容易忘記清狀態的路徑），**同一個對話框元件不重新掛載**，接著做一次
    // 完全獨立的手動連線並存檔。如果 editingId 殘留，第二次存檔會誤走
    // remoteHostsUpdate（帶著舊的 id "a"）而不是 remoteHostsAdd——這正是
    // 會覆蓋別人條目的那種資料損毀。
    //
    // 目前唯一擋住這件事的是 TerminalApp.tsx 在每次 onConnected 之後把
    // ConnectDialog 整個卸載重掛，屬於呼叫端的行為，不是這個元件自己的
    // 保證——所以這裡刻意不卸載，直接測元件本身夠不夠格自己守住這條規則。
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);

    // 第一段：編輯「辦公室」、金鑰留空、連線、按「不用」跳過存檔。
    await userEvent.click(await screen.findByRole("button", { name: /^編輯$/ }));
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));
    await userEvent.click(await screen.findByRole("button", { name: /^不用$/ }));
    expect(remoteHostsUpdate).not.toHaveBeenCalled();

    // 第二段：跟第一段完全無關的全新手動連線——換掉位址與金鑰再送出。
    const addressField = screen.getByLabelText(/位址|Address/);
    await userEvent.clear(addressField);
    await userEvent.type(addressField, "10.0.0.9:9000");
    const keyField = screen.getByLabelText(/金鑰|Key/);
    await userEvent.clear(keyField);
    await userEvent.type(keyField, "deadbeef");
    await userEvent.click(screen.getByRole("button", { name: /^連線$/ }));
    await screen.findByText(/存進地址簿/);
    await userEvent.click(screen.getByRole("button", { name: /^儲存$/ }));

    expect(remoteHostsAdd).toHaveBeenCalledWith({
      name: "10.0.0.9:9000",
      host: "10.0.0.9",
      port: 9000,
      secret: "deadbeef",
    });
    expect(remoteHostsUpdate).not.toHaveBeenCalled();
  });

  it("編輯一筆在別台裝置已經被刪掉的條目：重整清單並提示，不會直接送出", async () => {
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByRole("button", { name: /^編輯$/ });
    // editHost 會在點下去的當下重抓一次清單；模擬那次重抓發現這筆已經不見了。
    vi.mocked(remoteHostsList).mockResolvedValueOnce([]);
    await userEvent.click(screen.getByRole("button", { name: /^編輯$/ }));
    expect(await screen.findByText(/已經被移除/)).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/192\.168/)).not.toBeInTheDocument();
    expect(screen.queryByText("辦公室")).toBeNull();
  });
});
