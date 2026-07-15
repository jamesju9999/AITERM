import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ProviderForm } from "../Settings/ProviderForm";
import { addProvider } from "../../ipc/provider";
import { setExecutionMode, setOnboardingDone } from "../../ipc/config";
import type { ExecutionMode } from "../../ipc/config";
import type { ProviderInput } from "../../ipc/provider";
import { useLocale } from "../../contexts/LocaleContext";
import "./OnboardingWizard.css";

type Step = 1 | 2 | 3;

export function OnboardingWizard() {
  const navigate = useNavigate();
  const { t } = useLocale();
  const [step, setStep] = useState<Step>(1);
  const [mode, setMode] = useState<ExecutionMode>("always-confirm");
  const [finishing, setFinishing] = useState(false);
  const [providerAdded, setProviderAdded] = useState(false);

  const MODES: { value: ExecutionMode; label: string; desc: string }[] = [
    {
      value: "always-confirm",
      label: t.ob_mode_always_confirm,
      desc: t.ob_mode_always_confirm_desc,
    },
    {
      value: "graded",
      label: t.ob_mode_graded,
      desc: t.ob_mode_graded_desc,
    },
    {
      value: "full-auto",
      label: t.ob_mode_full_auto,
      desc: t.ob_mode_full_auto_desc,
    },
  ];

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
            <h1>{t.ob_welcome_title}</h1>
            <p className="onboarding-subtitle">
              {t.ob_welcome_subtitle}
            </p>
            <ul className="feature-list">
              <li>
                <span className="feature-icon">💬</span>
                {t.ob_feature_1}
              </li>
              <li>
                <span className="feature-icon">🔌</span>
                {t.ob_feature_2}
              </li>
              <li>
                <span className="feature-icon">🛡️</span>
                {t.ob_feature_3}
              </li>
            </ul>
            <button
              className="btn-primary btn-large"
              onClick={() => setStep(2)}
            >
              {t.ob_btn_start}
            </button>
          </div>
        )}

        {/* Step 2: Add first provider */}
        {step === 2 && (
          <div className="onboarding-step">
            <h2>{t.ob_add_provider_title}</h2>
            <p className="onboarding-subtitle">
              {t.ob_add_provider_subtitle}
            </p>
            <div className="form-wrapper">
              <ProviderForm
                onSave={handleAddProvider}
                onCancel={() => setStep(3)}
              />
            </div>
            <button className="btn-skip" onClick={() => setStep(3)}>
              {t.ob_btn_skip}
            </button>
          </div>
        )}

        {/* Step 3: Choose execution mode */}
        {step === 3 && (
          <div className="onboarding-step">
            <h2>{t.ob_choose_mode_title}</h2>
            <p className="onboarding-subtitle">
              {t.ob_choose_mode_subtitle}
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
                {t.ob_no_provider_notice}
              </p>
            )}
            <button
              className="btn-primary btn-large"
              onClick={handleFinish}
              disabled={finishing}
            >
              {finishing ? t.ob_btn_finishing : t.ob_btn_finish}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

