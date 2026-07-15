import { useState, useEffect } from "react";
import { getConfig, type EnterprisePolicy } from "../../ipc/config";
import { enterpriseRegisterDevice, enterpriseInstallService } from "../../ipc/enterprise";
import { useLocale } from "../../contexts/LocaleContext";

type RegisterStatus = "idle" | "registering" | "success" | "error";

export function EnterprisePage() {
  const { t } = useLocale();
  const [serverUrl, setServerUrl] = useState("");
  const [deviceName, setDeviceName] = useState("");
  const [deviceType, setDeviceType] = useState<"interactive" | "headless_worker">("interactive");
  const [role, setRole] = useState<"dev" | "dba" | "ops" | "qa">("dev");
  const [status, setStatus] = useState<RegisterStatus>("idle");
  const [errorMsg, setErrorMsg] = useState("");

  // Device info from existing config
  const [existingServerUrl, setExistingServerUrl] = useState<string | null>(null);
  const [existingDeviceId, setExistingDeviceId] = useState<string | null>(null);
  const [enterprisePolicy, setEnterprisePolicy] = useState<EnterprisePolicy | null>(null);

  useEffect(() => {
    getConfig().then((cfg) => {
      if (cfg.enterprise_server_url) {
        setExistingServerUrl(cfg.enterprise_server_url);
        setServerUrl(cfg.enterprise_server_url);
      }
      if (cfg.enterprise_device_id) {
        setExistingDeviceId(cfg.enterprise_device_id);
      }
      if (cfg.enterprise_policy) {
        setEnterprisePolicy(cfg.enterprise_policy);
      }
    });
  }, []);

  const handleRegister = async () => {
    if (!serverUrl.trim() || !deviceName.trim()) return;
    setStatus("registering");
    setErrorMsg("");
    try {
      const deviceId = await enterpriseRegisterDevice({
        serverUrl: serverUrl.trim(),
        deviceName: deviceName.trim(),
        deviceType,
        role,
      });
      setExistingDeviceId(deviceId);
      setExistingServerUrl(serverUrl.trim());
      setStatus("success");
    } catch (e) {
      setErrorMsg(String(e));
      setStatus("error");
    }
  };

  const isAlreadyRegistered = !!existingDeviceId;

  return (
    <div style={{ padding: "24px 32px", color: "#e0e0e0", maxWidth: 560 }}>
      <h2 style={{ margin: "0 0 8px", fontSize: 18 }}>{t.enterprise_title}</h2>
      <p style={{ color: "#888", marginBottom: 24, fontSize: 13 }}>
        {t.enterprise_desc}
      </p>

      {isAlreadyRegistered && (
        <div style={{
          background: "#1a2a1a", border: "1px solid #3a6a3a", borderRadius: 6,
          padding: "12px 16px", marginBottom: 24, fontSize: 13,
        }}>
          <div style={{ color: "#6abf6a", fontWeight: 600, marginBottom: 6 }}>{t.enterprise_registered}</div>
          <div><span style={{ color: "#888" }}>{t.enterprise_server_label}</span>{existingServerUrl}</div>
          <div><span style={{ color: "#888" }}>{t.enterprise_device_id_label}</span>
            <code style={{ fontSize: 12, color: "#aaa" }}>{existingDeviceId}</code>
          </div>
          {enterprisePolicy && (
            <div style={{ marginTop: 8 }}>
              <span style={{ color: "#888" }}>{t.enterprise_policy_version_label}</span>
              {String(enterprisePolicy.version ?? 0)}
              {enterprisePolicy.execution_mode && (
                <span style={{ marginLeft: 12, color: "#888" }}>
                  {t.enterprise_execution_label}<span style={{ color: "#e0c060" }}>{String(enterprisePolicy.execution_mode)}</span>
                  <span style={{ color: "#666", fontSize: 11, marginLeft: 4 }}>{t.settings_managed_by_admin || t.enterprise_managed}</span>
                </span>
              )}
            </div>
          )}
        </div>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <label style={{ fontSize: 13 }}>
          <div style={{ marginBottom: 4, color: "#aaa" }}>{t.enterprise_server_url_label}</div>
          <input
            type="text"
            value={serverUrl}
            onChange={(e) => setServerUrl(e.target.value)}
            placeholder="https://enterprise.example.com"
            style={{
              width: "100%", background: "#111", border: "1px solid #333",
              borderRadius: 4, padding: "6px 10px", color: "#e0e0e0",
              fontSize: 13, boxSizing: "border-box",
            }}
          />
        </label>

        <label style={{ fontSize: 13 }}>
          <div style={{ marginBottom: 4, color: "#aaa" }}>{t.enterprise_device_name_label}</div>
          <input
            type="text"
            value={deviceName}
            onChange={(e) => setDeviceName(e.target.value)}
            placeholder="e.g. dev-macbook-james"
            style={{
              width: "100%", background: "#111", border: "1px solid #333",
              borderRadius: 4, padding: "6px 10px", color: "#e0e0e0",
              fontSize: 13, boxSizing: "border-box",
            }}
          />
        </label>

        <div style={{ display: "flex", gap: 12 }}>
          <label style={{ flex: 1, fontSize: 13 }}>
            <div style={{ marginBottom: 4, color: "#aaa" }}>{t.enterprise_device_type_label}</div>
            <select
              value={deviceType}
              onChange={(e) => setDeviceType(e.target.value as typeof deviceType)}
              style={{
                width: "100%", background: "#111", border: "1px solid #333",
                borderRadius: 4, padding: "6px 10px", color: "#e0e0e0", fontSize: 13,
              }}
            >
              <option value="interactive">{t.enterprise_device_interactive}</option>
              <option value="headless_worker">{t.enterprise_device_headless}</option>
            </select>
          </label>

          <label style={{ flex: 1, fontSize: 13 }}>
            <div style={{ marginBottom: 4, color: "#aaa" }}>{t.enterprise_role_label}</div>
            <select
              value={role}
              onChange={(e) => setRole(e.target.value as typeof role)}
              style={{
                width: "100%", background: "#111", border: "1px solid #333",
                borderRadius: 4, padding: "6px 10px", color: "#e0e0e0", fontSize: 13,
              }}
            >
              <option value="dev">{t.enterprise_role_dev}</option>
              <option value="dba">{t.enterprise_role_dba}</option>
              <option value="ops">{t.enterprise_role_ops}</option>
              <option value="qa">{t.enterprise_role_qa}</option>
            </select>
          </label>
        </div>

        <button
          onClick={handleRegister}
          disabled={status === "registering" || !serverUrl.trim() || !deviceName.trim()}
          style={{
            marginTop: 4, padding: "8px 20px", background: "#2a5a9a", color: "#fff",
            border: "none", borderRadius: 4, cursor: "pointer", fontWeight: 600,
            fontSize: 13, opacity: (status === "registering" || !serverUrl.trim() || !deviceName.trim()) ? 0.5 : 1,
          }}
        >
          {status === "registering" ? t.enterprise_registering : isAlreadyRegistered ? t.enterprise_reregister : t.enterprise_register}
        </button>

        {status === "success" && (
          <div style={{ color: "#6abf6a", fontSize: 13 }}>
            {t.enterprise_register_success(existingDeviceId || "")}
          </div>
        )}
        {status === "error" && (
          <div style={{ color: "#e06060", fontSize: 13 }}>{errorMsg}</div>
        )}
      </div>

      {/* System Service Installer */}
      {isAlreadyRegistered && deviceType === "headless_worker" && (
        <ServiceInstallerSection />
      )}
    </div>
  );
}

function ServiceInstallerSection() {
  const { t } = useLocale();
  const [serviceContent, setServiceContent] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState("");
  const [installed, setInstalled] = useState(false);

  const preview = async () => {
    try {
      const content = await enterpriseInstallService(false);
      setServiceContent(content);
    } catch (e) {
      setInstallError(String(e));
    }
  };

  const install = async () => {
    setInstalling(true);
    setInstallError("");
    try {
      await enterpriseInstallService(true);
      setInstalled(true);
    } catch (e) {
      setInstallError(String(e));
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div style={{ marginTop: 32, borderTop: "1px solid #333", paddingTop: 24 }}>
      <div style={{ fontWeight: 600, marginBottom: 8, fontSize: 14 }}>{t.enterprise_service_title}</div>
      <p style={{ color: "#888", fontSize: 13, marginBottom: 12 }}>
        {t.enterprise_service_desc}
      </p>
      <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
        <button
          onClick={preview}
          style={{ padding: "6px 14px", background: "#222", border: "1px solid #444", color: "#e0e0e0", borderRadius: 4, cursor: "pointer", fontSize: 13 }}
        >
          {t.enterprise_preview_config}
        </button>
        <button
          onClick={install}
          disabled={installing}
          style={{ padding: "6px 14px", background: "#2a5a9a", border: "none", color: "#fff", borderRadius: 4, cursor: "pointer", fontSize: 13, opacity: installing ? 0.5 : 1 }}
        >
          {installing ? t.enterprise_installing : t.enterprise_install_service}
        </button>
      </div>
      {installed && <div style={{ color: "#6abf6a", fontSize: 13, marginBottom: 8 }}>{t.enterprise_install_success}</div>}
      {installError && <div style={{ color: "#e06060", fontSize: 13, marginBottom: 8 }}>{installError}</div>}
      {serviceContent && (
        <pre style={{ background: "#111", border: "1px solid #333", borderRadius: 4, padding: "10px 14px", fontSize: 11, color: "#ccc", whiteSpace: "pre-wrap", maxHeight: 240, overflow: "auto" }}>
          {serviceContent}
        </pre>
      )}
    </div>
  );
}

