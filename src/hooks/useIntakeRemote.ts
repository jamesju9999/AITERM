import { useEffect, useState } from "react";
import { intakeRemoteStatus, onIntakeRemote, type IntakeRemoteStatus } from "../ipc/intake";
import { unlistenOnCleanup } from "../lib/eventSubscription";

export const LOCAL_MODE: IntakeRemoteStatus = { connected: false, online: false, addr: "", name: "", role: null, error: null };

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
