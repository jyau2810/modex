import { invoke } from "@tauri-apps/api/core";
import type {
  ActionResult,
  AppLogEntry,
  AppSettings,
  DailyWakeSettings,
  Identity,
  ImportIdentityResult,
  SettingsPatch,
} from "../types";

export const modexApi = {
  getAppState: () => invoke<AppSettings>("get_app_state"),
  addIdentity: () => invoke<Identity>("add_identity"),
  addApiKeyIdentity: (apiKey: string, baseUrl?: string | null) =>
    invoke<Identity>("add_api_key_identity", { apiKey, baseUrl }),
  importCurrentIdentity: () => invoke<ImportIdentityResult>("import_current_identity"),
  deleteIdentity: (name: string) => invoke<ActionResult>("delete_identity", { name }),
  switchIdentity: (name: string) => invoke<ActionResult>("switch_identity", { name }),
  loginIdentity: (name: string) => invoke<ActionResult>("login_identity", { name }),
  refreshIdentity: (name: string) => invoke<Identity>("refresh_identity", { name }),
  refreshAll: () => invoke<Identity[]>("refresh_all"),
  updateSettings: (settingsPatch: SettingsPatch) =>
    invoke<AppSettings>("update_settings", { settingsPatch }),
  updateDailyWakeSettings: (dailyWake: DailyWakeSettings) =>
    invoke<AppSettings>("update_daily_wake_settings", { dailyWake }),
  runDailyWakeNow: () => invoke<ActionResult>("run_daily_wake_now"),
  getRecentLogEntries: () => invoke<AppLogEntry[]>("get_recent_log_entries"),
  openIdentityDirectory: (name: string) => invoke<ActionResult>("open_identity_directory", { name }),
};
