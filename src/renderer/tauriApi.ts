import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AppSettings,
  AppUpdateInfo,
  ArchiveAnalysis,
  CreateProfileInput,
  DesktopApi,
  GameDetectionResult,
  GameProfile,
  InstalledModRecord,
  InstallRequest,
  InstallResult,
  ModConfigFile,
  ModActionResult,
  ProfileActionResult,
  ProfileDependencyBootstrapResult,
  ProfileRefreshResult,
  UpdateModConfigValueInput
} from "../shared/contracts";

export const desktopApi: DesktopApi = {
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  updateAppSettings: (input: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { input }),
  checkAppUpdate: () => invoke<AppUpdateInfo>("check_app_update"),
  minimizeWindow: () => getCurrentWindow().minimize(),
  toggleMaximizeWindow: () => getCurrentWindow().toggleMaximize(),
  closeWindow: () => getCurrentWindow().close(),
  startWindowDrag: () => getCurrentWindow().startDragging(),
  listProfiles: () => invoke<GameProfile[]>("list_profiles"),
  createProfile: (input: CreateProfileInput) =>
    invoke<GameProfile>("create_profile", { input }),
  renameProfile: (profileId: string, name: string) =>
    invoke<GameProfile>("rename_profile", { profileId, name }),
  removeProfile: (profileId: string) =>
    invoke<ProfileActionResult>("remove_profile", { profileId }),
  refreshProfile: (profileId: string) =>
    invoke<ProfileRefreshResult>("refresh_profile", { profileId }),
  bootstrapProfileDependencies: (profileId: string) =>
    invoke<ProfileDependencyBootstrapResult>("bootstrap_profile_dependencies", { profileId }),
  selectGameFolder: async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Select game install folder"
    });
    return normalizeDialogSelection(selected);
  },
  detectGameSetup: (gamePath: string) =>
    invoke<GameDetectionResult>("detect_game_setup", { gamePath }),
  selectAndAnalyzeArchive: async (profileId: string) => {
    const selected = await open({
      directory: false,
      multiple: false,
      title: "Import mod archive",
      filters: [{ name: "Mod archives", extensions: ["zip", "7z", "rar"] }]
    });
    const archivePath = normalizeDialogSelection(selected);

    if (!archivePath) {
      return null;
    }

    return invoke<ArchiveAnalysis>("analyze_archive_for_profile", {
      profileId,
      archivePath
    });
  },
  selectAndAnalyzeModFolder: async (profileId: string) => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Import mod folder"
    });
    const archivePath = normalizeDialogSelection(selected);

    if (!archivePath) {
      return null;
    }

    return invoke<ArchiveAnalysis>("analyze_archive_for_profile", {
      profileId,
      archivePath
    });
  },
  analyzeArchivePath: (profileId: string, archivePath: string) =>
    invoke<ArchiveAnalysis>("analyze_archive_for_profile", {
      profileId,
      archivePath
    }),
  installArchive: (request: InstallRequest) =>
    invoke<InstallResult>("install_archive", { request }),
  listInstalledMods: (profileId: string) =>
    invoke<InstalledModRecord[]>("list_installed_mods", { profileId }),
  getModConfigDetails: (profileId: string, installedModId: string) =>
    invoke<ModConfigFile[]>("get_mod_config_details", { profileId, installedModId }),
  updateModConfigValue: (input: UpdateModConfigValueInput) =>
    invoke<ModConfigFile>("update_mod_config_value", { input }),
  disableMod: (profileId: string, installedModId: string) =>
    invoke<ModActionResult>("disable_mod", { profileId, installedModId }),
  enableMod: (profileId: string, installedModId: string) =>
    invoke<ModActionResult>("enable_mod", { profileId, installedModId }),
  removeMod: (profileId: string, installedModId: string) =>
    invoke<ModActionResult>("remove_mod", { profileId, installedModId }),
  openProfileGameFolder: (profileId: string) =>
    invoke<void>("open_profile_game_folder", { profileId }),
  getStorePath: () => invoke<string>("get_store_path")
};

function normalizeDialogSelection(selection: string | string[] | null): string | null {
  if (Array.isArray(selection)) {
    return selection[0] ?? null;
  }

  return selection;
}
