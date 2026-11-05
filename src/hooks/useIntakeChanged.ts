import { useEffect, useRef } from "react";
import { onIntakeChanged } from "../ipc/intake";
import { unlistenOnCleanup } from "../lib/eventSubscription";

/** 後端佇列有變動時呼叫 `cb`。用 ref 存最新的 `cb`，訂閱本身只掛一次。 */
export function useIntakeChanged(cb: () => void) {
  const ref = useRef(cb);
  useEffect(() => {
    ref.current = cb;
  });
  useEffect(() => {
    let alive = true;
    const pending = onIntakeChanged(() => {
      if (alive) ref.current();
    });
    const off = unlistenOnCleanup(pending, "intake://changed");
    return () => {
      alive = false;
      off();
    };
  }, []);
}
