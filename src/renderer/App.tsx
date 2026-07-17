import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Compass,
  Database,
  Download,
  FolderOpen,
  Home,
  Minus,
  PackagePlus,
  Power,
  PowerOff,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Trash2,
  Upload,
  X
} from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  FormEvent,
  memo,
  MouseEvent as ReactMouseEvent,
  useEffect,
  useMemo,
  useRef,
  useState
} from "react";
import {
  AppSettings,
  AppUpdateInfo,
  ArchiveAnalysis,
  CreateProfileInput,
  GameDetectionResult,
  GameProfile,
  InstalledModRecord,
  InstallPlan,
  ModConfigEntry,
  ModConfigFile,
  OnlineModRecord,
  ProfileDependencyBootstrapResult,
  ProfileRefreshResult
} from "../shared/contracts";
import { desktopApi } from "./tauriApi";

const emptyProfileInput: CreateProfileInput = {
  name: "",
  gamePath: "",
  gameId: undefined,
  engine: "unknown",
  loader: "none"
};

const defaultAppSettings: AppSettings = {
  minimizeToTrayOnClose: false,
  nexusApiKey: ""
};

const appDisplayVersion = "v0.5";

type ViewMode = "manager" | "discover" | "transfer" | "settings";
type ModSortMode = "newest" | "oldest";
type OnlineSortMode = "downloads" | "newest" | "oldest";
type TransferMode = "import" | "export" | null;
type NoticeKind = "success" | "warning" | "error";
type StartupSplashPhase = "intro" | "exiting" | "hidden";

interface Notice {
  motionId: number;
  kind: NoticeKind;
  title: string;
  detail: string;
}

type NoticeInput = Omit<Notice, "motionId">;

interface ConfigModalState {
  mod: InstalledModRecord;
  files: ModConfigFile[];
  isLoading: boolean;
  error?: string;
}

type MotionPhase = "entering" | "exiting";

interface MotionState<T> {
  className: string;
  phase: MotionPhase;
  value: T;
}

interface MotionPresence<T> {
  className: string;
  phase: MotionPhase;
  value: T | null;
}

const motionDurationMs = 180;
const discoverPageSize = 20;
const nexusApiKeysUrl = "https://www.nexusmods.com/settings/api-keys";
const startupSplashPulseMs = 2700;
const startupSplashFadeMs = 420;

export function App() {
  const [activeView, setActiveView] = useState<ViewMode>("manager");
  const [startupSplashPhase, setStartupSplashPhase] = useState<StartupSplashPhase>("intro");
  const [appSettings, setAppSettings] = useState<AppSettings>(defaultAppSettings);
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [profiles, setProfiles] = useState<GameProfile[]>([]);
  const [selectedProfileId, setSelectedProfileId] = useState<string>("");
  const [profileInput, setProfileInput] = useState<CreateProfileInput>(emptyProfileInput);
  const [analysis, setAnalysis] = useState<ArchiveAnalysis | null>(null);
  const [installedMods, setInstalledMods] = useState<InstalledModRecord[]>([]);
  const [discoverProfileId, setDiscoverProfileId] = useState<string>("");
  const [discoverLoadedProfileId, setDiscoverLoadedProfileId] = useState<string>("");
  const [onlineMods, setOnlineMods] = useState<OnlineModRecord[]>([]);
  const [modSortMode, setModSortMode] = useState<ModSortMode>("newest");
  const [transferMode, setTransferMode] = useState<TransferMode>(null);
  const [detection, setDetection] = useState<GameDetectionResult | null>(null);
  const [isDetecting, setIsDetecting] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isDiscoveringMods, setIsDiscoveringMods] = useState(false);
  const [installingOnlineModId, setInstallingOnlineModId] = useState<string>("");
  const [isCheckingForUpdate, setIsCheckingForUpdate] = useState(false);
  const [isDownloadingUpdate, setIsDownloadingUpdate] = useState(false);
  const [isTransferringProfile, setIsTransferringProfile] = useState(false);
  const [status, setStatus] = useState<string>("Ready");
  const [notice, setNoticeState] = useState<Notice | null>(null);
  const [nexusSettingsAttentionId, setNexusSettingsAttentionId] = useState(0);
  const [configModal, setConfigModal] = useState<ConfigModalState | null>(null);
  const [profilePendingRename, setProfilePendingRename] = useState<GameProfile | null>(null);
  const [profilePendingRemoval, setProfilePendingRemoval] = useState<GameProfile | null>(null);
  const [error, setErrorState] = useState<string>("");
  const [errorMotionId, setErrorMotionId] = useState(0);
  const noticeSequence = useRef(0);
  const errorSequence = useRef(0);
  const updateAnnouncementShown = useRef(false);

  function setNotice(nextNotice: NoticeInput | null) {
    setNoticeState(nextNotice ? { ...nextNotice, motionId: ++noticeSequence.current } : null);
  }

  function setError(nextError: string) {
    if (nextError) {
      setErrorMotionId(++errorSequence.current);
    }
    setErrorState(nextError);
  }

  const api = desktopApi;
  const viewMotion = useFadeSwitch(activeView);
  const renderedView = viewMotion.value;
  const configModalPresence = useFadePresence(configModal);
  const profileRenamePresence = useFadePresence(profilePendingRename);
  const profileRemovalPresence = useFadePresence(profilePendingRemoval);
  const noticePresence = useFadePresence(notice, 140);
  const errorPresence = useFadePresence(error ? error : null, 140);
  const selectedProfile = useMemo(
    () => profiles.find((profile) => profile.id === selectedProfileId),
    [profiles, selectedProfileId]
  );
  const sortedInstalledMods = useMemo(
    () =>
      [...installedMods].sort((first, second) => {
        const firstTime = Date.parse(first.installedAt) || 0;
        const secondTime = Date.parse(second.installedAt) || 0;
        return modSortMode === "newest" ? secondTime - firstTime : firstTime - secondTime;
      }),
    [installedMods, modSortMode]
  );
  const displayedOnlineMods =
    discoverLoadedProfileId === discoverProfileId ? onlineMods : [];
  const selectedPlan = analysis?.recommendedPlan;
  const detectionIssue = getDetectionIssue(detection);
  const healthTone: NoticeKind =
    error || notice?.kind === "error"
      ? "error"
      : notice?.kind === "warning" || status.toLowerCase().includes("needs")
        ? "warning"
        : "success";
  const healthMessage = isRefreshing
    ? "Checking systems"
    : healthTone === "success"
      ? "All systems functional"
      : "Needs attention";

  useEffect(() => {
    void bootstrap();
  }, []);

  useEffect(() => {
    void checkForUpdates(false);
  }, []);

  useEffect(() => {
    const exitTimeoutId = window.setTimeout(
      () => setStartupSplashPhase("exiting"),
      startupSplashPulseMs
    );
    const hideTimeoutId = window.setTimeout(
      () => setStartupSplashPhase("hidden"),
      startupSplashPulseMs + startupSplashFadeMs
    );

    return () => {
      window.clearTimeout(exitTimeoutId);
      window.clearTimeout(hideTimeoutId);
    };
  }, []);

  useEffect(() => {
    if (selectedProfileId) {
      void refreshInstalledMods(selectedProfileId);
    }
  }, [selectedProfileId]);

  useEffect(() => {
    if (activeView !== "discover") {
      return;
    }

    setDiscoverProfileId((current) =>
      current && profiles.some((profile) => profile.id === current)
        ? current
        : selectedProfileId || profiles[0]?.id || ""
    );
  }, [activeView, profiles, selectedProfileId]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload as {
          type: "over" | "drop" | "leave";
          paths?: string[];
        };

        if (payload.type === "over") {
          setIsDragOver(true);
          return;
        }

        if (payload.type === "leave") {
          setIsDragOver(false);
          return;
        }

        if (payload.type === "drop") {
          setIsDragOver(false);
          const importPath = payload.paths?.[0];
          if (importPath) {
            void handleArchivePath(importPath);
          } else {
            setNotice({
              kind: "warning",
              title: "Unsupported file",
              detail: "Drop a .zip, .7z, .rar, or mod folder."
            });
          }
        }
      })
      .then((handler) => {
        unlisten = handler;
      })
      .catch(() => {
        setNotice({
          kind: "warning",
          title: "Drag and drop unavailable",
          detail: "Use the Choose ZIP button to import mods."
        });
      });

    return () => {
      unlisten?.();
    };
  }, [selectedProfileId]);

  async function bootstrap() {
    try {
      const [loadedProfiles, loadedSettings] = await Promise.all([
        api.listProfiles(),
        api.getAppSettings()
      ]);
      setProfiles(loadedProfiles);
      setSelectedProfileId((current) => current || loadedProfiles[0]?.id || "");
      setAppSettings(loadedSettings);
    } catch (caughtError) {
      setError(String(caughtError));
    }
  }

  async function refreshInstalledMods(profileId: string) {
    setInstalledMods(await api.listInstalledMods(profileId));
  }

  async function updateAppSetting(nextSettings: AppSettings) {
    setAppSettings(nextSettings);
    try {
      const savedSettings = await api.updateAppSettings(nextSettings);
      setAppSettings(savedSettings);
      setStatus("Settings saved");
    } catch (caughtError) {
      setError(String(caughtError));
      setNotice({
        kind: "error",
        title: "Settings failed",
        detail: String(caughtError)
      });
    }
  }

  async function checkForUpdates(announce: boolean) {
    setIsCheckingForUpdate(true);
    try {
      const nextUpdateInfo = await api.checkAppUpdate();
      setUpdateInfo(nextUpdateInfo);

      const shouldAnnounceUpdate =
        nextUpdateInfo.updateAvailable && !updateAnnouncementShown.current;
      if (nextUpdateInfo.updateAvailable) {
        updateAnnouncementShown.current = true;
      } else {
        updateAnnouncementShown.current = false;
      }

      if (announce || shouldAnnounceUpdate) {
        const kind: NoticeKind = nextUpdateInfo.updateAvailable
          ? "warning"
          : nextUpdateInfo.status === "error"
            ? "error"
            : "success";
        setNotice({
          kind,
          title: nextUpdateInfo.updateAvailable ? "Update available" : "Update check complete",
          detail: nextUpdateInfo.message
        });
      }
    } catch (caughtError) {
      const message = String(caughtError);
      setUpdateInfo({
        currentVersion: "0.5.0",
        updateAvailable: false,
        status: "error",
        message
      });
      if (announce) {
        setNotice({
          kind: "error",
          title: "Update check failed",
          detail: message
        });
      }
    } finally {
      setIsCheckingForUpdate(false);
    }
  }

  async function showUpdateDetails() {
    if (updateInfo?.updateAvailable) {
      if (!updateInfo.installerUrl) {
        setNotice({
          kind: "error",
          title: "Installer unavailable",
          detail: updateInfo.releaseUrl
            ? `${updateInfo.message} No installer asset was found on the latest release.`
            : updateInfo.message
        });
        return;
      }

      setIsDownloadingUpdate(true);
      setStatus("Downloading update");
      setNotice({
        kind: "warning",
        title: "Downloading update",
        detail: `UniLoader ${updateInfo.latestVersion ? `v${updateInfo.latestVersion}` : "update"} installer is downloading now.`
      });
      try {
        const installerPath = await api.downloadUpdateInstaller(
          updateInfo.installerUrl,
          updateInfo.installerName
        );
        setStatus("Installer launched");
        setNotice({
          kind: "success",
          title: "Installer launched",
          detail: `Downloaded to ${installerPath}. UniLoader will close so the installer can update it.`
        });
      } catch (caughtError) {
        setStatus("Update download failed");
        setError(String(caughtError));
        setNotice({
          kind: "error",
          title: "Update download failed",
          detail: String(caughtError)
        });
      } finally {
        setIsDownloadingUpdate(false);
      }
      return;
    }

    await checkForUpdates(true);
  }

  async function refreshSelectedProfile() {
    if (!selectedProfile) {
      await bootstrap();
      return;
    }

    setError("");
    setIsRefreshing(true);
    setStatus("Refreshing profile");
    try {
      let result = await api.refreshProfile(selectedProfile.id);
      let repairDetail = "";
      let repairWarnings: string[] = [];

      if (result.missingDependencies.length > 0) {
        setStatus("Repairing dependencies");
        setNotice({
          kind: "warning",
          title: "Installing dependencies",
          detail: missingDependencyDetail(result)
        });

        try {
          const dependencyResult = await api.bootstrapProfileDependencies(selectedProfile.id);
          repairDetail = dependencyRepairSummary(dependencyResult);
          repairWarnings = actionableDependencyWarnings(dependencyResult.warnings);
          result = await api.refreshProfile(selectedProfile.id);
        } catch (caughtError) {
          repairWarnings = [String(caughtError)];
        }
      }

      setProfiles((current) =>
        current.map((profile) => (profile.id === result.profile.id ? result.profile : profile))
      );
      setSelectedProfileId(result.profile.id);
      setDetection(result.detection);
      setInstalledMods(result.installedMods);
      setAnalysis(null);
      const hasWarnings = result.warnings.length > 0 || repairWarnings.length > 0;
      const wasRepaired = repairDetail.length > 0 && result.missingDependencies.length === 0;
      setStatus(hasWarnings ? "Needs attention" : "Ready");
      setNotice({
        kind: hasWarnings ? "warning" : "success",
        title: hasWarnings
          ? "Refresh found issues"
          : wasRepaired
            ? "Dependencies repaired"
            : "Profile refreshed",
        detail: [refreshSummary(result), repairDetail, repairWarnings[0]]
          .filter(Boolean)
          .join(" ")
      });
      if (configModal) {
        const refreshedMod = result.installedMods.find((mod) => mod.id === configModal.mod.id);
        if (refreshedMod) {
          setConfigModal({ mod: refreshedMod, files: [], isLoading: true });
          const files = await api.getModConfigDetails(result.profile.id, refreshedMod.id);
          setConfigModal({ mod: refreshedMod, files, isLoading: false });
        }
      }
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Refresh failed");
      setNotice({
        kind: "error",
        title: "Refresh failed",
        detail: String(caughtError)
      });
    } finally {
      setIsRefreshing(false);
    }
  }

  async function selectGameFolder() {
    const folder = await api.selectGameFolder();
    if (!folder) {
      return;
    }

    setProfileInput((current) => ({
      ...current,
      gamePath: folder,
      name: current.name || getFolderName(folder),
      gameId: undefined,
      engine: "unknown",
      loader: "none"
    }));
    setDetection(null);
    setIsDetecting(true);
    setStatus("Detecting game setup");
    setError("");

    try {
      const detectedSetup = await api.detectGameSetup(folder);
      setDetection(detectedSetup);
      setProfileInput((current) => ({
        ...current,
        gameId: detectedSetup.gameId,
        engine: detectedSetup.engine,
        loader: detectedSetup.loader
      }));
      const issue = getDetectionIssue(detectedSetup);
      setStatus(issue ? "Detection needs review" : "Game folder ready");
      setNotice(
        issue
          ? {
              kind: "warning",
              title: "Detection needs review",
              detail: issue
            }
          : {
              kind: "success",
              title: "Game folder ready",
              detail: detectionRouteDetail(detectedSetup)
            }
      );
    } catch (caughtError) {
      setProfileInput((current) => ({ ...current, engine: "unknown", loader: "none" }));
      setError(String(caughtError));
      setStatus("Detection failed");
    } finally {
      setIsDetecting(false);
    }
  }

  async function createProfile(event: FormEvent) {
    event.preventDefault();
    setError("");
    setStatus("Creating profile");

    try {
      const profile = await api.createProfile(profileInput);
      setProfiles((current) => [...current, profile]);
      setSelectedProfileId(profile.id);
      setProfileInput(emptyProfileInput);
      setDetection(null);
      setAnalysis(null);
      setStatus("Installing profile dependencies");
      let dependencyDetail = "";
      let dependencyWarnings: string[] = [];
      try {
        const dependencyResult = await api.bootstrapProfileDependencies(profile.id);
        await refreshInstalledMods(profile.id);
        if (dependencyResult.installedDependencies.length > 0) {
          dependencyDetail = ` Installed ${dependencyResult.installedDependencies.length} profile dependenc${dependencyResult.installedDependencies.length === 1 ? "y" : "ies"} automatically.`;
        } else if (dependencyResult.skippedDependencies.length > 0) {
          dependencyDetail = " Profile dependencies were already installed.";
        }
        dependencyWarnings = actionableDependencyWarnings(dependencyResult.warnings);
      } catch (caughtError) {
        dependencyWarnings = [String(caughtError)];
      }
      setNotice({
        kind:
          profile.engine === "unknown" || profile.loader === "none" || dependencyWarnings.length > 0
            ? "warning"
            : "success",
        title: "Profile created",
        detail:
          profile.engine === "unknown" || profile.loader === "none"
            ? "UniLoader saved the profile, but mod installs may need manual review if detection remains unclear."
            : `Drop a mod archive into the workspace to install it.${dependencyDetail}${
                dependencyWarnings.length > 0 ? ` ${dependencyWarnings[0]}` : ""
              }`
      });
      setStatus("Profile created");
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Profile failed");
    }
  }

  async function renameProfile(profile: GameProfile, name: string) {
    const nextName = name.trim();
    if (!nextName || nextName === profile.name) {
      setProfilePendingRename(null);
      return;
    }

    setError("");
    setStatus("Renaming profile");
    try {
      const updatedProfile = await api.renameProfile(profile.id, nextName);
      setProfiles((current) =>
        current.map((item) => (item.id === updatedProfile.id ? updatedProfile : item))
      );
      setStatus("Profile renamed");
      setNotice({
        kind: "success",
        title: "Profile renamed",
        detail: `${profile.name} is now ${updatedProfile.name}.`
      });
      setProfilePendingRename(null);
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Rename failed");
      setNotice({
        kind: "error",
        title: "Rename failed",
        detail: String(caughtError)
      });
    }
  }

  async function removeProfile(profile: GameProfile) {
    setError("");
    setStatus("Removing profile");
    try {
      const result = await api.removeProfile(profile.id);
      const nextProfiles = profiles.filter((item) => item.id !== profile.id);
      setProfiles(nextProfiles);

      if (selectedProfileId === profile.id) {
        const nextSelectedProfileId = nextProfiles[0]?.id ?? "";
        setSelectedProfileId(nextSelectedProfileId);
        setInstalledMods([]);
        setAnalysis(null);
        setNotice(null);
        if (nextSelectedProfileId) {
          await refreshInstalledMods(nextSelectedProfileId);
        }
      }

      setStatus("Profile removed");
      setProfilePendingRemoval(null);
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Profile removed",
        detail:
          result.removedModRecords > 0
            ? `${result.name} was removed with ${result.removedModRecords} UniLoader mod record(s).`
            : `${result.name} was removed.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Remove failed");
      setNotice({
        kind: "error",
        title: "Remove failed",
        detail: String(caughtError)
      });
    }
  }

  async function openProfileGameFolder(profile: GameProfile) {
    setError("");
    setStatus("Opening game folder");
    try {
      await api.openProfileGameFolder(profile.id);
      setStatus("Game folder opened");
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Open folder failed");
      setNotice({
        kind: "error",
        title: "Open folder failed",
        detail: String(caughtError)
      });
    }
  }

  async function reselectProfileGameFolder(profile: GameProfile) {
    const folder = await api.selectGameFolder();
    if (!folder) {
      return;
    }

    setError("");
    setStatus("Updating game folder");
    try {
      const result = await api.updateProfileGameFolder(profile.id, folder);
      setProfiles((current) =>
        current.map((item) => (item.id === result.profile.id ? result.profile : item))
      );
      if (selectedProfileId === result.profile.id) {
        setInstalledMods(result.installedMods);
        setDetection(result.detection);
        setAnalysis(null);
      }
      setStatus(result.warnings.length > 0 ? "Needs attention" : "Game folder updated");
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Game folder updated",
        detail:
          result.warnings[0] ??
          `Redeployed ${result.deployedFiles.length} file(s) into the selected game folder.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Folder update failed");
      setNotice({
        kind: "error",
        title: "Folder update failed",
        detail: String(caughtError)
      });
    }
  }

  async function exportProfileBundle(profileId: string) {
    const profile = profiles.find((item) => item.id === profileId);
    if (!profile) {
      setNotice({
        kind: "warning",
        title: "Select a profile",
        detail: "Choose the profile you want to export."
      });
      return;
    }

    setError("");
    setIsTransferringProfile(true);
    setStatus("Exporting profile");
    try {
      const result = await api.exportProfileBundle(profile.id, profile.name);
      if (!result) {
        setStatus("Export canceled");
        return;
      }
      setStatus(result.warnings.length > 0 ? "Export needs attention" : "Profile exported");
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Profile exported",
        detail:
          result.warnings[0] ??
          `${result.profileName}: ${result.exportedMods} mod(s) and ${result.exportedConfigFiles} config file(s) bundled.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Export failed");
      setNotice({
        kind: "error",
        title: "Export failed",
        detail: String(caughtError)
      });
    } finally {
      setIsTransferringProfile(false);
    }
  }

  async function importProfileBundle() {
    setError("");
    setIsTransferringProfile(true);
    setStatus("Importing profile");
    try {
      const result = await api.importProfileBundle();
      if (!result) {
        setStatus("Import canceled");
        return;
      }
      setProfiles((current) => [...current, result.profile]);
      setSelectedProfileId(result.profile.id);
      setInstalledMods(result.installedMods);
      setAnalysis(null);
      setDetection(null);
      setStatus(result.warnings.length > 0 ? "Import needs attention" : "Profile imported");
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Profile imported",
        detail:
          result.warnings[0] ??
          `${result.profile.name}: ${result.installedMods.length} mod(s), ${result.deployedFiles.length} deployed file(s), and ${result.configFilesWritten.length} config file(s) restored.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Import failed");
      setNotice({
        kind: "error",
        title: "Import failed",
        detail: String(caughtError)
      });
    } finally {
      setIsTransferringProfile(false);
    }
  }

  async function loadOnlineMods(profileId: string) {
    setError("");
    setIsDiscoveringMods(true);
    setStatus("Discovering online mods");
    try {
      const mods = await api.discoverOnlineMods(profileId);
      setOnlineMods(mods);
      setDiscoverLoadedProfileId(profileId);
      setStatus("Discovery ready");
      const profile = profiles.find((item) => item.id === profileId);
      setNotice({
        kind: "success",
        title: "Discovery updated",
        detail: `${profile?.name ?? "Selected profile"}: found ${mods.length} online mod(s).`
      });
    } catch (caughtError) {
      setOnlineMods([]);
      setError(String(caughtError));
      setStatus("Discovery failed");
      setNotice({
        kind: "error",
        title: "Discovery failed",
        detail: String(caughtError)
      });
    } finally {
      setIsDiscoveringMods(false);
    }
  }

  async function installOnlineMod(mod: OnlineModRecord) {
    if (!discoverProfileId) {
      setNotice({
        kind: "warning",
        title: "Select a profile",
        detail: "Choose the profile you want to install this mod into."
      });
      return;
    }

    if (!mod.installSupported) {
      setNotice({
        kind: "warning",
        title: "Install needs provider support",
        detail: mod.installNote ?? `${mod.providerLabel} install is not available yet.`
      });
      return;
    }

    setError("");
    setInstallingOnlineModId(mod.id);
    setStatus("Installing online mod");
    try {
      const result = await api.installDiscoveredMod(discoverProfileId, mod);
      setOnlineMods((current) =>
        current.map((item) => (item.id === mod.id ? { ...item, installed: true } : item))
      );
      if (discoverProfileId === selectedProfileId) {
        await refreshInstalledMods(discoverProfileId);
      }
      setStatus("Mod installed");
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Online mod installed",
        detail: installSuccessDetail(mod.name, result.filesWritten.length, result.warnings)
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Install failed");
      setNotice({
        kind: "error",
        title: "Online install failed",
        detail: String(caughtError)
      });
    } finally {
      setInstallingOnlineModId("");
    }
  }

  function openNexusAuthSettings() {
    setActiveView("settings");
    setNexusSettingsAttentionId((current) => current + 1);
    setStatus("Nexus auth needed");
  }

  async function openExternalUrl(url: string) {
    setError("");
    try {
      await api.openExternalUrl(url);
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Link failed");
      setNotice({
        kind: "error",
        title: "Could not open link",
        detail: String(caughtError)
      });
    }
  }

  async function chooseArchive() {
    if (!selectedProfile) {
      setNotice({
        kind: "warning",
        title: "No profile selected",
        detail: "Create or select a game profile before adding mods."
      });
      return;
    }

    setError("");
    setStatus("Choosing archive");
    try {
      const nextAnalysis = await api.selectAndAnalyzeArchive(selectedProfile.id);
      if (!nextAnalysis) {
        setStatus("Import canceled");
        return;
      }

      await installAnalysis(nextAnalysis);
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Install failed");
    }
  }

  async function chooseModFolder() {
    if (!selectedProfile) {
      setNotice({
        kind: "warning",
        title: "No profile selected",
        detail: "Create or select a game profile before adding mods."
      });
      return;
    }

    setError("");
    setStatus("Choosing mod folder");
    try {
      const nextAnalysis = await api.selectAndAnalyzeModFolder(selectedProfile.id);
      if (!nextAnalysis) {
        setStatus("Import canceled");
        return;
      }

      await installAnalysis(nextAnalysis);
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Install failed");
    }
  }

  async function handleArchivePath(archivePath: string) {
    if (!selectedProfile) {
      setNotice({
        kind: "warning",
        title: "No profile selected",
        detail: "Create or select a game profile before dropping mods."
      });
      return;
    }

    setError("");
    setStatus("Analyzing import");
    try {
      const nextAnalysis = await api.analyzeArchivePath(selectedProfile.id, archivePath);
      await installAnalysis(nextAnalysis);
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Install failed");
      setNotice({
        kind: "error",
        title: "Install failed",
        detail: String(caughtError)
      });
    }
  }

  async function installAnalysis(nextAnalysis: ArchiveAnalysis) {
    if (!selectedProfile) {
      return;
    }

    const plan = nextAnalysis.recommendedPlan;
    setAnalysis(nextAnalysis);

    if (!plan) {
      setStatus("No install route");
      setNotice({
        kind: "error",
        title: "No install route found",
        detail: "UniLoader could not find a safe adapter for this archive."
      });
      return;
    }

    setIsInstalling(true);
    setStatus("Installing mod and dependencies");
    try {
      const result = await api.installArchive({
        profileId: selectedProfile.id,
        archivePath: nextAnalysis.archivePath,
        archiveName: nextAnalysis.archiveName,
        plan
      });
      await refreshInstalledMods(selectedProfile.id);
      setStatus("Mod installed");
      setNotice({
        kind: "success",
        title: "Mod installed",
        detail: installSuccessDetail(nextAnalysis.archiveName, result.filesWritten.length, result.warnings)
      });
    } catch (caughtError) {
      setStatus("Install failed");
      setNotice({
        kind: "error",
        title: "Install failed",
        detail: String(caughtError)
      });
      throw caughtError;
    } finally {
      setIsInstalling(false);
    }
  }

  async function handleModAction(
    action: "enable" | "disable" | "remove",
    mod: InstalledModRecord
  ) {
    if (!selectedProfile) {
      return;
    }

    setError("");
    setStatus(`${action[0].toUpperCase()}${action.slice(1)} mod`);
    try {
      const result =
        action === "enable"
          ? await api.enableMod(selectedProfile.id, mod.id)
          : action === "disable"
            ? await api.disableMod(selectedProfile.id, mod.id)
            : await api.removeMod(selectedProfile.id, mod.id);
      await refreshInstalledMods(selectedProfile.id);
      setStatus(result.status === "removed" ? "Mod removed" : "Mod updated");
      const modName = displayModName(mod);
      setNotice({
        kind: result.status === "removed" ? "warning" : "success",
        title:
          result.status === "removed"
            ? "Mod removed"
            : result.status === "disabled"
              ? "Mod disabled"
              : "Mod enabled",
        detail: `${modName}: ${result.filesChanged.length} file(s) changed.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Action failed");
      setNotice({
        kind: "error",
        title: "Action failed",
        detail: String(caughtError)
      });
    }
  }

  async function openModConfig(mod: InstalledModRecord) {
    if (!selectedProfile) {
      return;
    }

    setConfigModal({ mod, files: [], isLoading: true });
    try {
      const files = await api.getModConfigDetails(selectedProfile.id, mod.id);
      setConfigModal((current) =>
        current?.mod.id === mod.id ? { ...current, files, isLoading: false } : current
      );
    } catch (caughtError) {
      setConfigModal((current) =>
        current?.mod.id === mod.id
          ? { ...current, files: [], isLoading: false, error: String(caughtError) }
          : current
      );
    }
  }

  async function saveModConfigValue(
    file: ModConfigFile,
    entry: ModConfigEntry,
    value: string
  ) {
    if (!selectedProfile) {
      throw new Error("Create or select a game profile before editing config files.");
    }

    const updatedFile = await api.updateModConfigValue({
      profileId: selectedProfile.id,
      filePath: file.path,
      section: entry.section,
      key: entry.key,
      value
    });

    setConfigModal((current) =>
      current
        ? {
            ...current,
            files: current.files.map((item) => (item.path === updatedFile.path ? updatedFile : item))
          }
        : current
    );
  }

  function startWindowDrag(event: ReactMouseEvent<HTMLElement>) {
    if (event.button !== 0) {
      return;
    }

    const target = event.target;
    if (
      target instanceof Element &&
      target.closest("button, input, textarea, select, label, a")
    ) {
      return;
    }

    event.preventDefault();
    void api.startWindowDrag();
  }

  const profileToRename = profileRenamePresence.value;
  const profileToRemove = profileRemovalPresence.value;

  return (
    <>
    {startupSplashPhase !== "hidden" ? <StartupSplash phase={startupSplashPhase} /> : null}
    <main className={renderedView === "manager" ? "app-shell" : "app-shell settings-shell"}>
      <div
        className="window-drag-region"
        onMouseDown={startWindowDrag}
      />
      <WindowControls
        onClose={() => void api.closeWindow()}
        onMaximize={() => void api.toggleMaximizeWindow()}
        onMinimize={() => void api.minimizeWindow()}
      />
      <aside className="nav-rail">
        <div className="rail-brand" title="UniLoader">
          <UniLoaderMark />
        </div>
        <nav className="rail-nav" aria-label="Primary">
          <button
            className={activeView === "manager" ? "rail-button active" : "rail-button"}
            onClick={() => setActiveView("manager")}
            title="Mod manager"
            type="button"
          >
            <Home size={18} />
          </button>
          <button
            className={activeView === "discover" ? "rail-button active" : "rail-button"}
            onClick={() => setActiveView("discover")}
            title="Discover online mods"
            type="button"
          >
            <Compass size={18} />
          </button>
          <button
            className={activeView === "settings" ? "rail-button active" : "rail-button"}
            onClick={() => setActiveView("settings")}
            title="Settings"
            type="button"
          >
            <Settings2 size={18} />
          </button>
          <button
            className={activeView === "transfer" ? "rail-button active" : "rail-button"}
            onClick={() => setActiveView("transfer")}
            title="Import / export profiles"
            type="button"
          >
            <Upload size={18} />
          </button>
        </nav>
        <div className="rail-footer">
          <UpdateRailIndicator
            isChecking={isCheckingForUpdate}
            isDownloading={isDownloadingUpdate}
            updateInfo={updateInfo}
            onClick={() => void showUpdateDetails()}
          />
          <div className="rail-status" title={status} />
          <span className="app-version" title={`UniLoader ${appDisplayVersion}`}>
            {appDisplayVersion}
          </span>
        </div>
      </aside>
      {renderedView === "manager" ? (
      <aside className={`sidebar view-motion ${viewMotion.className}`}>
        <div className="brand-row">
          <div className="brand-mark">
            <UniLoaderMark />
          </div>
          <div className="brand-copy">
            <h1>UniLoader</h1>
            <p>Cross-game mod manager</p>
          </div>
        </div>

        <section className="panel">
          <div className="panel-heading">
            <Database size={17} />
            <h2>Profiles</h2>
          </div>
          <div className="profile-list">
            {profiles.map((profile) => (
              <div
                className={profile.id === selectedProfileId ? "profile active" : "profile"}
                key={profile.id}
              >
                <button
                  className="profile-select"
                  onClick={() => {
                    setSelectedProfileId(profile.id);
                    setAnalysis(null);
                    setNotice(null);
                  }}
                  type="button"
                >
                  <span>{profile.name}</span>
                  <small>{profile.engine === "unknown" || profile.loader === "none" ? "Needs review" : "Ready"}</small>
                </button>
                <div className="profile-actions">
                  <button
                    aria-label={`Rename ${profile.name}`}
                    onClick={() => setProfilePendingRename(profile)}
                    title="Rename profile"
                    type="button"
                  >
                    E
                  </button>
                  <button
                    aria-label={`Open ${profile.name} game folder`}
                    className="folder"
                    onClick={() => void openProfileGameFolder(profile)}
                    title="Open game folder"
                    type="button"
                  >
                    F
                  </button>
                  <button
                    aria-label={`Select ${profile.name} game folder`}
                    className="select-folder"
                    onClick={() => void reselectProfileGameFolder(profile)}
                    title="Select game folder"
                    type="button"
                  >
                    S
                  </button>
                  <button
                    aria-label={`Remove ${profile.name}`}
                    className="remove"
                    onClick={() => setProfilePendingRemoval(profile)}
                    title="Remove profile"
                    type="button"
                  >
                    X
                  </button>
                </div>
              </div>
            ))}
            {profiles.length === 0 ? <p className="muted">No profiles yet.</p> : null}
          </div>
        </section>

        <section className="panel">
          <div className="panel-heading">
            <FolderOpen size={17} />
            <h2>New Profile</h2>
          </div>
          <form className="profile-form" onSubmit={createProfile}>
            <label>
              Game name
              <input
                value={profileInput.name}
                onChange={(event) =>
                  setProfileInput((current) => ({ ...current, name: event.target.value }))
                }
                placeholder="Windrose"
              />
            </label>
            <label>
              Main game folder
              <div className="folder-input">
                <input readOnly value={profileInput.gamePath} placeholder="Select folder" />
                <button type="button" className="icon-button" onClick={selectGameFolder}>
                  <FolderOpen size={17} />
                </button>
              </div>
            </label>
            <DetectionWarning
              detection={detection}
              detectionIssue={detectionIssue}
              isDetecting={isDetecting}
            />
            <button className="primary-button" type="submit">
              <PackagePlus size={17} />
              Create Profile
            </button>
          </form>
        </section>
      </aside>
      ) : null}

      <section
        className={`${
          renderedView === "settings"
            ? "workspace settings-workspace"
            : renderedView === "transfer"
              ? "workspace transfer-workspace"
              : renderedView === "discover"
                ? "workspace discover-workspace"
                : "workspace"
        } view-motion ${viewMotion.className}`}
      >
        {renderedView === "manager" ? (
          <>
        <header className="topbar" onMouseDown={startWindowDrag}>
          <div className="topbar-copy">
            <p className="eyebrow">Active profile</p>
            <h2>{selectedProfile?.name ?? "No profile selected"}</h2>
          </div>
        </header>

        <div className="context-strip compact">
          <div className="context-item">
            <span>Profiles</span>
            <strong>{profiles.length}</strong>
          </div>
          <div className="context-item">
            <span>Available mods</span>
            <strong>{installedMods.length}</strong>
          </div>
          <div className="context-item">
            <span>Enabled</span>
            <strong>{installedMods.filter((mod) => mod.enabled).length}</strong>
          </div>
          <div className="context-item">
            <span>Disabled</span>
            <strong>{installedMods.filter((mod) => !mod.enabled).length}</strong>
          </div>
        </div>

        {errorPresence.value ? (
          <div className={`error-banner ${errorPresence.className}`} key={errorMotionId}>
            <AlertTriangle size={18} />
            <span>{errorPresence.value}</span>
          </div>
        ) : null}

        {noticePresence.value ? (
          <NoticeBanner
            key={noticePresence.value.motionId}
            notice={noticePresence.value}
            motionClassName={noticePresence.className}
          />
        ) : null}

          <div className="main-grid">
            <section className="work-panel analysis-panel">
              <div className="panel-title-row">
                <div>
                  <p className="eyebrow">Add mod</p>
                  <h3>Drop & Install</h3>
                </div>
                <div className="import-actions">
                  <button
                    className="primary-button"
                    disabled={!selectedProfile || isInstalling}
                    onClick={chooseArchive}
                    type="button"
                  >
                    <Download size={17} />
                    Choose File
                  </button>
                  <button
                    className="primary-button"
                    disabled={!selectedProfile || isInstalling}
                    onClick={chooseModFolder}
                    type="button"
                  >
                    <FolderOpen size={17} />
                    Choose Folder
                  </button>
                </div>
              </div>

              <DropZone
                analysis={analysis}
                isDragOver={isDragOver}
                isInstalling={isInstalling}
                selectedPlan={selectedPlan}
                selectedProfile={selectedProfile}
                onBrowserDrop={handleArchivePath}
              />
            </section>

            <section className="work-panel history-panel">
              <div className="panel-title-row compact">
                <div>
                  <p className="eyebrow">Mod library</p>
                  <h3>Available Mods</h3>
                </div>
                <div className="library-toolbar">
                  <div className="sort-toggle" aria-label="Sort installed mods">
                    <button
                      className={modSortMode === "newest" ? "active" : ""}
                      onClick={() => setModSortMode("newest")}
                      type="button"
                    >
                      Newest
                    </button>
                    <button
                      className={modSortMode === "oldest" ? "active" : ""}
                      onClick={() => setModSortMode("oldest")}
                      type="button"
                    >
                      Oldest
                    </button>
                  </div>
                  <span className="library-count">{installedMods.length}</span>
                </div>
              </div>
              <div className="history-list">
                {sortedInstalledMods.map((mod) => (
                  <ModCard
                    key={mod.id}
                    mod={mod}
                    onConfigure={() => void openModConfig(mod)}
                    onEnable={() => void handleModAction("enable", mod)}
                    onDisable={() => void handleModAction("disable", mod)}
                    onRemove={() => void handleModAction("remove", mod)}
                  />
                ))}
                {installedMods.length === 0 ? (
                  <div className="empty-mods">
                    <SlidersHorizontal size={22} />
                    <p>No mods installed yet.</p>
                  </div>
                ) : null}
              </div>
            </section>
          </div>
          </>
        ) : renderedView === "transfer" ? (
          <TransferView
            isBusy={isTransferringProfile}
            mode={transferMode}
            profiles={profiles}
            selectedProfileId={selectedProfileId}
            onExport={(profileId) => void exportProfileBundle(profileId)}
            onImport={() => void importProfileBundle()}
            onSelectMode={setTransferMode}
          />
        ) : renderedView === "discover" ? (
          <DiscoverView
            hasLoaded={discoverLoadedProfileId === discoverProfileId}
            installingModId={installingOnlineModId}
            isLoading={isDiscoveringMods}
            mods={displayedOnlineMods}
            profiles={profiles}
            selectedProfileId={discoverProfileId}
            onInstall={(mod) => void installOnlineMod(mod)}
            onNeedsAuth={openNexusAuthSettings}
            onRefresh={() => void loadOnlineMods(discoverProfileId)}
            onSelectProfile={setDiscoverProfileId}
          />
        ) : (
          <SettingsView
            appSettings={appSettings}
            nexusAttentionId={nexusSettingsAttentionId}
            onOpenExternalUrl={(url) => void openExternalUrl(url)}
            onUpdateSettings={updateAppSetting}
          />
        )}
      </section>
      {renderedView === "manager" ? (
      <HealthPanel
        healthMessage={healthMessage}
        healthTone={healthTone}
        isRefreshing={isRefreshing}
        motionClassName={viewMotion.className}
        status={status}
        onRefresh={() => void refreshSelectedProfile()}
      />
      ) : null}
    </main>
    {configModalPresence.value ? (
      <ConfigModal
        motionClassName={configModalPresence.className}
        state={configModalPresence.value}
        onClose={() => setConfigModal(null)}
        onSaveValue={saveModConfigValue}
      />
    ) : null}
    {profileToRename ? (
      <ProfileRenameDialog
        motionClassName={profileRenamePresence.className}
        profile={profileToRename}
        onCancel={() => setProfilePendingRename(null)}
        onConfirm={(nextName) => void renameProfile(profileToRename, nextName)}
      />
    ) : null}
    {profileToRemove ? (
      <ProfileRemovalDialog
        motionClassName={profileRemovalPresence.className}
        profile={profileToRemove}
        onCancel={() => setProfilePendingRemoval(null)}
        onConfirm={() => void removeProfile(profileToRemove)}
      />
    ) : null}
    </>
  );
}

function motionClassName(phase: MotionPhase): string {
  return phase === "exiting" ? "ui-motion-exit" : "ui-motion-enter";
}

function useFadePresence<T>(
  value: T | null,
  durationMs = motionDurationMs
): MotionPresence<T> {
  const [renderedValue, setRenderedValue] = useState<T | null>(value);
  const [phase, setPhase] = useState<MotionPhase>("entering");

  useEffect(() => {
    if (value !== null) {
      setRenderedValue(value);
      setPhase("entering");
      return undefined;
    }

    if (renderedValue === null) {
      return undefined;
    }

    setPhase("exiting");
    const timeoutId = window.setTimeout(() => {
      setRenderedValue(null);
      setPhase("entering");
    }, durationMs);

    return () => window.clearTimeout(timeoutId);
  }, [durationMs, renderedValue, value]);

  return {
    className: motionClassName(phase),
    phase,
    value: renderedValue
  };
}

function useFadeSwitch<T>(value: T, durationMs = motionDurationMs): MotionState<T> {
  const [renderedValue, setRenderedValue] = useState<T>(value);
  const [phase, setPhase] = useState<MotionPhase>("entering");

  useEffect(() => {
    if (Object.is(value, renderedValue)) {
      setPhase("entering");
      return undefined;
    }

    setPhase("exiting");
    const timeoutId = window.setTimeout(() => {
      setRenderedValue(value);
      setPhase("entering");
    }, durationMs);

    return () => window.clearTimeout(timeoutId);
  }, [durationMs, renderedValue, value]);

  return {
    className: motionClassName(phase),
    phase,
    value: renderedValue
  };
}

function getFolderName(folderPath: string): string {
  const parts = folderPath.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "";
}

interface ProfileRenameDialogProps {
  motionClassName: string;
  profile: GameProfile;
  onCancel(): void;
  onConfirm(name: string): void;
}

function ProfileRenameDialog({
  motionClassName,
  onCancel,
  onConfirm,
  profile
}: ProfileRenameDialogProps) {
  const [name, setName] = useState(profile.name);
  const nextName = name.trim();
  const canSave = nextName.length > 0 && nextName !== profile.name;

  useEffect(() => {
    setName(profile.name);
  }, [profile.id, profile.name]);

  return (
    <div className={`modal-backdrop ${motionClassName}`} onMouseDown={onCancel}>
      <form
        aria-label={`Rename ${profile.name} profile`}
        className="confirm-modal rename-modal"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          if (canSave) {
            onConfirm(nextName);
          }
        }}
        role="dialog"
      >
        <div className="confirm-icon rename-icon">
          <Database size={22} />
        </div>
        <div className="confirm-copy">
          <p className="eyebrow">Rename Profile</p>
          <h3>{profile.name}</h3>
          <label className="rename-field">
            Profile name
            <input
              autoFocus
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Profile name"
            />
          </label>
        </div>
        <div className="confirm-actions">
          <button className="secondary-button compact-button" onClick={onCancel} type="button">
            Cancel
          </button>
          <button className="primary-button compact-button" disabled={!canSave} type="submit">
            Save
          </button>
        </div>
      </form>
    </div>
  );
}

interface ProfileRemovalDialogProps {
  motionClassName: string;
  profile: GameProfile;
  onCancel(): void;
  onConfirm(): void;
}

function ProfileRemovalDialog({
  motionClassName,
  profile,
  onCancel,
  onConfirm
}: ProfileRemovalDialogProps) {
  return (
    <div className={`modal-backdrop ${motionClassName}`} onMouseDown={onCancel}>
      <section
        aria-label={`Remove ${profile.name} profile`}
        className="confirm-modal"
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <div className="confirm-icon">
          <AlertTriangle size={22} />
        </div>
        <div className="confirm-copy">
          <p className="eyebrow">Remove Profile</p>
          <h3>{profile.name}</h3>
          <p>
            This removes the profile and UniLoader's local records. It does not delete files from
            the game folder.
          </p>
        </div>
        <div className="confirm-actions">
          <button className="secondary-button compact-button" onClick={onCancel} type="button">
            Cancel
          </button>
          <button className="danger-button compact-button" onClick={onConfirm} type="button">
            Remove
          </button>
        </div>
      </section>
    </div>
  );
}

interface WindowControlsProps {
  onMinimize(): void;
  onMaximize(): void;
  onClose(): void;
}

function WindowControls({ onClose, onMaximize, onMinimize }: WindowControlsProps) {
  return (
    <div className="window-controls" aria-label="Window controls">
      <button aria-label="Minimize" onClick={onMinimize} type="button">
        <Minus size={14} />
      </button>
      <button aria-label="Maximize" onClick={onMaximize} type="button">
        <Square size={12} />
      </button>
      <button aria-label="Close" className="close" onClick={onClose} type="button">
        <X size={15} />
      </button>
    </div>
  );
}

interface UpdateRailIndicatorProps {
  isChecking: boolean;
  isDownloading: boolean;
  updateInfo: AppUpdateInfo | null;
  onClick(): void;
}

function UpdateRailIndicator({
  isChecking,
  isDownloading,
  updateInfo,
  onClick
}: UpdateRailIndicatorProps) {
  const shouldShow =
    isChecking || isDownloading || updateInfo?.updateAvailable || updateInfo?.status === "error";

  if (!shouldShow) {
    return null;
  }

  const tone = updateInfo?.updateAvailable ? "available" : "warning";
  const title = updateInfo?.updateAvailable
    ? `Download UniLoader ${updateInfo.latestVersion ? `v${updateInfo.latestVersion}` : "update"}`
    : isDownloading
      ? "Downloading update"
      : isChecking
      ? "Checking for updates"
      : updateInfo?.message ?? "Update check needs attention";

  return (
    <button
      aria-label={title}
      className={`rail-update-button ${tone}`}
      disabled={isChecking || isDownloading}
      onClick={onClick}
      title={title}
      type="button"
    >
      <Download className={isChecking || isDownloading ? "spin-icon" : ""} size={16} />
    </button>
  );
}

interface HealthPanelProps {
  healthMessage: string;
  healthTone: NoticeKind;
  isRefreshing: boolean;
  motionClassName?: string;
  status: string;
  onRefresh(): void;
}

function HealthPanel({
  healthMessage,
  healthTone,
  isRefreshing,
  motionClassName = "",
  status,
  onRefresh
}: HealthPanelProps) {
  return (
    <div className={`health-panel health-dock ${healthTone} ${motionClassName}`}>
      <div className="health-orb">
        <Activity size={22} strokeWidth={2.35} />
      </div>
      <div className="health-copy">
        <span>Health</span>
        <strong>{healthMessage}</strong>
      </div>
      <span className="health-status">{status}</span>
      <button
        className="health-refresh"
        disabled={isRefreshing}
        onClick={onRefresh}
        title="Rescan selected profile"
        type="button"
      >
        <RefreshCw size={15} />
        {isRefreshing ? "Refreshing" : "Refresh"}
      </button>
    </div>
  );
}

function UniLoaderMark() {
  return (
    <svg aria-hidden="true" viewBox="0 0 48 48">
      <path
        d="M13 13v14c0 6.6 4.9 11 11 11s11-4.4 11-11V13"
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="5"
      />
      <path
        d="M9 20 13 13l4 7M31 20l4-7 4 7M24 7v18M17 18l7 7 7-7"
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="4"
      />
    </svg>
  );
}

function StartupSplash({ phase }: { phase: StartupSplashPhase }) {
  return (
    <div
      aria-label="UniLoader starting"
      className={phase === "exiting" ? "startup-splash exiting" : "startup-splash"}
      role="status"
    >
      <div className="startup-splash-mark">
        <UniLoaderMark />
      </div>
    </div>
  );
}

interface DiscoverViewProps {
  hasLoaded: boolean;
  installingModId: string;
  isLoading: boolean;
  mods: OnlineModRecord[];
  profiles: GameProfile[];
  selectedProfileId: string;
  onInstall(mod: OnlineModRecord): void;
  onNeedsAuth(): void;
  onRefresh(): void;
  onSelectProfile(profileId: string): void;
}

function DiscoverView({
  hasLoaded,
  installingModId,
  isLoading,
  mods,
  profiles,
  selectedProfileId,
  onInstall,
  onNeedsAuth,
  onRefresh,
  onSelectProfile
}: DiscoverViewProps) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(1);
  const [sortMode, setSortMode] = useState<OnlineSortMode>("downloads");
  const selectedProfile = profiles.find((profile) => profile.id === selectedProfileId);
  const normalizedQuery = query.toLowerCase().trim();
  const filteredMods = useMemo(
    () =>
      normalizedQuery
        ? mods.filter((mod) =>
            [
              mod.name,
              mod.owner,
              mod.description,
              mod.providerLabel,
              mod.categories.join(" ")
            ]
              .join(" ")
              .toLowerCase()
              .includes(normalizedQuery)
          )
        : mods,
    [mods, normalizedQuery]
  );
  const sortedOnlineMods = useMemo(
    () =>
      [...filteredMods].sort((first, second) => {
        const firstTime = onlineModTimestamp(first);
        const secondTime = onlineModTimestamp(second);

        if (sortMode === "newest") {
          return (
            secondTime - firstTime ||
            second.downloads - first.downloads ||
            second.ratingScore - first.ratingScore
          );
        }

        if (sortMode === "oldest") {
          return (
            firstTime - secondTime ||
            second.downloads - first.downloads ||
            second.ratingScore - first.ratingScore
          );
        }

        return (
          second.downloads - first.downloads ||
          second.ratingScore - first.ratingScore ||
          first.name.localeCompare(second.name)
        );
      }),
    [filteredMods, sortMode]
  );
  const pageCount = Math.max(1, Math.ceil(sortedOnlineMods.length / discoverPageSize));
  const currentPage = Math.min(page, pageCount);
  const pageStart = (currentPage - 1) * discoverPageSize;
  const visibleMods = useMemo(
    () => sortedOnlineMods.slice(pageStart, pageStart + discoverPageSize),
    [sortedOnlineMods, pageStart]
  );

  useEffect(() => {
    setPage(1);
  }, [normalizedQuery, selectedProfileId, mods, sortMode]);

  return (
    <div className="discover-layout">
      <div className="discover-hero">
        <div>
          <p className="eyebrow">Discover</p>
          <h2>Online Mods</h2>
          <span>
            {selectedProfile
              ? `${selectedProfile.name} results sorted by popularity`
              : "Select a profile to load provider results"}
          </span>
        </div>
        <button
          className="secondary-button"
          disabled={!selectedProfileId || isLoading}
          onClick={onRefresh}
          type="button"
        >
          <RefreshCw size={16} />
          {isLoading ? "Loading" : hasLoaded ? "Refresh" : "Load Online Mods"}
        </button>
      </div>

      <section className="work-panel discover-controls">
        <label className="transfer-field">
          Profile
          <select
            value={selectedProfileId}
            onChange={(event) => onSelectProfile(event.target.value)}
          >
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.name}
              </option>
            ))}
          </select>
        </label>
        <label className="discover-search">
          <Search size={17} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search online mods"
          />
          {query ? (
            <button onClick={() => setQuery("")} title="Clear search" type="button">
              X
            </button>
          ) : null}
        </label>
        <div className="discover-provider-pill" title={`${sortedOnlineMods.length} total online mods`}>
          <Compass size={17} />
          <span>Total Mods</span>
          <strong>{formatCompactNumber(sortedOnlineMods.length)}</strong>
        </div>
        <div className="sort-toggle discover-sort" aria-label="Sort online mods">
          <button
            className={sortMode === "downloads" ? "active" : ""}
            onClick={() => setSortMode("downloads")}
            type="button"
          >
            Total Downloads
          </button>
          <button
            className={sortMode === "newest" ? "active" : ""}
            onClick={() => setSortMode("newest")}
            type="button"
          >
            Newest
          </button>
          <button
            className={sortMode === "oldest" ? "active" : ""}
            onClick={() => setSortMode("oldest")}
            type="button"
          >
            Oldest
          </button>
        </div>
      </section>

      <section className="discover-results" aria-label="Online mod results">
        {visibleMods.map((mod) => (
          <OnlineModCard
            installing={installingModId === mod.id}
            key={`${mod.provider}:${mod.id}:${mod.version}`}
            mod={mod}
            onInstall={onInstall}
            onNeedsAuth={onNeedsAuth}
          />
        ))}
        {!isLoading && !hasLoaded ? (
          <div className="empty-mods discover-empty">
            <Compass size={26} />
            <p>Select a profile, then load online mods when you are ready.</p>
          </div>
        ) : null}
        {!isLoading && hasLoaded && mods.length === 0 ? (
          <div className="empty-mods discover-empty">
            <Compass size={26} />
            <p>No online provider results were found for this profile.</p>
          </div>
        ) : null}
        {!isLoading && hasLoaded && mods.length > 0 && visibleMods.length === 0 ? (
          <div className="empty-mods discover-empty">
            <Search size={24} />
            <p>No online mods match that search.</p>
          </div>
        ) : null}
        {!isLoading && filteredMods.length > discoverPageSize ? (
          <div className="discover-pagination">
            <button
              className="secondary-button compact-button"
              disabled={currentPage <= 1}
              onClick={() => setPage((value) => Math.max(1, value - 1))}
              type="button"
            >
              Previous
            </button>
            <span>
              Page {currentPage} / {pageCount}
            </span>
            <button
              className="secondary-button compact-button"
              disabled={currentPage >= pageCount}
              onClick={() => setPage((value) => Math.min(pageCount, value + 1))}
              type="button"
            >
              Next
            </button>
          </div>
        ) : null}
        {isLoading ? (
          <div className="empty-mods discover-empty">
            <RefreshCw size={24} />
            <p>Loading online mods...</p>
          </div>
        ) : null}
      </section>
    </div>
  );
}

interface OnlineModCardProps {
  installing: boolean;
  mod: OnlineModRecord;
  onInstall(mod: OnlineModRecord): void;
  onNeedsAuth(): void;
}

const OnlineModCard = memo(function OnlineModCard({
  installing,
  mod,
  onInstall,
  onNeedsAuth
}: OnlineModCardProps) {
  const needsAuth = !mod.installSupported && mod.provider === "nexus";
  const installDisabled = mod.installed || installing || (!mod.installSupported && !needsAuth);
  const installTitle = !mod.installSupported
    ? (mod.installNote ?? `${mod.providerLabel} direct install is not available yet.`)
    : undefined;

  return (
    <article className={mod.installed ? "online-mod-card installed" : "online-mod-card"}>
      <div className="online-mod-icon">
        {mod.iconUrl ? (
          <img alt="" decoding="async" loading="lazy" src={mod.iconUrl} />
        ) : (
          <PackagePlus size={24} />
        )}
      </div>
      <div className="online-mod-copy">
        <div className="online-mod-heading">
          <div>
            <p className="eyebrow">{mod.providerLabel}</p>
            <h3>{mod.name}</h3>
          </div>
          <span className={mod.installed ? "online-mod-state installed" : "online-mod-state"}>
            {mod.installed ? "Installed" : `v${mod.version}`}
          </span>
        </div>
        <p>{mod.description || "No description provided."}</p>
        <div className="online-mod-meta">
          <span>{mod.owner}</span>
          <span>{formatCompactNumber(mod.downloads)} downloads</span>
          <span>{mod.dependencyCount} dependenc{mod.dependencyCount === 1 ? "y" : "ies"}</span>
          {mod.fileSize ? <span>{formatFileSize(mod.fileSize)}</span> : null}
        </div>
        {mod.categories.length > 0 ? (
          <div className="dependency-chips">
            {mod.categories.slice(0, 4).map((category) => (
              <span key={category}>{category}</span>
            ))}
          </div>
        ) : null}
      </div>
      <div className="online-mod-actions">
        {mod.packageUrl ? (
          <a className="secondary-button compact-button" href={mod.packageUrl} target="_blank" rel="noreferrer">
            Page
          </a>
        ) : null}
        <button
          className={needsAuth ? "secondary-button compact-button auth-button" : "primary-button compact-button"}
          disabled={installDisabled}
          onClick={() => (needsAuth ? onNeedsAuth() : onInstall(mod))}
          title={installTitle}
          type="button"
        >
          {needsAuth ? <Settings2 size={15} /> : <Download size={15} />}
          {mod.installed
            ? "Installed"
            : installing
              ? "Installing"
              : mod.installSupported
                ? "Install"
                : "Needs Auth"}
        </button>
      </div>
    </article>
  );
});

interface TransferViewProps {
  isBusy: boolean;
  mode: TransferMode;
  profiles: GameProfile[];
  selectedProfileId: string;
  onExport(profileId: string): void;
  onImport(): void;
  onSelectMode(mode: TransferMode): void;
}

function TransferView({
  isBusy,
  mode,
  profiles,
  selectedProfileId,
  onExport,
  onImport,
  onSelectMode
}: TransferViewProps) {
  const [exportProfileId, setExportProfileId] = useState(selectedProfileId);

  useEffect(() => {
    setExportProfileId((current) =>
      current && profiles.some((profile) => profile.id === current)
        ? current
        : selectedProfileId || profiles[0]?.id || ""
    );
  }, [profiles, selectedProfileId]);

  return (
    <div className="transfer-layout">
      <div className="transfer-hero">
        <p className="eyebrow">Profile transfer</p>
        <h2>Import / Export</h2>
      </div>

      <div className="transfer-choice-grid">
        <button
          className={mode === "import" ? "transfer-choice active" : "transfer-choice"}
          onClick={() => onSelectMode("import")}
          type="button"
        >
          <Download size={24} />
          <strong>Import</strong>
          <span>Restore a shared UniLoader profile bundle into a selected game folder.</span>
        </button>
        <button
          className={mode === "export" ? "transfer-choice active" : "transfer-choice"}
          onClick={() => onSelectMode("export")}
          type="button"
        >
          <Upload size={24} />
          <strong>Export</strong>
          <span>Bundle one profile with its managed mods and detected config files.</span>
        </button>
      </div>

      {mode ? (
        <section className="work-panel transfer-step-panel ui-motion-enter" key={mode}>
          <div className="panel-title-row">
            <div>
              <p className="eyebrow">{mode === "import" ? "Import" : "Export"}</p>
              <h3>{mode === "import" ? "Restore Profile Bundle" : "Create Profile Bundle"}</h3>
            </div>
            {mode === "import" ? <Download size={20} /> : <Upload size={20} />}
          </div>

          {mode === "import" ? (
            <>
              <div className="transfer-steps">
                <span>1. Choose a `.uniloader-profile` bundle.</span>
                <span>2. Select the matching game install folder.</span>
                <span>3. UniLoader recreates the profile and deploys enabled mods.</span>
              </div>
              <button
                className="primary-button transfer-action"
                disabled={isBusy}
                onClick={onImport}
                type="button"
              >
                <Download size={17} />
                {isBusy ? "Importing" : "Import Profile"}
              </button>
            </>
          ) : (
            <>
              <label className="transfer-field">
                Profile
                <select
                  value={exportProfileId}
                  onChange={(event) => setExportProfileId(event.target.value)}
                >
                  {profiles.map((profile) => (
                    <option key={profile.id} value={profile.id}>
                      {profile.name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="transfer-steps">
                <span>1. Pick the profile to share.</span>
                <span>2. Choose where to save the bundle.</span>
                <span>3. Send the saved file to a friend.</span>
              </div>
              <button
                className="primary-button transfer-action"
                disabled={isBusy || profiles.length === 0}
                onClick={() => onExport(exportProfileId)}
                type="button"
              >
                <Upload size={17} />
                {isBusy ? "Exporting" : "Export Profile"}
              </button>
            </>
          )}
        </section>
      ) : null}
    </div>
  );
}

interface SettingsViewProps {
  appSettings: AppSettings;
  nexusAttentionId: number;
  onOpenExternalUrl(url: string): void;
  onUpdateSettings(settings: AppSettings): Promise<void>;
}

function SettingsView({
  appSettings,
  nexusAttentionId,
  onOpenExternalUrl,
  onUpdateSettings
}: SettingsViewProps) {
  return (
    <div className="settings-grid">
      <section className="work-panel settings-panel">
        <div className="panel-title-row">
          <div>
            <p className="eyebrow">Settings</p>
            <h3>Window Behavior</h3>
          </div>
          <Settings2 size={20} />
        </div>

        <label className="setting-toggle-row">
          <div>
            <strong>Minimize to system tray on close</strong>
            <span>Close hides UniLoader; left-click tray to restore, right-click tray to quit.</span>
          </div>
          <input
            checked={appSettings.minimizeToTrayOnClose}
            onChange={(event) =>
              void onUpdateSettings({
                ...appSettings,
                minimizeToTrayOnClose: event.target.checked
              })
            }
            type="checkbox"
          />
        </label>
      </section>

      <section
        className={`work-panel settings-panel nexus-settings-panel ${
          nexusAttentionId > 0 ? "attention" : ""
        }`}
      >
        <div className="panel-title-row">
          <div>
            <p className="eyebrow">Provider</p>
            <h3>Nexus Mods</h3>
          </div>
          <Compass size={20} />
        </div>

        <label className="settings-field">
          <span>Personal API key</span>
          <input
            autoComplete="off"
            placeholder="Paste Nexus API key"
            type="password"
            value={appSettings.nexusApiKey}
            onChange={(event) =>
              void onUpdateSettings({
                ...appSettings,
                nexusApiKey: event.target.value.trim()
              })
            }
          />
        </label>
        <p className={appSettings.nexusApiKey.trim() ? "settings-status connected" : "settings-status"}>
          {appSettings.nexusApiKey.trim()
            ? "Nexus install support is enabled for discovered mods."
            : "Nexus results are browse-only until an API key is saved."}
        </p>
        <ol className="settings-steps">
          <li>
            <strong>Step 1</strong>
            <span>
              Log in at{" "}
              <a
                href={nexusApiKeysUrl}
                onClick={(event) => {
                  event.preventDefault();
                  onOpenExternalUrl(nexusApiKeysUrl);
                }}
              >
                Nexus Mods API Keys
              </a>
              .
            </span>
          </li>
          <li>
            <strong>Step 2</strong>
            <span>Scroll all the way to the bottom of the page.</span>
          </li>
          <li>
            <strong>Step 3</strong>
            <span>Request a personal API key.</span>
          </li>
          <li>
            <strong>Step 4</strong>
            <span>Paste the key into UniLoader.</span>
          </li>
        </ol>
      </section>
    </div>
  );
}

function installSuccessDetail(archiveName: string, fileCount: number, warnings: string[]): string {
  const dependencyNotes = warnings.filter((warning) => warning.startsWith("Installed dependency "));
  const base = `${archiveName} installed ${fileCount} file(s).`;

  if (dependencyNotes.length === 0) {
    return base;
  }

  return `${base} ${dependencyNotes.length} dependency package(s) installed automatically.`;
}

function onlineModTimestamp(mod: OnlineModRecord): number {
  return Date.parse(mod.updatedAt ?? mod.createdAt ?? "") || 0;
}

function formatCompactNumber(value: number): string {
  return Intl.NumberFormat(undefined, {
    maximumFractionDigits: value >= 1000 ? 1 : 0,
    notation: value >= 1000 ? "compact" : "standard"
  }).format(value);
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024 * 1024) {
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  }

  return `${(bytes / 1024 / 1024).toFixed(bytes >= 100 * 1024 * 1024 ? 0 : 1)} MB`;
}

function refreshSummary(result: ProfileRefreshResult): string {
  const configFileCount = result.installedMods.reduce(
    (total, mod) => total + (mod.configFiles?.length ?? 0),
    0
  );
  const missingFileCount = result.modFileHealth.reduce(
    (total, health) => total + health.missingFiles.length,
    0
  );
  const parts = [
    `Checked ${result.installedMods.length} mod(s) and ${configFileCount} config file(s).`
  ];

  if (result.detection.createdModFolders.length > 0) {
    parts.push(`Created ${result.detection.createdModFolders.length} missing route(s).`);
  }

  if (result.adoptedNativeScriptMods > 0) {
    parts.push(
      `Adopted ${result.adoptedNativeScriptMods} existing script mod${
        result.adoptedNativeScriptMods === 1 ? "" : "s"
      }.`
    );
  }

  if (missingFileCount > 0) {
    parts.push(`${missingFileCount} expected installed file(s) are missing.`);
  }

  if (result.missingDependencies.length > 0) {
    parts.push(
      `${result.missingDependencies.length} required dependenc${
        result.missingDependencies.length === 1 ? "y is" : "ies are"
      } still missing: ${dependencyNames(result.missingDependencies)}.`
    );
  }

  if (result.warnings.length > 0 && missingFileCount === 0 && result.missingDependencies.length === 0) {
    parts.push(result.warnings[0]);
  }

  return parts.join(" ");
}

function missingDependencyDetail(result: ProfileRefreshResult): string {
  return `Missing: ${dependencyNames(result.missingDependencies)}. UniLoader is trying to install what it can automatically.`;
}

function dependencyRepairSummary(result: ProfileDependencyBootstrapResult): string {
  if (result.installedDependencies.length > 0) {
    return `Installed ${result.installedDependencies.length} missing dependenc${
      result.installedDependencies.length === 1 ? "y" : "ies"
    }: ${result.installedDependencies.slice(0, 3).join(", ")}${
      result.installedDependencies.length > 3 ? ", ..." : ""
    }.`;
  }

  if (result.skippedDependencies.length > 0) {
    return "Required dependencies were already present after repair.";
  }

  return "";
}

function actionableDependencyWarnings(warnings: string[]): string[] {
  return warnings.filter((warning) => !warning.startsWith("Installed dependency "));
}

function dependencyNames(dependencies: { name: string }[]): string {
  return dependencies
    .slice(0, 3)
    .map((dependency) => dependency.name)
    .join(", ")
    .concat(dependencies.length > 3 ? ", ..." : "");
}

function displayModName(mod: InstalledModRecord): string {
  return polishModName(mod.displayName?.trim() || humanizeModName(mod.archiveName), mod);
}

function humanizeModName(rawName: string): string {
  const baseName = rawName
    .replace(/\\/g, "/")
    .split("/")
    .filter(Boolean)
    .pop() ?? rawName;
  const withoutExtension = baseName.replace(/\.(zip|7z|rar|pak|dll|lua|as)$/i, "");
  const withoutUnrealPakSuffix = withoutExtension.replace(/([_-])P$/i, "");
  const words = withoutUnrealPakSuffix
    .replace(/[^a-zA-Z0-9]+/g, " ")
    .trim()
    .split(/\s+/)
    .flatMap(splitReadableModWords)
    .filter(Boolean);

  let removedHostingTail = false;
  while (words.length > 0) {
    const lastWord = words[words.length - 1];
    if (isStrongHostingNoiseToken(lastWord)) {
      words.pop();
      removedHostingTail = true;
      continue;
    }

    if (removedHostingTail && /^\d+$/.test(lastWord)) {
      words.pop();
      continue;
    }

    break;
  }

  return polishModName(words.join(" ") || "Unknown Mod");
}

function polishModName(rawName: string, mod?: InstalledModRecord): string {
  let name = rawName
    .replace(/\bBep\s+In\s+Ex\b/gi, "BepInEx")
    .replace(/\bBepInExPack\b/gi, "BepInEx Pack")
    .replace(/\bUe\s+4\s+Ss\b/gi, "UE4SS")
    .replace(/\bRe\s+Framework\b/gi, "REFramework")
    .replace(/\s+(as|lua|dll|pak|zip|rar|7z)$/i, "")
    .replace(/\s+\d+\s+\d+\s+\d+(?:\s+\d+)?$/g, "")
    .trim();

  if (mod?.packageId?.startsWith("thunderstore:")) {
    const namespace = mod.packageId.slice("thunderstore:".length).split(/[/-]/)[0];
    const namespaceName = humanizeModName(namespace);
    name = name.replace(new RegExp(`^${escapeRegExp(namespaceName)}\\s+`, "i"), "");
  }

  name = name.replace(/^(denikson|thunderstore|nexusmods|nexus|overwolf)\s+/i, "");
  return name.replace(/\s+/g, " ") || "Unknown Mod";
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function splitReadableModWords(word: string): string[] {
  return word.match(/[A-Z]+(?=[A-Z][a-z]|\d|$)|[A-Z]?[a-z]+|\d+[a-zA-Z]*|\d+/g) ?? [word];
}

function isStrongHostingNoiseToken(token: string): boolean {
  const lower = token.toLowerCase();
  const hasDigit = /\d/.test(lower);
  const hasAlpha = /[a-z]/.test(lower);

  return (
    (lower.length >= 8 && /^[a-f0-9]+$/.test(lower)) ||
    (lower.length >= 8 && hasDigit) ||
    ["manual", "main", "latest", "version", "release", "file", "files"].includes(lower) ||
    (hasDigit && hasAlpha && lower.length >= 6)
  );
}

function getDetectionIssue(detection: GameDetectionResult | null): string {
  if (!detection) {
    return "";
  }

  if (detection.engine === "unknown" && detection.loader === "none") {
    return "UniLoader could not identify the game engine or loader route from this folder.";
  }

  if (detection.engine === "unknown") {
    return "UniLoader could not identify the game engine from this folder.";
  }

  if (detection.loader === "none") {
    return "UniLoader could not identify a loader route for this folder.";
  }

  return "";
}

function detectionRouteDetail(detection: GameDetectionResult): string {
  if (detection.createdModFolders.length > 0) {
    return `Created ${detection.createdModFolders.length} missing mod folder route(s).`;
  }

  if (detection.expectedModFolders.length > 0) {
    return "Mod folder routes are ready.";
  }

  return "UniLoader identified the setup and will choose the install route automatically.";
}

interface DetectionWarningProps {
  detection: GameDetectionResult | null;
  detectionIssue: string;
  isDetecting: boolean;
}

function DetectionWarning({ detection, detectionIssue, isDetecting }: DetectionWarningProps) {
  if (isDetecting) {
    return <div className="detection-card">Scanning selected folder...</div>;
  }

  if (!detection) {
    return (
      <div className="detection-card muted-card">
        UniLoader will detect the game setup after folder selection.
      </div>
    );
  }

  if (detectionIssue) {
    return (
      <div className="detection-card warning-card">
        <strong>Detection warning</strong>
        <span>{detectionIssue}</span>
      </div>
    );
  }

  return (
    <div className="detection-card success-card">
      <strong>Folder ready</strong>
      <span>{detectionRouteDetail(detection)}</span>
    </div>
  );
}

interface NoticeBannerProps {
  motionClassName?: string;
  notice: Notice;
}

function NoticeBanner({ motionClassName = "", notice }: NoticeBannerProps) {
  return (
    <div className={`notice-banner ${notice.kind} ${motionClassName}`}>
      {notice.kind === "success" ? <CheckCircle2 size={18} /> : <AlertTriangle size={18} />}
      <div>
        <strong>{notice.title}</strong>
        <span>{notice.detail}</span>
      </div>
    </div>
  );
}

interface DropZoneProps {
  analysis: ArchiveAnalysis | null;
  isDragOver: boolean;
  isInstalling: boolean;
  selectedPlan?: InstallPlan;
  selectedProfile?: GameProfile;
  onBrowserDrop(archivePath: string): void;
}

function DropZone({
  analysis,
  isDragOver,
  isInstalling,
  selectedPlan,
  selectedProfile,
  onBrowserDrop
}: DropZoneProps) {
  return (
    <div
      className={`drop-zone ${isDragOver ? "drag-over" : ""}`}
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault();
        const archivePath = Array.from(event.dataTransfer.files)
          .map((file) => (file as File & { path?: string }).path)
          .find((path): path is string => Boolean(path));
        if (archivePath) {
          onBrowserDrop(archivePath);
        }
      }}
    >
      <ShieldCheck size={42} />
      <h4>{isInstalling ? "Installing..." : selectedProfile ? "Drop mod file or folder here" : "Create a profile first"}</h4>
      <p>
        {selectedProfile
          ? "UniLoader accepts ZIP, 7Z, RAR, and folders, then chooses the safest adapter and reports the result."
          : "Add a game name and select the main game folder before importing mods."}
      </p>
      {analysis && selectedPlan ? (
        <div className="last-import">
          <span>Last import</span>
          <strong>{analysis.archiveName}</strong>
          <small>{selectedPlan.adapterName} - {Math.round(selectedPlan.confidence * 100)}% confidence</small>
        </div>
      ) : null}
    </div>
  );
}

interface ModCardProps {
  mod: InstalledModRecord;
  onConfigure(): void;
  onEnable(): void;
  onDisable(): void;
  onRemove(): void;
}

interface ConfigModalProps {
  motionClassName: string;
  state: ConfigModalState;
  onClose(): void;
  onSaveValue(file: ModConfigFile, entry: ModConfigEntry, value: string): Promise<void>;
}

function normalizeConfigSearchText(value: string): string {
  return value.toLowerCase().replace(/\s+/g, " ").trim();
}

function configEntryMatchesQuery(
  file: ModConfigFile,
  entry: ModConfigEntry,
  normalizedQuery: string
): boolean {
  if (!normalizedQuery) {
    return true;
  }

  return normalizeConfigSearchText(
    [
      file.fileName,
      file.path,
      entry.section ?? "General",
      entry.key,
      entry.description ?? "",
      entry.value,
      entry.defaultValue ?? "",
      entry.valueType ?? ""
    ].join(" ")
  ).includes(normalizedQuery);
}

function ConfigModal({ motionClassName, state, onClose, onSaveValue }: ConfigModalProps) {
  const modName = displayModName(state.mod);
  const [configQuery, setConfigQuery] = useState("");
  const normalizedQuery = normalizeConfigSearchText(configQuery);
  const filteredFiles = useMemo(
    () =>
      state.files
        .map((file) => {
          const entries = normalizedQuery
            ? file.entries.filter((entry) => configEntryMatchesQuery(file, entry, normalizedQuery))
            : file.entries;
          const rawPreviewMatches =
            normalizedQuery.length > 0 &&
            file.rawPreview &&
            normalizeConfigSearchText(`${file.fileName} ${file.path} ${file.rawPreview}`).includes(
              normalizedQuery
            );

          if (!normalizedQuery || entries.length > 0 || rawPreviewMatches) {
            return { entries, file, showRawPreview: Boolean(rawPreviewMatches) };
          }

          return null;
        })
        .filter((file): file is { entries: ModConfigEntry[]; file: ModConfigFile; showRawPreview: boolean } =>
          Boolean(file)
        ),
    [normalizedQuery, state.files]
  );
  const resultCount = filteredFiles.reduce(
    (count, file) => count + file.entries.length + (file.showRawPreview ? 1 : 0),
    0
  );

  useEffect(() => {
    setConfigQuery("");
  }, [state.mod.id]);

  return (
    <div className={`modal-backdrop ${motionClassName}`} onMouseDown={onClose}>
      <section
        aria-label={`${modName} configuration`}
        className="config-modal"
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <header className="config-modal-header">
          <div>
            <p className="eyebrow">Configuration</p>
            <h3>{modName}</h3>
          </div>
          <div className="config-header-actions">
            {!state.isLoading && !state.error && state.files.length > 0 ? (
              <div className="config-search" title="Search configuration settings">
                <Search size={15} />
                <input
                  aria-label="Search configuration settings"
                  onChange={(event) => setConfigQuery(event.target.value)}
                  placeholder="Search settings"
                  value={configQuery}
                />
                {configQuery ? (
                  <button
                    aria-label="Clear configuration search"
                    onClick={() => setConfigQuery("")}
                    type="button"
                  >
                    X
                  </button>
                ) : null}
              </div>
            ) : null}
            <button className="modal-close-button" onClick={onClose} type="button">
              X
            </button>
          </div>
        </header>

        {state.isLoading ? (
          <div className="config-empty">Reading configuration files...</div>
        ) : state.error ? (
          <div className="config-error">{state.error}</div>
        ) : (
          <div className="config-file-list">
            {normalizedQuery ? (
              <div className="config-search-summary">
                {resultCount > 0
                  ? `${resultCount} matching setting${resultCount === 1 ? "" : "s"}`
                  : "No matching settings"}
              </div>
            ) : null}
            {filteredFiles.map(({ entries, file, showRawPreview }) => (
              <ConfigFilePanel
                entries={entries}
                file={file}
                key={file.path}
                onSaveValue={onSaveValue}
                showRawPreview={showRawPreview}
              />
            ))}
            {state.files.length === 0 ? (
              <div className="config-empty">No configuration files were found.</div>
            ) : null}
            {state.files.length > 0 && filteredFiles.length === 0 ? (
              <div className="config-empty">No configuration settings match this search.</div>
            ) : null}
          </div>
        )}
      </section>
    </div>
  );
}

interface ConfigFilePanelProps {
  entries: ModConfigEntry[];
  file: ModConfigFile;
  onSaveValue(file: ModConfigFile, entry: ModConfigEntry, value: string): Promise<void>;
  showRawPreview?: boolean;
}

function ConfigFilePanel({
  entries,
  file,
  onSaveValue,
  showRawPreview = false
}: ConfigFilePanelProps) {
  return (
    <article className="config-file-panel">
      <div className="config-file-heading">
        <strong>{file.fileName}</strong>
        <span>{file.path}</span>
      </div>

      {file.warning ? <div className="config-warning">{file.warning}</div> : null}

      {entries.length > 0 ? (
        <div className="config-entry-grid">
          {entries.map((entry, index) => (
            <ConfigEntryEditor
              entry={entry}
              file={file}
              key={`${entry.section ?? "root"}-${entry.key}-${index}`}
              onSaveValue={onSaveValue}
            />
          ))}
        </div>
      ) : showRawPreview && file.rawPreview ? (
        <pre className="config-raw-preview">{file.rawPreview}</pre>
      ) : null}
    </article>
  );
}

interface ConfigEntryEditorProps {
  entry: ModConfigEntry;
  file: ModConfigFile;
  onSaveValue(file: ModConfigFile, entry: ModConfigEntry, value: string): Promise<void>;
}

function ConfigEntryEditor({ entry, file, onSaveValue }: ConfigEntryEditorProps) {
  const [nextValue, setNextValue] = useState(entry.value);
  const [isSaving, setIsSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const valueKind = configValueKind(entry);
  const isDirty = nextValue !== entry.value;

  useEffect(() => {
    setNextValue(entry.value);
    setSaveError("");
  }, [entry.value, entry.key, entry.section]);

  async function saveValue() {
    if (!isDirty || isSaving) {
      return;
    }

    setIsSaving(true);
    setSaveError("");
    try {
      await onSaveValue(file, entry, nextValue);
    } catch (caughtError) {
      setSaveError(String(caughtError));
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <div className="config-entry">
      <div className="config-entry-topline">
        <span>{entry.section ?? "General"}</span>
        {entry.valueType ? <small>{entry.valueType}</small> : null}
      </div>
      <strong>{entry.key}</strong>
      {entry.description ? <p>{entry.description}</p> : null}

      <div className="config-editor">
        <label>
          <span>Current</span>
          {valueKind === "boolean" ? (
            <button
              className={`config-toggle ${isTruthyConfigValue(nextValue) ? "on" : ""}`}
              onClick={() => setNextValue(isTruthyConfigValue(nextValue) ? "false" : "true")}
              type="button"
            >
              {isTruthyConfigValue(nextValue) ? "true" : "false"}
            </button>
          ) : valueKind === "number" ? (
            <input
              className="config-input"
              onChange={(event) => setNextValue(event.target.value)}
              type="number"
              value={nextValue}
            />
          ) : nextValue.length > 42 || nextValue.includes(",") ? (
            <textarea
              className="config-textarea"
              onChange={(event) => setNextValue(event.target.value)}
              rows={3}
              value={nextValue}
            />
          ) : (
            <input
              className="config-input"
              onChange={(event) => setNextValue(event.target.value)}
              type="text"
              value={nextValue}
            />
          )}
        </label>

        {entry.defaultValue ? (
          <div className="config-default-row">
            <span>Default</span>
            <code>{entry.defaultValue}</code>
          </div>
        ) : null}

        <div className="config-save-row">
          <button
            className="secondary-button compact-button"
            disabled={!isDirty || isSaving}
            onClick={() => void saveValue()}
            type="button"
          >
            {isSaving ? "Saving" : isDirty ? "Save" : "Saved"}
          </button>
        </div>

        {saveError ? <div className="config-inline-error">{saveError}</div> : null}
      </div>
    </div>
  );
}

function configValueKind(entry: ModConfigEntry): "boolean" | "number" | "text" {
  const type = entry.valueType?.toLowerCase() ?? "";
  const value = entry.value.trim();

  if (type.includes("bool") || /^(true|false)$/i.test(value)) {
    return "boolean";
  }

  if (
    type.includes("int") ||
    type.includes("float") ||
    type.includes("double") ||
    type.includes("single") ||
    type.includes("decimal") ||
    /^-?\d+(\.\d+)?$/.test(value)
  ) {
    return "number";
  }

  return "text";
}

function isTruthyConfigValue(value: string): boolean {
  return value.trim().toLowerCase() === "true";
}

function ModCard({ mod, onConfigure, onEnable, onDisable, onRemove }: ModCardProps) {
  const configFiles = mod.configFiles ?? [];
  const dependencies = mod.dependencies ?? [];
  const modName = displayModName(mod);
  const [showExtraDetails, setShowExtraDetails] = useState(false);

  return (
    <article className={`mod-card ${mod.enabled ? "enabled" : "disabled"}`}>
      <div className="mod-card-header">
        <div className="mod-card-title">
          <strong title={mod.archiveName}>{modName}</strong>
        </div>
        <div className="mod-card-controls">
          <label className="details-toggle" title="Show extra details">
            <input
              aria-label="Show extra details"
              checked={showExtraDetails}
              onChange={(event) => setShowExtraDetails(event.target.checked)}
              type="checkbox"
            />
          </label>
          <small>{mod.enabled ? "Enabled" : "Disabled"}</small>
        </div>
      </div>

      {showExtraDetails ? (
        <div className="mod-details">
          <span>{mod.summary}</span>
          <div className="mod-meta">
            <span>{mod.adapterId}</span>
            <span>{mod.filesWritten.length} file(s)</span>
          </div>
        </div>
      ) : null}

      {dependencies.length > 0 ? (
        <div className="dependency-chips">
          {dependencies.slice(0, 4).map((dependency) => (
            <span key={dependency.id}>{dependency.name}</span>
          ))}
        </div>
      ) : null}

      <div className="mod-actions">
        {configFiles.length > 0 ? (
          <button className="secondary-button compact-button" onClick={onConfigure}>
            <Settings2 size={15} />
            Configure
          </button>
        ) : null}
        {mod.enabled ? (
          <button className="secondary-button compact-button" onClick={onDisable}>
            <PowerOff size={15} />
            Disable
          </button>
        ) : (
          <button className="secondary-button compact-button" onClick={onEnable}>
            <Power size={15} />
            Enable
          </button>
        )}
        <button className="danger-button compact-button" onClick={onRemove}>
          <Trash2 size={15} />
          Remove
        </button>
      </div>
    </article>
  );
}
