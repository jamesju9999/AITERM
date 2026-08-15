import { useState, useCallback } from "react";


export interface AgentMission {
  goal: string;
  active: boolean;
  stepCount: number;
  maxSteps: number;
  tokensUsed: number;
  history: {
    command: string;
    exitCode: number;
    output: string;
  }[];
}

export function useAgentMission() {
  const [agentMission, setAgentMission] = useState<AgentMission | null>(null);

  const startMission = useCallback((goal: string, maxSteps = 5) => {
    setAgentMission({
      goal,
      active: true,
      stepCount: 0,
      maxSteps,
      tokensUsed: 0,
      history: [],
    });
  }, []);

  const appendHistory = useCallback((command: string, exitCode: number, output: string) => {
    setAgentMission((prev) => {
      if (!prev || !prev.active) return prev;
      return {
        ...prev,
        stepCount: prev.stepCount + 1,
        history: [...prev.history, { command, exitCode, output }],
      };
    });
  }, []);

  const addTokens = useCallback((n: number) => {
    setAgentMission((prev) => {
      if (!prev || !prev.active) return prev;
      return { ...prev, tokensUsed: prev.tokensUsed + n };
    });
  }, []);

  const stopMission = useCallback(() => {
    setAgentMission((prev) => (prev ? { ...prev, active: false } : null));
  }, []);

  const clearMission = useCallback(() => {
    setAgentMission(null);
  }, []);

  return {
    agentMission,
    startMission,
    appendHistory,
    addTokens,
    stopMission,
    clearMission,
  };
}
