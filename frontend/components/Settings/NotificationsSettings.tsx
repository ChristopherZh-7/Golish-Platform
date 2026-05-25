import { sendNotification as doSendNotification } from "@tauri-apps/plugin-notification";
import { AlertCircle, Bell, CheckCircle, Terminal, Volume2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import type { NotificationsSettings as NotificationsSettingsType } from "@/lib/settings";
import { playNotificationSound, playTestSound } from "@/lib/sound";
import { isPermissionGranted, requestPermission } from "@/lib/systemNotifications";

interface NotificationsSettingsProps {
  settings: NotificationsSettingsType;
  onChange: (settings: NotificationsSettingsType) => void;
}

export function NotificationsSettings({ settings, onChange }: NotificationsSettingsProps) {
  const { t } = useTranslation();
  const [permissionStatus, setPermissionStatus] = useState<"unknown" | "granted" | "denied">(
    "unknown"
  );
  const [testStatus, setTestStatus] = useState<{
    agent?: "success" | "error";
    command?: "success" | "error";
    sound?: "success" | "error";
  }>({});

  // Check permission status when component mounts or when enabled
  const checkPermission = async () => {
    const granted = await isPermissionGranted();
    setPermissionStatus(granted ? "granted" : "denied");
    return granted;
  };

  const handleNativeToggle = async (checked: boolean) => {
    if (checked) {
      // Request permission when enabling
      const granted = await requestPermission();
      setPermissionStatus(granted ? "granted" : "denied");
      if (granted) {
        onChange({ ...settings, native_enabled: true });
      }
      // Don't enable if permission denied
    } else {
      onChange({ ...settings, native_enabled: false });
    }
  };

  const handleSoundToggle = (checked: boolean) => {
    onChange({ ...settings, sound_enabled: checked });
  };

  const getSoundForTest = (): string | undefined => {
    // Use configured sound if provided and non-empty
    if (settings.sound && settings.sound.trim() !== "") {
      return settings.sound;
    }
    // Default to "Blow" on macOS, undefined on other platforms
    const isMacOS = navigator.platform.toLowerCase().includes("mac");
    return isMacOS ? "Blow" : undefined;
  };

  const handleSoundNameChange = (value: string) => {
    // Store empty string as null
    const soundValue = value.trim() === "" ? null : value;
    onChange({
      ...settings,
      sound: soundValue,
    });
  };

  const sendTestNotification = async (type: "agent" | "command") => {
    // Check permission first
    const granted = await checkPermission();
    if (!granted) {
      setTestStatus((prev) => ({ ...prev, [type]: "error" }));
      return;
    }

    const sound = getSoundForTest();

    try {
      // Play in-app sound if enabled (native notification sounds are unreliable)
      if (settings.sound_enabled) {
        playNotificationSound(type);
      }

      if (type === "agent") {
        doSendNotification({
          title: t("notificationsPanel.agentCompletedTitle"),
          body: t("notificationsPanel.agentCompletedBody"),
          sound,
        });
      } else {
        doSendNotification({
          title: t("notificationsPanel.commandCompletedTitle"),
          body: "✓ echo 'Hello, World!'",
          sound,
        });
      }
      setTestStatus((prev) => ({ ...prev, [type]: "success" }));

      // Clear status after 3 seconds
      setTimeout(() => {
        setTestStatus((prev) => ({ ...prev, [type]: undefined }));
      }, 3000);
    } catch {
      setTestStatus((prev) => ({ ...prev, [type]: "error" }));
    }
  };

  const handleTestSound = () => {
    try {
      playTestSound();
      setTestStatus((prev) => ({ ...prev, sound: "success" }));

      // Clear status after 3 seconds
      setTimeout(() => {
        setTestStatus((prev) => ({ ...prev, sound: undefined }));
      }, 3000);
    } catch {
      setTestStatus((prev) => ({ ...prev, sound: "error" }));
    }
  };

  return (
    <div className="space-y-6">
      {/* In-App Sound */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <label
            htmlFor="notifications-sound-enabled"
            className="text-sm font-medium text-foreground cursor-pointer"
          >
            {t("notificationsPanel.inAppSounds")}
          </label>
          <p className="text-xs text-muted-foreground">{t("notificationsPanel.inAppSoundsDesc")}</p>
        </div>
        <Switch
          id="notifications-sound-enabled"
          checked={settings.sound_enabled ?? true}
          onCheckedChange={handleSoundToggle}
        />
      </div>

      {/* Sound Test Button */}
      {settings.sound_enabled && (
        <div className="flex items-center justify-between pl-4 border-l-2 border-[var(--border-medium)]">
          <div className="flex items-center gap-2">
            <Volume2 className="w-4 h-4 text-muted-foreground" />
            <span className="text-sm text-foreground">{t("notificationsPanel.testSound")}</span>
            {testStatus.sound === "success" && <CheckCircle className="w-4 h-4 text-green-500" />}
            {testStatus.sound === "error" && <AlertCircle className="w-4 h-4 text-red-500" />}
          </div>
          <Button variant="outline" size="sm" onClick={handleTestSound}>
            {t("notificationsPanel.play")}
          </Button>
        </div>
      )}

      {/* Divider */}
      <div className="border-t border-[var(--border-medium)]" />

      {/* Native System Notifications */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <label
            htmlFor="notifications-native"
            className="text-sm font-medium text-foreground cursor-pointer"
          >
            {t("notificationsPanel.nativeNotifications")}
          </label>
          <p className="text-xs text-muted-foreground">
            {t("notificationsPanel.nativeNotificationsDesc")}
          </p>
        </div>
        <Switch
          id="notifications-native"
          checked={settings.native_enabled}
          onCheckedChange={handleNativeToggle}
        />
      </div>

      {/* Native Notification Sound Name (macOS) */}
      {settings.native_enabled && (
        <div className="space-y-2 pl-4 border-l-2 border-[var(--border-medium)]">
          <label htmlFor="notifications-sound" className="text-sm font-medium text-foreground">
            {t("notificationsPanel.systemSoundName")}
          </label>
          <Input
            id="notifications-sound"
            type="text"
            placeholder={t("notificationsPanel.systemSoundPlaceholder")}
            value={settings.sound ?? ""}
            onChange={(e) => handleSoundNameChange(e.target.value)}
            className="max-w-md"
          />
          <p className="text-xs text-muted-foreground">
            {t("notificationsPanel.systemSoundDescPrefix")} <span className="font-mono">Ping</span>{" "}
            {t("notificationsPanel.or")} <span className="font-mono">Blow</span>;{" "}
            {t("notificationsPanel.systemSoundDescSuffix")}
          </p>
        </div>
      )}

      {/* Permission Status */}
      {settings.native_enabled && (
        <div className="flex items-center gap-2 text-sm pl-4 border-l-2 border-[var(--border-medium)]">
          {permissionStatus === "granted" ? (
            <>
              <CheckCircle className="w-4 h-4 text-green-500" />
              <span className="text-muted-foreground">
                {t("notificationsPanel.permissionGranted")}
              </span>
            </>
          ) : permissionStatus === "denied" ? (
            <>
              <AlertCircle className="w-4 h-4 text-yellow-500" />
              <span className="text-muted-foreground">
                {t("notificationsPanel.permissionDenied")}
              </span>
            </>
          ) : null}
        </div>
      )}

      {/* Test Native Notifications Section */}
      {settings.native_enabled && (
        <div className="space-y-4 p-4 rounded-lg bg-muted border border-[var(--border-medium)]">
          <h4 className="text-sm font-medium text-accent">{t("notificationsPanel.testNative")}</h4>
          <p className="text-xs text-muted-foreground">{t("notificationsPanel.testNativeDesc")}</p>

          <div className="space-y-3">
            {/* Agent Completion Test */}
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Bell className="w-4 h-4 text-muted-foreground" />
                <span className="text-sm text-foreground">
                  {t("notificationsPanel.agentCompletion")}
                </span>
                {testStatus.agent === "success" && (
                  <CheckCircle className="w-4 h-4 text-green-500" />
                )}
                {testStatus.agent === "error" && <AlertCircle className="w-4 h-4 text-red-500" />}
              </div>
              <Button variant="outline" size="sm" onClick={() => sendTestNotification("agent")}>
                {t("notificationsPanel.sendTest")}
              </Button>
            </div>

            {/* Command Completion Test */}
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Terminal className="w-4 h-4 text-muted-foreground" />
                <span className="text-sm text-foreground">
                  {t("notificationsPanel.commandCompletion")}
                </span>
                {testStatus.command === "success" && (
                  <CheckCircle className="w-4 h-4 text-green-500" />
                )}
                {testStatus.command === "error" && <AlertCircle className="w-4 h-4 text-red-500" />}
              </div>
              <Button variant="outline" size="sm" onClick={() => sendTestNotification("command")}>
                {t("notificationsPanel.sendTest")}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
