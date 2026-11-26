import { useEffect, useState } from "react";
import { intakeRemoteConnections, intakeRemoteStatus, onIntakeRemote, type IntakeRemoteConnection, type IntakeRemoteStatus } from "../ipc/intake";
import { unlistenOnCleanup } from "../lib/eventSubscription";

export const LOCAL_MODE: IntakeRemoteStatus = { id: "", connected: false, online: false, addr: "", name: "", role: null, error: null };

/** 問題收集目前是本機模式還是連到 server；連線狀態變動時自動更新。 */
export function useIntakeRemote(): IntakeRemoteStatus {
  const [status, setStatus] = useState<IntakeRemoteStatus>(LOCAL_MODE);
  useEffect(() => {
    let alive = true;
    intakeRemoteStatus()
      .then((s) => alive && setStatus(s))
      .catch(() => {});
    const off = unlistenOnCleanup(
      onIntakeRemote((s) => {
        if (alive) setStatus(s);
      }),
      "intake://remote",
    );
    return () => {
      alive = false;
      off();
    };
  }, []);
  return status;
}

/** 已記錄的 server 清單；連線狀態一變（連上、切換、中斷、刪除）就重抓。 */
export function useIntakeConnections(): IntakeRemoteConnection[] {
  const [list, setList] = useState<IntakeRemoteConnection[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () =>
      intakeRemoteConnections()
        .then((l) => alive && setList(l))
        .catch(() => {});
    void load();
    const off = unlistenOnCleanup(onIntakeRemote(() => void load()), "intake://remote");
    return () => {
      alive = false;
      off();
    };
  }, []);
  return list;
}
