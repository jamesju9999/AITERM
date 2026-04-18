import { useState, useCallback } from "react";


export interface AgentMission {
  goal: string;
  active: boolean;
  stepCount: number;
  maxSteps: number;
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
    stopMission,
    clearMission,
  };
}
