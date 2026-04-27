import { useState, useEffect, useCallback, useRef } from "react";
import { listenTelegramMessage, sendTelegramMessage } from "../ipc/telegram";

export function useTelegramRemoteControl(
  _tabId: string,
  isActive: boolean,
  onMessageReceived: (text: string) => void
) {
  const [isRemoteEnabled, setIsRemoteEnabled] = useState(false);

  const onMessageReceivedRef = useRef(onMessageReceived);
  useEffect(() => {
    onMessageReceivedRef.current = onMessageReceived;
  }, [onMessageReceived]);

  useEffect(() => {
    if (!isRemoteEnabled || !isActive) return;

    let unlisten: (() => void) | undefined;
    
    listenTelegramMessage((payload) => {
      onMessageReceivedRef.current(payload.text);
    }).then((un) => {
      unlisten = un;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, [isRemoteEnabled, isActive]);

  const sendRemoteResponse = useCallback(
    async (text: string) => {
      if (isRemoteEnabled) {
        try {
          await sendTelegramMessage(text);
        } catch (e) {
          console.error("Failed to send telegram message:", e);
        }
      }
    },
    [isRemoteEnabled]
  );

  return {
    isRemoteEnabled,
    setIsRemoteEnabled,
    sendRemoteResponse,
  };
}
