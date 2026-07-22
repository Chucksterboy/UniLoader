export const gameEngines = [
  "unity-mono",
  "unity-il2cpp",
  "unreal",
  "re-engine",
  "unknown"
] as const;

export type GameEngine = (typeof gameEngines)[number];

export const loaderKinds = [
  "none",
  "bepinex",
  "bepinex-il2cpp",
  "ue4ss",
  "reframework",
  "loose-files"
] as const;

export type LoaderKind = (typeof loaderKinds)[number];

export type AdapterId =
  | "bepinex"
  | "ue4ss"
  | "reframework"
  | "re-engine-native"
  | "unreal-pak"
  | "loose-files"
  | "script-files";

export type DependencyProvider =
  | "thunderstore"
  | "github-release"
  | "bepinbuilds"
  | "nexus"
  | "curseforge"
  | "overwolf"
  | "modio"
  | "known-runtime"
  | "manual";

export type DependencyStatus =
  | "already-installed"
  | "missing"
  | "planned"
  | "manual";

export interface GameProfile {
  id: string;
  name: string;
  gamePath: string;
  gameId?: string;
  steamAppId?: string;
  engine: GameEngine;
  loader: LoaderKind;
  setupStatus: "setting-up" | "ready" | "needs-action" | "failed";
  setupWarnings: string[];
  setupUpdatedAt?: string;
  modsEnabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface CreateProfileInput {
  name: string;
  gamePath: string;
  gameId?: string;
  steamAppId?: string;
  engine: GameEngine;
  loader: LoaderKind;
}

export interface SteamGameRecord {
  appId: string;
  name: string;
  installDir: string;
  libraryPath: string;
}

export interface DetectionSignal {
  label: string;
  path: string;
  weight: number;
}

export interface GameDetectionResult {
  gamePath: string;
  gameId?: string;
  engine: GameEngine;
  loader: LoaderKind;
  recommendedLoader: LoaderKind;
  engineConfidence: number;
  loaderConfidence: number;
  loaderInstalled: boolean;
  expectedModFolders: string[];
  createdModFolders: string[];
  signals: DetectionSignal[];
  warnings: string[];
}

export interface ArchiveEntry {
  path: string;
  logicalPath: string;
  size: number;
  isDirectory: boolean;
}

export interface ThunderstoreManifest {
  name: string;
  version_number: string;
  website_url?: string;
  description?: string;
  dependencies?: string[];
}

export interface DependencySpec {
  id: string;
  name: string;
  version?: string;
  provider: DependencyProvider;
  required: boolean;
  status: DependencyStatus;
  source?: string;
  notes?: string;
}

export interface InstallMapping {
  sourcePath: string;
  targetRoot: "game" | "profile";
  targetRelativePath: string;
  reason: string;
}

export interface InstallPlan {
  adapterId: AdapterId;
  adapterName: string;
  confidence: number;
  summary: string;
  mappings: InstallMapping[];
  dependencies: DependencySpec[];
  warnings: string[];
  requiresConfirmation: boolean;
}

export type PackageProvider = "thunderstore" | "nexus" | "curseforge" | "unknown";

export interface PackageIdentity {
  provider: PackageProvider;
  packageId?: string;
  version?: string;
  providerGameId?: string;
  modTypes: AdapterId[];
  dependencies: string[];
  evidence: string[];
  confidence: number;
}

export interface CompatibilityResult {
  status: "compatible" | "blocked";
  reason: string;
  confidence: number;
  gameId?: string;
  providerGameId?: string;
  detectedModTypes: AdapterId[];
  supportedModTypes: AdapterId[];
}

export interface ArchiveAnalysis {
  archivePath: string;
  archiveName: string;
  entries: ArchiveEntry[];
  manifest?: ThunderstoreManifest;
  packageIdentity: PackageIdentity;
  compatibility: CompatibilityResult;
  plans: InstallPlan[];
  recommendedPlan?: InstallPlan;
}

export interface InstallRequest {
  profileId: string;
  archivePath: string;
  archiveName?: string;
  packageIdentity?: PackageIdentity;
  plan: InstallPlan;
}

export interface InstallResult {
  profileId: string;
  archivePath: string;
  installedModId: string;
  installedAt: string;
  filesWritten: string[];
  backupsWritten: string[];
  warnings: string[];
}

export interface InstallTargetOption {
  id: string;
  label: string;
  relativePath: string;
  scope: string;
  recommended: boolean;
}

export interface InstallPreflightResult {
  dependencies: DependencySpec[];
  missingDependencies: DependencySpec[];
  confirmationRequired: boolean;
  installTargets: InstallTargetOption[];
}

export interface NexusNxmInstallResult {
  modId: string;
  installResult: InstallResult;
}

export interface InstalledModRecord {
  id: string;
  profileId: string;
  archivePath: string;
  archiveName: string;
  displayName?: string;
  packageId?: string;
  dependencyString?: string;
  packageProvider?: string;
  packageVersion?: string;
  providerFileId?: string;
  providerVariant?: string;
  sourceArchiveSha256?: string;
  iconUrl?: string;
  adapterId: AdapterId;
  summary: string;
  installedAt: string;
  filesWritten: string[];
  backupsWritten: string[];
  dependencies: DependencySpec[];
  configFiles: string[];
  runtimeId?: string;
  externallyManaged: boolean;
  enabled: boolean;
  lastStatus: "installed" | "disabled" | "removed" | "failed";
  plan?: InstallPlan;
}

export interface ModConfigEntry {
  section?: string;
  key: string;
  value: string;
  valueType?: string;
  defaultValue?: string;
  description?: string;
}

export interface ModConfigFile {
  path: string;
  fileName: string;
  entries: ModConfigEntry[];
  rawPreview: string;
  warning?: string;
}

export interface UpdateModConfigValueInput {
  profileId: string;
  filePath: string;
  section?: string;
  key: string;
  value: string;
}

export interface AppSettings {
  minimizeToTrayOnClose: boolean;
  nexusApiKey?: string;
  nexusApiKeyConfigured: boolean;
}

export type AppUpdateStatus = "up-to-date" | "available" | "unavailable" | "error";

export interface AppUpdateInfo {
  currentVersion: string;
  latestVersion?: string;
  updateAvailable: boolean;
  releaseUrl?: string;
  installerUrl?: string;
  installerName?: string;
  status: AppUpdateStatus;
  message: string;
}

export type OnlineModProvider = "thunderstore" | "nexus";

export type OnlineModFileAction = "direct" | "browser" | "auth" | "unsupported";

export interface OnlineModFileOption {
  id: string;
  name: string;
  version?: string;
  category?: string;
  description?: string;
  fileName?: string;
  fileSize?: number;
  uploadedAt?: string;
  primary: boolean;
  action: OnlineModFileAction;
  downloadPageUrl?: string;
}

export interface OnlineInstallSelection {
  replaceInstalledModId?: string;
  installTargetId?: string;
}

export interface OnlineModRecord {
  id: string;
  provider: OnlineModProvider;
  providerLabel: string;
  gameId?: string;
  providerGameId?: string;
  name: string;
  owner: string;
  version: string;
  description: string;
  categories: string[];
  downloads: number;
  ratingScore: number;
  dependencyCount: number;
  fileSize?: number;
  iconUrl?: string;
  packageUrl?: string;
  websiteUrl?: string;
  installed: boolean;
  createdAt?: string;
  updatedAt?: string;
  installSupported: boolean;
  installNote?: string;
}

export interface DiscoveryPage {
  items: OnlineModRecord[];
  total: number;
  page: number;
  pageSize: number;
  hasMore: boolean;
  providerWarnings: string[];
}

export interface ModActionResult {
  profileId: string;
  installedModId: string;
  status: InstalledModRecord["lastStatus"];
  filesChanged: string[];
  warnings: string[];
}

export interface ProfileModToggleResult {
  profileId: string;
  enabled: boolean;
  changedMods: number;
  filesChanged: string[];
  warnings: string[];
  installedMods: InstalledModRecord[];
}

export interface ProfileLaunchModeResult {
  profileId: string;
  modsEnabled: boolean;
  changedComponents: number;
}

export interface ProfileActionResult {
  profileId: string;
  name: string;
  removedModRecords: number;
  warnings: string[];
}

export interface ProfileDependencyBootstrapResult {
  profileId: string;
  installedDependencies: string[];
  skippedDependencies: string[];
  warnings: string[];
}

export interface ModFileHealth {
  installedModId: string;
  modName: string;
  checkedFiles: number;
  missingFiles: string[];
  suspendedFiles: string[];
}

export interface RuntimeUpdateResult {
  runtimeId: string;
  name: string;
  previousVersion?: string;
  installedVersion: string;
}

export interface ProfileRefreshResult {
  profile: GameProfile;
  detection: GameDetectionResult;
  installedMods: InstalledModRecord[];
  modFileHealth: ModFileHealth[];
  missingDependencies: DependencySpec[];
  adoptedNativeScriptMods: number;
  runtimeUpdates?: RuntimeUpdateResult[];
  runtimeUpdateNotes?: string[];
  warnings: string[];
}

export interface ProfileGameFolderUpdateResult {
  profile: GameProfile;
  detection: GameDetectionResult;
  installedMods: InstalledModRecord[];
  deployedFiles: string[];
  warnings: string[];
}

export interface ProfileExportResult {
  outputPath: string;
  profileName: string;
  exportedMods: number;
  exportedConfigFiles: number;
  warnings: string[];
}

export interface ProfileImportResult {
  profile: GameProfile;
  installedMods: InstalledModRecord[];
  deployedFiles: string[];
  configFilesWritten: string[];
  warnings: string[];
}

export interface AppState {
  profiles: GameProfile[];
  installedMods: InstalledModRecord[];
}

export interface DesktopApi {
  getAppSettings(): Promise<AppSettings>;
  updateAppSettings(input: AppSettings): Promise<AppSettings>;
  saveNexusApiKey(apiKey: string): Promise<AppSettings>;
  checkAppUpdate(): Promise<AppUpdateInfo>;
  minimizeWindow(): Promise<void>;
  toggleMaximizeWindow(): Promise<void>;
  closeWindow(): Promise<void>;
  downloadUpdateInstaller(url: string, fileName?: string): Promise<string>;
  getCachedSteamArtwork(
    steamAppId: string,
    variant: "hero" | "poster"
  ): Promise<string | null>;
  scanSteamGames(): Promise<SteamGameRecord[]>;
  createSteamProfile(game: SteamGameRecord): Promise<GameProfile>;
  launchProfileGame(profileId: string, modsEnabled: boolean): Promise<void>;
  setProfileModLaunchMode(
    profileId: string,
    modsEnabled: boolean
  ): Promise<ProfileLaunchModeResult>;
  profileGameRunning(profileId: string): Promise<boolean>;
  setAllProfileModsEnabled(profileId: string, enabled: boolean): Promise<ProfileModToggleResult>;
  listProfiles(): Promise<GameProfile[]>;
  profileFolderExists(profileId: string): Promise<boolean>;
  createProfile(input: CreateProfileInput): Promise<GameProfile>;
  renameProfile(profileId: string, name: string): Promise<GameProfile>;
  removeProfile(profileId: string): Promise<ProfileActionResult>;
  refreshProfile(profileId: string): Promise<ProfileRefreshResult>;
  bootstrapProfileDependencies(profileId: string): Promise<ProfileDependencyBootstrapResult>;
  updateProfileGameFolder(profileId: string, gamePath: string): Promise<ProfileGameFolderUpdateResult>;
  exportProfileBundle(profileId: string, profileName: string): Promise<ProfileExportResult | null>;
  importProfileBundle(): Promise<ProfileImportResult | null>;
  selectGameFolder(): Promise<string | null>;
  detectGameSetup(gamePath: string): Promise<GameDetectionResult>;
  selectAndAnalyzeArchive(profileId: string): Promise<ArchiveAnalysis | null>;
  selectAndAnalyzeModFolder(profileId: string): Promise<ArchiveAnalysis | null>;
  analyzeArchivePath(profileId: string, archivePath: string): Promise<ArchiveAnalysis>;
  installArchive(request: InstallRequest): Promise<InstallResult>;
  discoverOnlineMods(
    profileId: string,
    page: number,
    pageSize: number,
    sort: "downloads" | "newest" | "oldest",
    query: string
  ): Promise<DiscoveryPage>;
  listDiscoveredModFiles(
    profileId: string,
    mod: OnlineModRecord
  ): Promise<OnlineModFileOption[]>;
  preflightDiscoveredModInstall(
    profileId: string,
    mod: OnlineModRecord
  ): Promise<InstallPreflightResult>;
  installDiscoveredMod(
    profileId: string,
    mod: OnlineModRecord,
    file?: OnlineModFileOption,
    selection?: OnlineInstallSelection
  ): Promise<InstallResult>;
  beginNexusBrowserDownload(
    profileId: string,
    mod: OnlineModRecord,
    file: OnlineModFileOption,
    selection?: OnlineInstallSelection
  ): Promise<string>;
  beginNexusRequirementDownload(profileId: string, dependencyId: string): Promise<string>;
  installNexusNxmLink(nxmUrl: string): Promise<NexusNxmInstallResult>;
  listInstalledMods(profileId: string): Promise<InstalledModRecord[]>;
  refreshInstalledModArtwork(profileId: string): Promise<InstalledModRecord[]>;
  getModConfigDetails(profileId: string, installedModId: string): Promise<ModConfigFile[]>;
  updateModConfigValue(input: UpdateModConfigValueInput): Promise<ModConfigFile>;
  disableMod(profileId: string, installedModId: string): Promise<ModActionResult>;
  enableMod(profileId: string, installedModId: string): Promise<ModActionResult>;
  removeMod(profileId: string, installedModId: string): Promise<ModActionResult>;
  openProfileGameFolder(profileId: string): Promise<void>;
  openExternalUrl(url: string): Promise<void>;
  getStorePath(): Promise<string>;
}
