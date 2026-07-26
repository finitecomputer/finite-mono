"use client";

import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

const AGENT_ONBOARDING_STAGES = [
  "profile",
  "access",
  "launch",
  "ready",
] as const;

export type AgentOnboardingStage = (typeof AGENT_ONBOARDING_STAGES)[number];

type AgentOnboardingStageContextValue = {
  stage: AgentOnboardingStage;
  setStage: (stage: AgentOnboardingStage) => void;
};

const AgentOnboardingStageContext =
  createContext<AgentOnboardingStageContextValue | null>(null);

const AGENT_ONBOARDING_STAGE_LABELS: Record<AgentOnboardingStage, string> = {
  profile: "Profile",
  access: "Access",
  launch: "Launch",
  ready: "Ready",
};

export function AgentOnboardingStageProvider({
  children,
  initialStage,
}: {
  children: ReactNode;
  initialStage: AgentOnboardingStage;
}) {
  const [stage, setStage] = useState(initialStage);
  const value = useMemo(() => ({ stage, setStage }), [stage]);

  return (
    <AgentOnboardingStageContext.Provider value={value}>
      {children}
    </AgentOnboardingStageContext.Provider>
  );
}

export function useAgentOnboardingStage() {
  return useContext(AgentOnboardingStageContext);
}

export function AgentOnboardingStageSync({
  stage,
}: {
  stage: AgentOnboardingStage;
}) {
  const context = useAgentOnboardingStage();
  const setStage = context?.setStage;

  useEffect(() => {
    setStage?.(stage);
  }, [setStage, stage]);

  return null;
}

export function AgentOnboardingProgress() {
  const context = useAgentOnboardingStage();
  const stage = context?.stage ?? "profile";
  const stageIndex = AGENT_ONBOARDING_STAGES.indexOf(stage);

  return (
    <ol
      className="grid w-32 grid-cols-4 gap-1 sm:w-40"
      aria-label="Agent setup progress"
    >
      {AGENT_ONBOARDING_STAGES.map((item, index) => (
        <li
          key={item}
          className={`h-0.5 rounded-full ${
            index <= stageIndex ? "bg-foreground" : "bg-border"
          }`}
          aria-current={index === stageIndex ? "step" : undefined}
        >
          <span className="sr-only">
            {AGENT_ONBOARDING_STAGE_LABELS[item]}:{" "}
            {index < stageIndex
              ? "complete"
              : index === stageIndex
                ? "current"
                : "upcoming"}
          </span>
        </li>
      ))}
    </ol>
  );
}

export function agentOnboardingStageFromSearchParams(
  searchParams: Pick<URLSearchParams, "get">
): AgentOnboardingStage {
  if (searchParams.get("creation")) {
    return "launch";
  }
  if (searchParams.get("billing")) {
    return "access";
  }
  return "profile";
}
