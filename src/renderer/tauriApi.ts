import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AppSettings,
  AppUpdateInfo,
  ArchiveAnalysis,
  CreateProfileInput,
  DesktopApi,
  GameDetectionResult,
  GameProfile,
  DiscoveryPage,
  InstalledModRecord,
  InstallRequest,
  InstallPreflightResult,
  InstallResult,
  ModConfigFile,
  ModActionResult,
  NexusNxmInstallResult,
  OnlineInstallSelection,
  OnlineModRecord,
  OnlineModFileOption,
  ProfileActionResult,
  ProfileDependencyBootstrapResult,
  ProfileExportResult,
  ProfileGameFolderUpdateResult,
  ProfileImportResult,
  ProfileLaunchModeResult,
  ProfileModToggleResult,
  ProfileRefreshResult,
  SteamGameRecord,
  UpdateModConfigValueInput
} from "../shared/contracts";

export const desktopApi: DesktopApi = {
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  updateAppSettings: (input: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { input }),
  saveNexusApiKey: (apiKey: string) =>
    invoke<AppSettings>("save_nexus_api_key", { apiKey }),
  checkAppUpdate: () => invoke<AppUpdateInfo>("check_app_update"),
  minimizeWindow: () => getCurrentWindow().minimize(),
  toggleMaximizeWindow: () => getCurrentWindow().toggleMaximize(),
  closeWindow: () => getCurrentWindow().close(),
  downloadUpdateInstaller: (url: string, fileName?: string) =>
    invoke<string>("download_update_installer", { url, fileName }),
  getCachedSteamArtwork: (steamAppId, variant) =>
    invoke<string | null>("get_cached_steam_artwork", { steamAppId, variant }),
  scanSteamGames: () => invoke<SteamGameRecord[]>("scan_steam_games"),
  createSteamProfile: (game: SteamGameRecord) =>
    invoke<GameProfile>("create_steam_profile", { game }),
  launchProfileGame: (profileId: string, modsEnabled: boolean) =>
    invoke<void>("launch_profile_game", { profileId, modsEnabled }),
  setProfileModLaunchMode: (profileId: string, modsEnabled: boolean) =>
    invoke<ProfileLaunchModeResult>("set_profile_mod_launch_mode", {
      profileId,
      modsEnabled
    }),
  profileGameRunning: (profileId: string) =>
    invoke<boolean>("profile_game_running", { profileId }),
  setAllProfileModsEnabled: (profileId: string, enabled: boolean) =>
    invoke<ProfileModToggleResult>("set_all_profile_mods_enabled", { profileId, enabled }),
  listProfiles: () => invoke<GameProfile[]>("list_profiles"),
  profileFolderExists: (profileId: string) =>
    invoke<boolean>("profile_folder_exists", { profileId }),
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
  updateProfileGameFolder: (profileId: string, gamePath: string) =>
    invoke<ProfileGameFolderUpdateResult>("update_profile_game_folder", { profileId, gamePath }),
  exportProfileBundle: async (profileId: string, profileName: string) => {
    const selected = await save({
      title: "Export UniLoader profile",
      defaultPath: `${safeFileName(profileName)}.uniloader-profile`,
      filters: [{ name: "UniLoader profile bundle", extensions: ["uniloader-profile"] }]
    });

    if (!selected) {
      return null;
    }

    return invoke<ProfileExportResult>("export_profile_bundle", {
      profileId,
      outputPath: selected
    });
  },
  importProfileBundle: async () => {
    const bundleSelection = await open({
      directory: false,
      multiple: false,
      title: "Import UniLoader profile"
    });
    const bundlePath = normalizeDialogSelection(bundleSelection);

    if (!bundlePath) {
      return null;
    }

    return invoke<ProfileImportResult>("import_profile_bundle", { bundlePath });
  },
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
  discoverOnlineMods: (profileId, page, pageSize, sort, query, requestId) =>
    invoke<DiscoveryPage>("discover_online_mods", {
      profileId,
      page,
      pageSize,
      sort,
      query,
      requestId
    }),
  listDiscoveredModFiles: (profileId: string, mod: OnlineModRecord) =>
    invoke<OnlineModFileOption[]>("list_discovered_mod_files", {
      profileId,
      provider: mod.provider,
      modId: mod.id,
      providerGameId: mod.providerGameId
    }),
  preflightDiscoveredModInstall: (profileId: string, mod: OnlineModRecord) =>
    invoke<InstallPreflightResult>("preflight_discovered_mod_install", {
      profileId,
      provider: mod.provider,
      modId: mod.id
    }),
  installDiscoveredMod: (
    profileId: string,
    mod: OnlineModRecord,
    file?: OnlineModFileOption,
    selection?: OnlineInstallSelection
  ) =>
    invoke<InstallResult>("install_discovered_mod", {
      profileId,
      provider: mod.provider,
      modId: mod.id,
      version: file?.version ?? mod.version,
      providerGameId: mod.providerGameId,
      selectedFileId: file?.id,
      replaceInstalledModId: selection?.replaceInstalledModId,
      installTargetId: selection?.installTargetId
    }),
  beginNexusBrowserDownload: (
    profileId: string,
    mod: OnlineModRecord,
    file: OnlineModFileOption,
    selection?: OnlineInstallSelection
  ) =>
    invoke<string>("begin_nexus_browser_download", {
      profileId,
      modId: mod.id,
      version: file.version ?? mod.version,
      providerGameId: mod.providerGameId,
      selectedFileId: file.id,
      replaceInstalledModId: selection?.replaceInstalledModId,
      installTargetId: selection?.installTargetId
    }),
  beginNexusRequirementDownload: (profileId: string, dependencyId: string) =>
    invoke<string>("begin_nexus_requirement_download", { profileId, dependencyId }),
  installNexusNxmLink: (nxmUrl: string) =>
    invoke<NexusNxmInstallResult>("install_nexus_nxm_link", { nxmUrl }),
  listInstalledMods: (profileId: string) =>
    invoke<InstalledModRecord[]>("list_installed_mods", { profileId }),
  refreshInstalledModArtwork: (profileId: string) =>
    invoke<InstalledModRecord[]>("refresh_installed_mod_artwork", { profileId }),
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
  openExternalUrl: (url: string) =>
    invoke<void>("open_external_url", { url }),
  getStorePath: () => invoke<string>("get_store_path")
};

function normalizeDialogSelection(selection: string | string[] | null): string | null {
  if (Array.isArray(selection)) {
    return selection[0] ?? null;
  }

  return selection;
}

function safeFileName(name: string): string {
  const cleaned = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, " ").replace(/\s+/g, " ").trim();
  return cleaned || "UniLoader Profile";
}
