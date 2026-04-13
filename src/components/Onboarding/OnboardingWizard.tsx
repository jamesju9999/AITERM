import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProviderForm } from "../Settings/ProviderForm";
import { addProvider } from "../../ipc/provider";
import { setExecutionMode, setOnboardingDone } from "../../ipc/config";
import type { ExecutionMode } from "../../ipc/config";
import type { ProviderInput } from "../../ipc/provider";
import "./OnboardingWizard.css";

type Step = 1 | 2 | 3;

const MODES: { value: ExecutionMode; label: string; desc: string }[] = [
  {
    value: "always-confirm",
    label: "一律確認（推薦）",
    desc: "所有 AI 命令都需要確認後才執行，最安全的選擇。",
  },
  {
    value: "graded",
    label: "分級自動",
    desc: "無風險命令（如 ls）自動執行，其他命令仍需確認。",
  },
  {
    value: "full-auto",
    label: "全自動 Agent",
    desc: "大多數命令自動執行，只有危險命令才詢問確認。",
  },
];

export function OnboardingWizard() {
  const navigate = useNavigate();
  const [step, setStep] = useState<Step>(1);
  const [mode, setMode] = useState<ExecutionMode>("always-confirm");
  const [finishing, setFinishing] = useState(false);
  const [providerAdded, setProviderAdded] = useState(false);

  const handleAddProvider = async (input: ProviderInput) => {
    await addProvider(input);
    setProviderAdded(true);
    setStep(3);
  };

  const handleFinish = async () => {
    setFinishing(true);
    try {
      await setExecutionMode(mode);
      await setOnboardingDone();
      navigate("/");
    } finally {
      setFinishing(false);
    }
  };

  return (
    <div className="onboarding-overlay">
      <div className="onboarding-card">
        {/* Step indicator */}
        <div className="step-indicator">
          {([1, 2, 3] as Step[]).map((s) => (
            <div
              key={s}
              className={`step-dot ${step === s ? "step-dot--active" : ""} ${step > s ? "step-dot--done" : ""}`}
            />
          ))}
          <span className="step-label">{step} / 3</span>
        </div>

        {/* Step 1: Welcome */}
        {step === 1 && (
          <div className="onboarding-step">
            <div className="onboarding-icon">⚡</div>
            <h1>歡迎使用 AITerm</h1>
            <p className="onboarding-subtitle">
              由 AI 驅動的跨平台終端機，讓命令列操作更直覺
            </p>
            <ul className="feature-list">
              <li>
                <span className="feature-icon">💬</span>
                用自然語言描述你想做的事，AI 自動產生命令
              </li>
              <li>
                <span className="feature-icon">🔌</span>
                支援 OpenAI、Anthropic、Ollama 等多個 AI 後端
              </li>
              <li>
                <span className="feature-icon">🛡️</span>
                內建命令安全分級，危險命令不會被意外執行
              </li>
            </ul>
            <button
              className="btn-primary btn-large"
              onClick={() => setStep(2)}
            >
              開始設定
            </button>
          </div>
        )}

        {/* Step 2: Add first provider */}
        {step === 2 && (
          <div className="onboarding-step">
            <h2>新增你的第一個 AI Provider</h2>
            <p className="onboarding-subtitle">
              選擇你要使用的 AI 服務並填入相關設定
            </p>
            <div className="form-wrapper">
              <ProviderForm
                onSave={handleAddProvider}
                onCancel={() => setStep(3)}
              />
            </div>
            <button className="btn-skip" onClick={() => setStep(3)}>
              稍後再設定
            </button>
          </div>
        )}

        {/* Step 3: Choose execution mode */}
        {step === 3 && (
          <div className="onboarding-step">
            <h2>選擇執行模式</h2>
            <p className="onboarding-subtitle">
              決定 AI 產出的命令要如何被執行，之後可在設定頁更改。
            </p>
            <div className="mode-list">
              {MODES.map((m) => (
                <label key={m.value} className="mode-option">
                  <input
                    type="radio"
                    name="exec_mode"
                    value={m.value}
                    checked={mode === m.value}
                    onChange={() => setMode(m.value)}
                  />
                  <div className="mode-text">
                    <span className="mode-label">{m.label}</span>
                    <span className="mode-desc">{m.desc}</span>
                  </div>
                </label>
              ))}
            </div>
            {!providerAdded && (
              <p className="notice">
                你尚未設定 Provider，可以在設定頁中隨時新增。
              </p>
            )}
            <button
              className="btn-primary btn-large"
              onClick={handleFinish}
              disabled={finishing}
            >
              {finishing ? "完成中…" : "完成設定，開始使用！"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
