export const ptyDataEvent = (sessionId: string): string =>
  `pty://data/${sessionId}`;

export const ptyClosedEvent = (sessionId: string): string =>
  `pty://closed/${sessionId}`;

export interface PtyDataPayload {
  base64: string;
}

export interface PtyClosedPayload {
  reason: string;
}
