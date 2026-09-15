import { describe, expect, it, vi } from "vitest";
import { render, screen, within, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RemoteHostList } from "./RemoteHostList";
import { LocaleProvider } from "../../contexts/LocaleContext";

const hosts = [
  { id: "a", name: "辦公室", host: "192.168.1.50", port: 8022, has_key: true },
  { id: "b", name: "雲端", host: "10.0.0.9", port: 9000, has_key: false },
];

function renderList(props: Partial<Parameters<typeof RemoteHostList>[0]> = {}) {
  return render(
    <LocaleProvider>
      <RemoteHostList
        hosts={props.hosts ?? hosts}
        onConnect={props.onConnect ?? vi.fn()}
        onEdit={props.onEdit ?? vi.fn()}
        onDelete={props.onDelete ?? vi.fn()}
        disabled={props.disabled}
      />
    </LocaleProvider>,
  );
}

describe("RemoteHostList", () => {
  it("每一筆都看得到別名與位址", () => {
    renderList();
    expect(screen.getByText("辦公室")).toBeInTheDocument();
    expect(screen.getByText("192.168.1.50:8022")).toBeInTheDocument();
    expect(screen.getByText("雲端")).toBeInTheDocument();
    expect(screen.getByText("10.0.0.9:9000")).toBeInTheDocument();
  });

  it("點一列就帶著那一筆呼叫 onConnect", async () => {
    const onConnect = vi.fn();
    renderList({ onConnect });
    await userEvent.click(screen.getByText("辦公室"));
    expect(onConnect).toHaveBeenCalledTimes(1);
    expect(onConnect.mock.calls[0][0].id).toBe("a");
  });

  it("金鑰不在這台電腦上的那一筆會標示出來，有金鑰的那一筆不標", () => {
    // 不標的話使用者只會看到連線失敗，不知道要重貼金鑰。
    renderList();
    const rows = screen.getAllByRole("listitem");
    expect(within(rows[1]).queryByTestId("no-key")).not.toBeNull();
    expect(within(rows[0]).queryByTestId("no-key")).toBeNull();
  });

  it("沒有任何一筆時整塊都不渲染", () => {
    renderList({ hosts: [] });
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("點編輯鈕只呼叫 onEdit，不會連帶觸發 onConnect", async () => {
    // 邊按鈕跟連線按鈕是手足關係、不是巢狀——但值得專門一條測試釘住，
    // 不然一旦有人把結構改成巢狀（例如把整列包成一個大 button），
    // 點編輯會意外連帶觸發連線。
    const onConnect = vi.fn();
    const onEdit = vi.fn();
    renderList({ onConnect, onEdit });
    const rows = screen.getAllByRole("listitem");
    const editButton = within(rows[0]).getByText("編輯");
    await userEvent.click(editButton);
    expect(onEdit).toHaveBeenCalledTimes(1);
    expect(onEdit.mock.calls[0][0].id).toBe("a");
    expect(onConnect).not.toHaveBeenCalled();
  });

  it("disabled 時整排都不能按，點了也不會觸發 onConnect", async () => {
    // 手動連線送出後、結果還沒回來之前，使用者理論上可以再點一筆已存主機，
    // 兩個 shareViewerConnect 同時飛出去——跟 ConnectDialog 送出鈕的
    // disabled={busy || ...} 是同一條規則，這裡也要守住。
    const onConnect = vi.fn();
    renderList({ onConnect, disabled: true });
    const rows = screen.getAllByRole("listitem");
    for (const row of rows) {
      for (const button of within(row).getAllByRole("button")) {
        expect(button).toBeDisabled();
      }
    }
    // 用 fireEvent 而不是 userEvent.click：userEvent 會自己檢查
    // pointer-events/disabled 而直接跳過，測不出「就算真的送出點擊事件，
    // React 也不會呼叫 handler」這件事。
    fireEvent.click(screen.getByText("辦公室"));
    expect(onConnect).not.toHaveBeenCalled();
  });

  // 刪除確認是這個元件自己的契約，不再是父元件的責任——ConnectDialog 那邊的
  // 整合測試照樣涵蓋接線，這裡鎖的是「onDelete 只在確認後才會被呼叫」本身。
  describe("刪除確認", () => {
    it("按刪除不會立刻呼叫 onDelete，要按確認列的刪除才會", async () => {
      const onDelete = vi.fn();
      renderList({ onDelete });
      const rows = screen.getAllByRole("listitem");
      await userEvent.click(within(rows[0]).getByRole("button", { name: /^刪除$/ }));
      expect(onDelete).not.toHaveBeenCalled();

      const confirm = screen.getByText(/確定要從地址簿刪除「辦公室」/).parentElement!;
      await userEvent.click(within(confirm).getByRole("button", { name: /^刪除$/ }));
      expect(onDelete).toHaveBeenCalledTimes(1);
      expect(onDelete.mock.calls[0][0].id).toBe("a");
    });

    it("按取消收起確認列，而且不呼叫 onDelete", async () => {
      const onDelete = vi.fn();
      renderList({ onDelete });
      const rows = screen.getAllByRole("listitem");
      await userEvent.click(within(rows[1]).getByRole("button", { name: /^刪除$/ }));
      const confirm = screen.getByText(/確定要從地址簿刪除「雲端」/).parentElement!;
      await userEvent.click(within(confirm).getByRole("button", { name: /^取消$/ }));
      expect(onDelete).not.toHaveBeenCalled();
      expect(screen.queryByText(/確定要從地址簿刪除/)).toBeNull();
    });

    it("onDelete 完成之後才收起確認列", async () => {
      // 搬進來之前父元件是 `await remoteHostsRemove(); setConfirmDelete(null)`，
      // 這裡保持同一個時序：刪除還在進行時確認列仍在，不會提早消失讓人以為
      // 已經刪好了。
      let finish!: () => void;
      const onDelete = vi.fn(() => new Promise<void>((r) => { finish = r; }));
      renderList({ onDelete });
      await userEvent.click(within(screen.getAllByRole("listitem")[0]).getByRole("button", { name: /^刪除$/ }));
      const confirm = screen.getByText(/確定要從地址簿刪除/).parentElement!;
      await userEvent.click(within(confirm).getByRole("button", { name: /^刪除$/ }));
      expect(screen.queryByText(/確定要從地址簿刪除/)).not.toBeNull();
      finish();
      await screen.findByText("辦公室"); // 讓 promise 的後續更新落地
      await vi.waitFor(() => expect(screen.queryByText(/確定要從地址簿刪除/)).toBeNull());
    });
  });
});
