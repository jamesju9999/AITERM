import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
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
});
