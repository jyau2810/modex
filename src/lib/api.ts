import { invoke } from "@tauri-apps/api/core";
import type { ActionResult, AppSettings, Identity, ImportIdentityResult, SettingsPatch } from "../types";

export const modexApi = {
  getAppState: () => invoke<AppSettings>("get_app_state"),
  addIdentity: () => invoke<Identity>("add_identity"),
  importCurrentIdentity: () => invoke<ImportIdentityResult>("import_current_identity"),
  deleteIdentity: (name: string) => invoke<ActionResult>("delete_identity", { name }),
  switchIdentity: (name: string) => invoke<ActionResult>("switch_identity", { name }),
  loginIdentity: (name: string) => invoke<ActionResult>("login_identity", { name }),
  refreshIdentity: (name: string) => invoke<Identity>("refresh_identity", { name }),
  refreshAll: () => invoke<Identity[]>("refresh_all"),
  updateSettings: (settingsPatch: SettingsPatch) =>
    invoke<AppSettings>("update_settings", { settingsPatch }),
  openIdentityDirectory: (name: string) => invoke<ActionResult>("open_identity_directory", { name }),
};
