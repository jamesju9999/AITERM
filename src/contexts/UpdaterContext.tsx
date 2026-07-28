import { createContext, useContext, type ReactNode } from "react";
import { useUpdater, type UpdaterApi } from "../hooks/useUpdater";

const UpdaterContext = createContext<UpdaterApi | null>(null);

export function UpdaterProvider({ children }: { children: ReactNode }) {
  const updater = useUpdater();
  return <UpdaterContext.Provider value={updater}>{children}</UpdaterContext.Provider>;
}

/**
 * Returns an inert updater when no provider is mounted, matching useLocale's
 * fallback so components stay renderable in isolation (tests, storybook-style use).
 */
export function useUpdaterContext(): UpdaterApi {
  const ctx = useContext(UpdaterContext);
  if (!ctx) {
    return {
      state: { status: "idle" },
      hasUpdate: false,
      dismissed: false,
      check: async () => {},
      install: async () => {},
      relaunch: async () => {},
      dismiss: () => {},
    };
  }
  return ctx;
}
