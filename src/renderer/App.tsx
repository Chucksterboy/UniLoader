import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Compass,
  Database,
  Download,
  ExternalLink,
  FolderOpen,
  Gamepad2,
  Heart,
  Home,
  Minus,
  PackagePlus,
  Pencil,
  Play,
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
import { getVersion } from "@tauri-apps/api/app";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import {
  useEffect,
  useMemo,
  useRef,
  useState
} from "react";
import {
  AppSettings,
  AppUpdateInfo,
  ArchiveAnalysis,
  DiscoveryPage,
  CreateProfileInput,
  DependencySpec,
  GameDetectionResult,
  GameProfile,
  InstalledModRecord,
  InstallPlan,
  InstallPreflightResult,
  ModConfigEntry,
  ModConfigFile,
  OnlineModFileOption,
  OnlineModRecord,
  ProfileDependencyBootstrapResult,
  ProfileRefreshResult,
  SteamGameRecord
} from "../shared/contracts";
import { desktopApi } from "./tauriApi";

const emptyProfileInput: CreateProfileInput = {
  name: "",
  gamePath: "",
  gameId: undefined,
  steamAppId: undefined,
  engine: "unknown",
  loader: "none"
};

const defaultAppSettings: AppSettings = {
  minimizeToTrayOnClose: false,
  nexusApiKeyConfigured: false
};

const defaultThemeId = "neon-circuit";
const supportPageUrl = "https://ko-fi.com/chucksterboy";

type ViewMode = "manager" | "discover" | "transfer" | "settings";
type ModSortMode = "newest" | "oldest";
type OnlineSortMode = "downloads" | "newest" | "oldest";
type TransferMode = "import" | "export" | null;
type NoticeKind = "success" | "warning" | "error";
type StartupSplashPhase = "intro" | "exiting" | "hidden";

interface PendingDependencyPrompt {
  mod: OnlineModRecord;
  file?: OnlineModFileOption;
  preflight: InstallPreflightResult;
}

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
const startupSplashMaximumMs = 8000;

export function App() {
  const [activeView, setActiveView] = useState<ViewMode>("manager");
  const [startupSplashPhase, setStartupSplashPhase] = useState<StartupSplashPhase>("intro");
  const [startupMinimumElapsed, setStartupMinimumElapsed] = useState(false);
  const [bootstrapReady, setBootstrapReady] = useState(false);
  const [appVersion, setAppVersion] = useState("");
  const [appSettings, setAppSettings] = useState<AppSettings>(defaultAppSettings);
  const [updateInfo, setUpdateInfo] = useState<AppUpdateInfo | null>(null);
  const [profiles, setProfiles] = useState<GameProfile[]>([]);
  const [selectedProfileId, setSelectedProfileId] = useState<string>("");
  const [expandedProfileId, setExpandedProfileId] = useState<string>("");
  const [selectedProfileFolderConnected, setSelectedProfileFolderConnected] = useState<
    boolean | null
  >(null);
  const [profileInput, setProfileInput] = useState<CreateProfileInput>(emptyProfileInput);
  const [analysis, setAnalysis] = useState<ArchiveAnalysis | null>(null);
  const [installedMods, setInstalledMods] = useState<InstalledModRecord[]>([]);
  const [discoverProfileId, setDiscoverProfileId] = useState<string>("");
  const [discoverLoadedProfileId, setDiscoverLoadedProfileId] = useState<string>("");
  const [onlineMods, setOnlineMods] = useState<OnlineModRecord[]>([]);
  const [onlineModTotal, setOnlineModTotal] = useState(0);
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
  const [isScanningSteamGames, setIsScanningSteamGames] = useState(false);
  const [isCreatingSteamProfile, setIsCreatingSteamProfile] = useState(false);
  const [isLaunchingGame, setIsLaunchingGame] = useState(false);
  const [isTogglingAllMods, setIsTogglingAllMods] = useState(false);
  const [steamGames, setSteamGames] = useState<SteamGameRecord[]>([]);
  const [selectedSteamGameAppId, setSelectedSteamGameAppId] = useState("");
  const [profileCreatorOpen, setProfileCreatorOpen] = useState(false);
  const [steamGameSearch, setSteamGameSearch] = useState("");
  const [status, setStatus] = useState<string>("Ready");
  const [notice, setNoticeState] = useState<Notice | null>(null);
  const [nexusSettingsAttentionId, setNexusSettingsAttentionId] = useState(0);
  const [configModal, setConfigModal] = useState<ConfigModalState | null>(null);
  const [profilePendingRename, setProfilePendingRename] = useState<GameProfile | null>(null);
  const [profilePendingRemoval, setProfilePendingRemoval] = useState<GameProfile | null>(null);
  const [pendingDependencyPrompt, setPendingDependencyPrompt] =
    useState<PendingDependencyPrompt | null>(null);
  const [error, setErrorState] = useState<string>("");
  const [errorMotionId, setErrorMotionId] = useState(0);
  const noticeSequence = useRef(0);
  const errorSequence = useRef(0);
  const updateAnnouncementShown = useRef(false);
  const installedModsRequestSequence = useRef(0);
  const discoveryRequestSequence = useRef(0);
  const processedNxmLinks = useRef(new Set<string>());

  function setNotice(nextNotice: NoticeInput | null) {
    setNoticeState(nextNotice ? { ...nextNotice, motionId: ++noticeSequence.current } : null);
  }

  useEffect(() => {
    if (selectedProfileId) {
      setExpandedProfileId(selectedProfileId);
    }
  }, [selectedProfileId]);

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
  const dependencyPromptPresence = useFadePresence(pendingDependencyPrompt);
  const profileCreatorPresence = useFadePresence(profileCreatorOpen ? true : null);
  const noticePresence = useFadePresence(notice, 140);
  const errorPresence = useFadePresence(error ? error : null, 140);
  const selectedProfile = useMemo(
    () => profiles.find((profile) => profile.id === selectedProfileId),
    [profiles, selectedProfileId]
  );
  const discoveryProfiles = useMemo(
    () => profiles.filter((profile) => Boolean(profile.steamAppId)),
    [profiles]
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
  const selectedSteamGame = useMemo(
    () => steamGames.find((game) => game.appId === selectedSteamGameAppId),
    [selectedSteamGameAppId, steamGames]
  );
  const filteredSteamGames = useMemo(() => {
    const query = steamGameSearch.trim().toLowerCase();
    if (!query) {
      return steamGames;
    }
    return steamGames.filter((game) =>
      `${game.name} ${game.installDir}`.toLowerCase().includes(query)
    );
  }, [steamGameSearch, steamGames]);
  const enabledProfileModCount = installedMods.filter((mod) => mod.enabled).length;
  const allProfileModsEnabled =
    installedMods.length > 0 && enabledProfileModCount === installedMods.length;
  const profileModToggleLabel = allProfileModsEnabled ? "ON" : "OFF";
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
    void getVersion().then(setAppVersion).catch(() => setAppVersion("unknown"));
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const receiveUrls = (urls: string[]) => {
      if (!disposed) {
        void handleNxmUrls(urls);
      }
    };

    void onOpenUrl(receiveUrls)
      .then((handler) => {
        if (disposed) {
          handler();
        } else {
          unlisten = handler;
        }
      })
      .catch((caughtError) => {
        if (!disposed) {
          setNotice({
            kind: "warning",
            title: "Nexus handoff unavailable",
            detail: String(caughtError)
          });
        }
      });

    void getCurrent()
      .then((urls) => {
        if (urls) {
          receiveUrls(urls);
        }
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    void checkForUpdates(false);
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => setStartupMinimumElapsed(true), startupSplashPulseMs);
    return () => window.clearTimeout(timeoutId);
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => setBootstrapReady(true), startupSplashMaximumMs);
    return () => window.clearTimeout(timeoutId);
  }, []);

  useEffect(() => {
    if (!startupMinimumElapsed || !bootstrapReady) {
      return;
    }
    setStartupSplashPhase("exiting");
    const hideTimeoutId = window.setTimeout(
      () => setStartupSplashPhase("hidden"),
      startupSplashFadeMs
    );
    return () => window.clearTimeout(hideTimeoutId);
  }, [bootstrapReady, startupMinimumElapsed]);

  useEffect(() => {
    if (selectedProfileId) {
      void refreshInstalledMods(selectedProfileId);
    }
  }, [selectedProfileId]);

  useEffect(() => {
    if (!selectedProfile) {
      setSelectedProfileFolderConnected(null);
      return;
    }

    let cancelled = false;
    setSelectedProfileFolderConnected(null);
    void api
      .profileFolderExists(selectedProfile.id)
      .then((connected) => {
        if (!cancelled) {
          setSelectedProfileFolderConnected(connected);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setSelectedProfileFolderConnected(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [selectedProfile?.gamePath, selectedProfile?.id]);

  useEffect(() => {
    if (activeView !== "discover") {
      return;
    }

    setDiscoverProfileId((current) =>
      current && discoveryProfiles.some((profile) => profile.id === current)
        ? current
        : selectedProfile?.steamAppId
          ? selectedProfileId
          : discoveryProfiles[0]?.id || ""
    );
  }, [
    activeView,
    discoveryProfiles,
    selectedProfile?.steamAppId,
    selectedProfileId
  ]);

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
          detail: "Use Choose File to import a local mod."
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
    } finally {
      setBootstrapReady(true);
    }
  }

  async function refreshInstalledMods(profileId: string) {
    const requestId = ++installedModsRequestSequence.current;
    const mods = await api.listInstalledMods(profileId);
    if (requestId === installedModsRequestSequence.current) {
      setInstalledMods(mods);
    }
  }

  async function handleNxmUrls(urls: string[]) {
    for (const nxmUrl of urls) {
      if (!nxmUrl.toLowerCase().startsWith("nxm://")) {
        continue;
      }
      const replayKey = nexusNxmReplayKey(nxmUrl);
      if (!replayKey || processedNxmLinks.current.has(replayKey)) {
        continue;
      }
      processedNxmLinks.current.add(replayKey);
      setError("");
      setInstallingOnlineModId("nexus-handoff");
      setStatus("Installing Nexus download");
      setNotice({
        kind: "warning",
        title: "Nexus download authorized",
        detail: "UniLoader received the selected file and is checking it before installation."
      });

      try {
        const result = await api.installNexusNxmLink(nxmUrl);
        setSelectedProfileId(result.installResult.profileId);
        setExpandedProfileId(result.installResult.profileId);
        setOnlineMods((current) =>
          current.map((item) =>
            item.id === result.modId ? { ...item, installed: true } : item
          )
        );
        await refreshInstalledMods(result.installResult.profileId);
        setStatus("Mod installed");
        setNotice({
          kind: result.installResult.warnings.length > 0 ? "warning" : "success",
          title: "Nexus mod installed",
          detail: installSuccessDetail(
            "Selected Nexus file",
            result.installResult.filesWritten.length,
            result.installResult.warnings
          )
        });
      } catch (caughtError) {
        processedNxmLinks.current.delete(replayKey);
        setError(String(caughtError));
        setStatus("Nexus install failed");
        setNotice({
          kind: "error",
          title: "Nexus install failed",
          detail: String(caughtError)
        });
      } finally {
        setInstallingOnlineModId("");
      }
    }
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

  async function saveNexusApiKey(apiKey: string) {
    try {
      const savedSettings = await api.saveNexusApiKey(apiKey);
      setAppSettings(savedSettings);
      setOnlineMods((current) =>
        current.map((mod) =>
          mod.provider === "nexus"
            ? {
                ...mod,
                installSupported: savedSettings.nexusApiKeyConfigured,
                installNote: savedSettings.nexusApiKeyConfigured
                  ? "Uses your saved Nexus API key to request the mod file download."
                  : "Add a Nexus API key in Settings to enable direct install for Nexus mods."
              }
            : mod
        )
      );
      setStatus(apiKey.trim() ? "Nexus API key saved securely" : "Nexus API key removed");
      setNotice({
        kind: "success",
        title: apiKey.trim() ? "Nexus connected" : "Nexus disconnected",
        detail: apiKey.trim()
          ? "The API key is stored in Windows Credential Manager."
          : "The saved Nexus API key was removed."
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setNotice({
        kind: "error",
        title: "Nexus settings failed",
        detail: String(caughtError)
      });
      throw caughtError;
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
        currentVersion: appVersion || "unknown",
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

  function openProfileCreator() {
    setProfileInput(emptyProfileInput);
    setDetection(null);
    setSelectedSteamGameAppId("");
    setSteamGameSearch("");
    setProfileCreatorOpen(true);
    if (steamGames.length === 0 && !isScanningSteamGames) {
      void scanSteamGames();
    }
  }

  function closeProfileCreator() {
    if (isCreatingSteamProfile) {
      return;
    }
    setProfileCreatorOpen(false);
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

  async function scanSteamGames() {
    setError("");
    setIsScanningSteamGames(true);
    setStatus("Scanning Steam games");
    try {
      const games = await api.scanSteamGames();
      setSteamGames(games);
      setSelectedSteamGameAppId("");
      setStatus(games.length > 0 ? "Steam games found" : "No Steam games found");
      setNotice({
        kind: games.length > 0 ? "success" : "warning",
        title: games.length > 0 ? "Steam games found" : "No Steam games found",
        detail:
          games.length > 0
            ? `Found ${games.length} installed Steam game${games.length === 1 ? "" : "s"}. Pick one below to create a profile.`
            : "UniLoader could not find Steam library manifests on this PC."
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Steam scan failed");
      setNotice({
        kind: "error",
        title: "Steam scan failed",
        detail: String(caughtError)
      });
    } finally {
      setIsScanningSteamGames(false);
    }
  }

  async function selectSteamGameForProfile(appId: string) {
    setSelectedSteamGameAppId(appId);
    const game = steamGames.find((item) => item.appId === appId);
    if (!game) {
      return;
    }

    setProfileInput((current) => ({
      ...current,
      name: game.name,
      gamePath: game.installDir,
      steamAppId: game.appId
    }));
    setDetection(null);
    setIsDetecting(true);
    setStatus("Detecting Steam game");
    setError("");

    try {
      const detectedSetup = await api.detectGameSetup(game.installDir);
      setDetection(detectedSetup);
      setProfileInput((current) => ({
        ...current,
        gameId: detectedSetup.gameId,
        engine: detectedSetup.engine,
        loader: detectedSetup.loader
      }));
      const issue = getDetectionIssue(detectedSetup);
      setStatus(issue ? "Detection needs review" : "Steam game ready");
      setNotice(
        issue
          ? {
              kind: "warning",
              title: "Steam game needs review",
              detail: issue
            }
          : {
              kind: "success",
              title: "Steam game ready",
              detail: `${game.name} folder was selected automatically.`
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

  async function createSelectedSteamProfile() {
    if (!selectedSteamGame) {
      setNotice({
        kind: "warning",
        title: "Choose a Steam game",
        detail: "Scan Steam games, then choose one from the dropdown."
      });
      return;
    }

    await createProfileFromSteam(selectedSteamGame);
  }

  async function createProfileFromSteam(game: SteamGameRecord) {
    const existingProfile = profiles.find(
      (profile) =>
        profile.steamAppId === game.appId ||
        profile.gamePath.toLowerCase() === game.installDir.toLowerCase()
    );

    if (existingProfile) {
      setSelectedProfileId(existingProfile.id);
      setProfileCreatorOpen(false);
      setNotice({
        kind: "warning",
        title: "Profile already exists",
        detail: `${existingProfile.name} is already in UniLoader.`
      });
      return;
    }

    setError("");
    setIsCreatingSteamProfile(true);
    setStatus("Creating Steam profile");
    try {
      const profile = await api.createSteamProfile(game);
      setProfiles((current) => [...current, profile]);
      setSelectedProfileId(profile.id);
      setAnalysis(null);
      setDetection(null);
      setSelectedSteamGameAppId("");
      setProfileInput(emptyProfileInput);
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

      setStatus("Steam profile created");
      setNotice({
        kind:
          profile.engine === "unknown" || profile.loader === "none" || dependencyWarnings.length > 0
            ? "warning"
            : "success",
        title: "Steam profile created",
        detail:
          profile.engine === "unknown" || profile.loader === "none"
            ? `${profile.name} was added from Steam, but detection may need review before installing mods.`
            : `${profile.name} was added with Steam launch support.${dependencyDetail}${
                dependencyWarnings.length > 0 ? ` ${dependencyWarnings[0]}` : ""
              }`
      });
      setProfileCreatorOpen(false);
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Steam profile failed");
      setNotice({
        kind: "error",
        title: "Steam profile failed",
        detail: String(caughtError)
      });
    } finally {
      setIsCreatingSteamProfile(false);
    }
  }

  async function launchSelectedProfileGame() {
    if (!selectedProfile) {
      setNotice({
        kind: "warning",
        title: "Select a profile",
        detail: "Choose a profile before launching the game."
      });
      return;
    }

    setError("");
    setIsLaunchingGame(true);
    setStatus("Launching game");
    try {
      await api.launchProfileGame(selectedProfile.id);
      setStatus("Game launched");
      setNotice({
        kind: "success",
        title: "Launch requested",
        detail: `Steam is launching ${selectedProfile.name}.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Launch failed");
      setNotice({
        kind: "error",
        title: "Launch failed",
        detail: String(caughtError)
      });
    } finally {
      setIsLaunchingGame(false);
    }
  }

  async function setAllModsEnabled(enabled: boolean) {
    if (!selectedProfile) {
      setNotice({
        kind: "warning",
        title: "Select a profile",
        detail: "Choose a profile before changing installed mods."
      });
      return;
    }

    if (installedMods.length === 0) {
      setNotice({
        kind: "warning",
        title: "No mods installed",
        detail: "Install at least one mod before using the all-mods switch."
      });
      return;
    }

    setError("");
    setIsTogglingAllMods(true);
    setStatus(enabled ? "Enabling all mods" : "Disabling all mods");
    try {
      const result = await api.setAllProfileModsEnabled(selectedProfile.id, enabled);
      setInstalledMods(result.installedMods);
      setStatus(enabled ? "All mods enabled" : "All mods disabled");
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: enabled ? "All mods enabled" : "All mods disabled",
        detail:
          result.warnings[0] ??
          `${result.changedMods} mod${result.changedMods === 1 ? "" : "s"} ${enabled ? "enabled" : "disabled"}.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Mod toggle failed");
      setNotice({
        kind: "error",
        title: "Mod toggle failed",
        detail: String(caughtError)
      });
    } finally {
      setIsTogglingAllMods(false);
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

  async function loadOnlineMods(
    profileId: string,
    page = 1,
    sort: OnlineSortMode = "downloads",
    query = ""
  ) {
    const requestId = ++discoveryRequestSequence.current;
    setError("");
    setIsDiscoveringMods(true);
    setStatus("Discovering online mods");
    try {
      const result: DiscoveryPage = await api.discoverOnlineMods(
        profileId,
        page,
        discoverPageSize,
        sort,
        query
      );
      if (requestId !== discoveryRequestSequence.current) {
        return;
      }
      setOnlineMods(result.items);
      setOnlineModTotal(result.total);
      setDiscoverLoadedProfileId(profileId);
      setStatus("Discovery ready");
      const profile = profiles.find((item) => item.id === profileId);
      setNotice({
        kind: "success",
        title: "Discovery updated",
        detail: `${profile?.name ?? "Selected profile"}: found ${result.total} online mod(s).`
      });
    } catch (caughtError) {
      if (requestId !== discoveryRequestSequence.current) {
        return;
      }
      setOnlineMods([]);
      setOnlineModTotal(0);
      setError(String(caughtError));
      setStatus("Discovery failed");
      setNotice({
        kind: "error",
        title: "Discovery failed",
        detail: String(caughtError)
      });
    } finally {
      if (requestId === discoveryRequestSequence.current) {
        setIsDiscoveringMods(false);
      }
    }
  }

  async function loadOnlineModFiles(mod: OnlineModRecord) {
    if (!discoverProfileId) {
      throw new Error("Select a profile before loading mod files.");
    }
    return api.listDiscoveredModFiles(discoverProfileId, mod);
  }

  async function installOnlineMod(
    mod: OnlineModRecord,
    file?: OnlineModFileOption,
    skipDependencyPrompt = false
  ) {
    if (!discoverProfileId) {
      setNotice({
        kind: "warning",
        title: "Select a profile",
        detail: "Choose the profile you want to install this mod into."
      });
      return;
    }

    if (mod.provider === "nexus" && !skipDependencyPrompt) {
      try {
        const preflight = await api.preflightDiscoveredModInstall(discoverProfileId, mod);
        if (preflight.confirmationRequired) {
          setPendingDependencyPrompt({ mod, file, preflight });
          setStatus("Dependency confirmation needed");
          return;
        }
      } catch (caughtError) {
        setError(String(caughtError));
        setStatus("Dependency check failed");
        setNotice({
          kind: "error",
          title: "Could not verify requirements",
          detail: String(caughtError)
        });
        return;
      }
    }

    if (file?.action === "browser") {
      try {
        const downloadPageUrl = await api.beginNexusBrowserDownload(
          discoverProfileId,
          mod,
          file
        );
        await openExternalUrl(downloadPageUrl);
        setStatus("Waiting for Nexus confirmation");
        setNotice({
          kind: "warning",
          title: "Continue in Nexus",
          detail: "Click Slow download, then Open via UniLoader for automated installs."
        });
      } catch (caughtError) {
        setError(String(caughtError));
        setStatus("Nexus handoff failed");
        setNotice({
          kind: "error",
          title: "Could not start Nexus download",
          detail: String(caughtError)
        });
      }
      return;
    }

    if (file?.action === "auth") {
      openNexusAuthSettings();
      return;
    }

    if (!mod.installSupported || file?.action === "unsupported") {
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
      const result = await api.installDiscoveredMod(discoverProfileId, mod, file);
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

  async function confirmDependencyInstall(prompt: PendingDependencyPrompt) {
    const missingNexusDependency = prompt.preflight.missingDependencies.find(
      (dependency) => dependency.provider === "nexus"
    );
    const missingManualDependency = prompt.preflight.missingDependencies.find(
      (dependency) => dependency.provider === "manual"
    );
    setPendingDependencyPrompt(null);

    if (missingManualDependency?.source) {
      await openExternalUrl(missingManualDependency.source);
      setStatus("External requirement opened");
      setNotice({
        kind: "warning",
        title: "Install the required component",
        detail: `${missingManualDependency.name} is hosted outside Nexus. Install it, then retry the mod.`
      });
      return;
    }

    if (prompt.file?.action === "browser" && missingNexusDependency) {
      try {
        const downloadPageUrl = await api.beginNexusRequirementDownload(
          discoverProfileId,
          missingNexusDependency.id
        );
        await openExternalUrl(downloadPageUrl);
        setStatus("Waiting for Nexus requirement");
        setNotice({
          kind: "warning",
          title: `Required: ${missingNexusDependency.name}`,
          detail: "Click Slow download, then Open via UniLoader. Retry the original mod after this requirement installs."
        });
      } catch (caughtError) {
        setError(String(caughtError));
        setStatus("Requirement handoff failed");
        setNotice({
          kind: "error",
          title: "Could not start requirement download",
          detail: String(caughtError)
        });
      }
      return;
    }

    await installOnlineMod(prompt.mod, prompt.file, true);
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

  async function openSupportPage() {
    if (!supportPageUrl) {
      setNotice({
        kind: "warning",
        title: "Support Me",
        detail: "The Patreon page is coming soon."
      });
      return;
    }
    await openExternalUrl(supportPageUrl);
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
        title: nextAnalysis.compatibility.status === "blocked" ? "Incompatible mod" : "No install route found",
        detail: nextAnalysis.compatibility.reason
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
        packageIdentity: nextAnalysis.packageIdentity,
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

  const profileToRename = profileRenamePresence.value;
  const profileToRemove = profileRemovalPresence.value;
  const dependencyPrompt = dependencyPromptPresence.value;

  return (
    <>
    {startupSplashPhase !== "hidden" ? <StartupSplash phase={startupSplashPhase} /> : null}
    <main
      className={renderedView === "manager" ? "app-shell" : "app-shell settings-shell"}
      data-theme={defaultThemeId}
    >
      <header className="window-title-strip" data-tauri-drag-region>
        <span className="window-title-identity">
          <span className="window-title-mark">
            <UniLoaderMark />
          </span>
          <strong>UniLoader</strong>
        </span>
        <span className="window-title-trace" />
      </header>
      <WindowControls
        onClose={() => void api.closeWindow()}
        onMaximize={() => void api.toggleMaximizeWindow()}
        onMinimize={() => void api.minimizeWindow()}
      />
      <aside className="nav-rail">
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
        </nav>
        <div className="rail-footer">
          <UpdateRailIndicator
            isChecking={isCheckingForUpdate}
            isDownloading={isDownloadingUpdate}
            updateInfo={updateInfo}
            onClick={() => void showUpdateDetails()}
          />
          <button
            aria-label="Support Me"
            className="rail-support-button"
            data-tooltip="Support Me"
            onClick={() => void openSupportPage()}
            type="button"
          >
            <Heart size={17} />
          </button>
          <div className="rail-version-row">
            <div className="rail-status" title={status} />
            <span className="app-version" title={`UniLoader ${appVersion || "loading"}`}>
              {appVersion ? `v${appVersion}` : ""}
            </span>
          </div>
        </div>
      </aside>
      {renderedView === "manager" ? (
      <aside className={`sidebar view-motion ${viewMotion.className}`}>
        <section className="panel">
          <div className="panel-heading">
            <Database size={17} />
            <h2>Profiles</h2>
          </div>
          <div className="profile-list">
            {profiles.map((profile) => {
              const isSelected = profile.id === selectedProfileId;
              const isExpanded = profile.id === expandedProfileId;
              const profileStatus = profile.engine === "unknown"
                ? "Game not identified"
                : profile.loader === "none" && profile.engine !== "unreal"
                  ? "Loader not detected"
                  : "Ready";
              return (
                <div
                  className={`profile${isSelected ? " active" : ""}${isExpanded ? " expanded" : ""}`}
                  key={profile.id}
                >
                  <button
                    aria-expanded={isExpanded}
                    className="profile-select"
                    onClick={() => {
                      if (isSelected) {
                        setExpandedProfileId((current) => (current === profile.id ? "" : profile.id));
                      } else {
                        setSelectedProfileId(profile.id);
                        setExpandedProfileId(profile.id);
                      }
                      setAnalysis(null);
                      setNotice(null);
                    }}
                    type="button"
                  >
                    <ProfileArtwork profile={profile} />
                    <span className="profile-copy">
                      <strong>{profile.name}</strong>
                      <small>{profileStatus}</small>
                    </span>
                    <span className="profile-chevron" aria-hidden="true">
                      {isExpanded ? <ChevronUp size={18} /> : <ChevronDown size={18} />}
                    </span>
                  </button>
                  {isExpanded ? (
                    <div className="profile-expanded-content">
                      <div className="profile-actions">
                        <button
                          onClick={() => setProfilePendingRename(profile)}
                          type="button"
                        >
                          <Pencil size={15} />
                          <span>Rename</span>
                        </button>
                        <button
                          className="folder"
                          onClick={() => void openProfileGameFolder(profile)}
                          type="button"
                        >
                          <FolderOpen size={15} />
                          <span>Open Folder</span>
                        </button>
                        <button
                          className="remove"
                          onClick={() => setProfilePendingRemoval(profile)}
                          type="button"
                        >
                          <Trash2 size={15} />
                          <span>Remove</span>
                        </button>
                      </div>
                      <div
                        className={`profile-folder-state ${
                          selectedProfileFolderConnected === true
                            ? "connected"
                            : selectedProfileFolderConnected === false
                              ? "disconnected"
                              : "checking"
                        }`}
                      >
                        {selectedProfileFolderConnected === true ? (
                          <CheckCircle2 size={16} />
                        ) : selectedProfileFolderConnected === false ? (
                          <AlertTriangle size={16} />
                        ) : (
                          <RefreshCw size={16} className="spin-icon" />
                        )}
                        <span>
                          {selectedProfileFolderConnected === true
                            ? "Game folder connected"
                            : selectedProfileFolderConnected === false
                              ? "Game folder unavailable"
                              : "Checking game folder"}
                        </span>
                      </div>
                    </div>
                  ) : null}
                </div>
              );
            })}
            {profiles.length === 0 ? <p className="muted">No profiles yet.</p> : null}
          </div>
          <button className="new-profile-trigger" onClick={openProfileCreator} type="button">
            <PackagePlus size={17} />
            <span>New Profile</span>
          </button>
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
        <header className="topbar">
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

        <section className="profile-command-panel">
          <div className="profile-command-copy">
            <p className="eyebrow">Profile controls</p>
            <h3>Launch & mod state</h3>
            <small>
              {selectedProfile?.steamAppId
                ? `Steam App ${selectedProfile.steamAppId}`
                : "Steam launch is available for profiles added from Steam scan."}
            </small>
          </div>
          <div className="profile-command-actions">
            <button
              className="primary-button"
              disabled={!selectedProfile || !selectedProfile.steamAppId || isLaunchingGame}
              onClick={() => void launchSelectedProfileGame()}
              title={
                selectedProfile?.steamAppId
                  ? "Launch selected game through Steam"
                  : "Add this profile from Steam scan to enable Steam launch"
              }
              type="button"
            >
              <Play size={17} />
              {isLaunchingGame ? "Launching" : "Launch Game"}
            </button>
            <label
              className={isTogglingAllMods ? "master-mod-toggle busy" : "master-mod-toggle"}
              title="Enable or disable every installed mod in this profile"
            >
              <input
                checked={allProfileModsEnabled}
                disabled={!selectedProfile || installedMods.length === 0 || isTogglingAllMods}
                onChange={(event) => void setAllModsEnabled(event.currentTarget.checked)}
                type="checkbox"
              />
              <span className="master-mod-track">
                <span />
              </span>
              <span className="master-mod-copy">
                <strong>{profileModToggleLabel}</strong>
              </span>
            </label>
          </div>
        </section>

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
            profiles={discoveryProfiles}
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
            total={discoverLoadedProfileId === discoverProfileId ? onlineModTotal : 0}
            profiles={profiles}
            selectedProfileId={discoverProfileId}
            onInstall={(mod, file) => void installOnlineMod(mod, file)}
            onLoadFiles={loadOnlineModFiles}
            onNeedsAuth={openNexusAuthSettings}
            onOpenPage={(url) => void openExternalUrl(url)}
            onLoad={(page, sort, query) => void loadOnlineMods(discoverProfileId, page, sort, query)}
            onSelectProfile={setDiscoverProfileId}
          />
        ) : (
          <SettingsView
            appSettings={appSettings}
            nexusAttentionId={nexusSettingsAttentionId}
            onOpenExternalUrl={(url) => void openExternalUrl(url)}
            onSaveNexusApiKey={saveNexusApiKey}
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
    {profileCreatorPresence.value ? (
      <ProfileCreatorDialog
        detection={detection}
        detectionIssue={detectionIssue}
        filteredSteamGames={filteredSteamGames}
        isCreatingSteamProfile={isCreatingSteamProfile}
        isDetecting={isDetecting}
        isScanningSteamGames={isScanningSteamGames}
        motionClassName={profileCreatorPresence.className}
        profileInput={profileInput}
        selectedSteamGameAppId={selectedSteamGameAppId}
        steamGameSearch={steamGameSearch}
        steamGamesCount={steamGames.length}
        onCancel={closeProfileCreator}
        onChangeSteamSearch={setSteamGameSearch}
        onCreateSteam={() => void createSelectedSteamProfile()}
        onScanSteam={() => void scanSteamGames()}
        onSelectSteamGame={(appId) => void selectSteamGameForProfile(appId)}
      />
    ) : null}
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
    {dependencyPrompt ? (
      <DependencyInstallDialog
        missingDependencies={dependencyPrompt.preflight.missingDependencies}
        modName={dependencyPrompt.mod.name}
        motionClassName={dependencyPromptPresence.className}
        onCancel={() => setPendingDependencyPrompt(null)}
        onConfirm={() => void confirmDependencyInstall(dependencyPrompt)}
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

interface ProfileCreatorDialogProps {
  detection: GameDetectionResult | null;
  detectionIssue: string;
  filteredSteamGames: SteamGameRecord[];
  isCreatingSteamProfile: boolean;
  isDetecting: boolean;
  isScanningSteamGames: boolean;
  motionClassName: string;
  profileInput: CreateProfileInput;
  selectedSteamGameAppId: string;
  steamGameSearch: string;
  steamGamesCount: number;
  onCancel(): void;
  onChangeSteamSearch(value: string): void;
  onCreateSteam(): void;
  onScanSteam(): void;
  onSelectSteamGame(appId: string): void;
}

function ProfileCreatorDialog({
  detection,
  detectionIssue,
  filteredSteamGames,
  isCreatingSteamProfile,
  isDetecting,
  isScanningSteamGames,
  motionClassName,
  profileInput,
  selectedSteamGameAppId,
  steamGameSearch,
  steamGamesCount,
  onCancel,
  onChangeSteamSearch,
  onCreateSteam,
  onScanSteam,
  onSelectSteamGame
}: ProfileCreatorDialogProps) {
  const isBusy = isCreatingSteamProfile;
  const canCreateSteam = selectedSteamGameAppId.length > 0 && !isDetecting;

  return (
    <div className={`modal-backdrop ${motionClassName}`} onMouseDown={onCancel}>
      <form
        aria-label="Create game profile"
        aria-modal="true"
        className="profile-creator-modal steam-only"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          if (canCreateSteam) {
            onCreateSteam();
          }
        }}
        role="dialog"
      >
        <header className="profile-creator-header">
          <div className="profile-creator-title">
            <span className="profile-creator-icon">
              <PackagePlus size={22} />
            </span>
            <div>
              <p className="eyebrow">Profiles</p>
              <h3>New Profile</h3>
            </div>
          </div>
          <button
            aria-label="Close new profile window"
            className="icon-button"
            disabled={isBusy}
            onClick={onCancel}
            title="Close"
            type="button"
          >
            <X size={18} />
          </button>
        </header>

        <div className="profile-creator-content">
          <div className="steam-creator-flow">
            <div className="steam-creator-toolbar">
              <label className="search-field">
                <Search size={16} />
                <input
                  autoFocus
                  onChange={(event) => onChangeSteamSearch(event.target.value)}
                  placeholder="Search installed Steam games"
                  value={steamGameSearch}
                />
              </label>
              <button
                aria-label="Scan Steam games"
                className="icon-button steam-scan-button"
                disabled={isScanningSteamGames}
                onClick={onScanSteam}
                title="Scan Steam games"
                type="button"
              >
                <RefreshCw className={isScanningSteamGames ? "spin-icon" : ""} size={17} />
              </button>
            </div>

            <div className="steam-creator-list" aria-label="Installed Steam games">
              {isScanningSteamGames && steamGamesCount === 0 ? (
                <div className="profile-creator-empty">Scanning Steam libraries...</div>
              ) : null}
              {!isScanningSteamGames && steamGamesCount === 0 ? (
                <div className="profile-creator-empty">No installed Steam games were found.</div>
              ) : null}
              {steamGamesCount > 0 && filteredSteamGames.length === 0 ? (
                <div className="profile-creator-empty">No installed games match this search.</div>
              ) : null}
              {filteredSteamGames.map((game) => (
                <button
                  aria-pressed={selectedSteamGameAppId === game.appId}
                  className={
                    selectedSteamGameAppId === game.appId
                      ? "steam-creator-game selected"
                      : "steam-creator-game"
                  }
                  key={game.appId}
                  onClick={() => onSelectSteamGame(game.appId)}
                  type="button"
                >
                  <span>
                    <strong>{game.name}</strong>
                    <small title={game.installDir}>{game.installDir}</small>
                  </span>
                  <CheckCircle2 size={17} />
                </button>
              ))}
            </div>
          </div>
        </div>

        <section className="profile-detection-preview" aria-label="Profile detection preview">
          <div className="profile-detection-copy">
            <span>Selected game</span>
            <strong>{profileInput.name || "Waiting for selection"}</strong>
            <small title={profileInput.gamePath}>
              {profileInput.gamePath || "Choose an installed Steam game."}
            </small>
          </div>
          <DetectionWarning
            detection={detection}
            detectionIssue={detectionIssue}
            isDetecting={isDetecting}
          />
        </section>

        <footer className="profile-creator-actions">
          <button className="secondary-button compact-button" onClick={onCancel} type="button">
            Cancel
          </button>
          <button
            className="primary-button compact-button"
            disabled={isBusy || !canCreateSteam}
            type="submit"
          >
            <PackagePlus size={16} />
            {isBusy ? "Creating..." : "Create Profile"}
          </button>
        </footer>
      </form>
    </div>
  );
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

interface DependencyInstallDialogProps {
  missingDependencies: DependencySpec[];
  modName: string;
  motionClassName: string;
  onCancel(): void;
  onConfirm(): void;
}

function DependencyInstallDialog({
  missingDependencies,
  modName,
  motionClassName,
  onCancel,
  onConfirm
}: DependencyInstallDialogProps) {
  return (
    <div className={`modal-backdrop ${motionClassName}`} onMouseDown={onCancel}>
      <section
        aria-label={`Install requirements for ${modName}`}
        className="confirm-modal dependency-confirm-modal"
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <div className="confirm-icon dependency-confirm-icon">
          <PackagePlus size={22} />
        </div>
        <div className="confirm-copy">
          <p className="eyebrow">Required Components</p>
          <h3>{modName}</h3>
          <p>UniLoader checked this profile first. These required components are still missing:</p>
          <div className="dependency-confirm-list">
            {missingDependencies.map((dependency) => (
              <div className="dependency-confirm-item" key={dependency.id}>
                <strong>{dependency.name}</strong>
                <span>{dependency.provider === "nexus" ? "Nexus Mods" : "External source"}</span>
              </div>
            ))}
          </div>
        </div>
        <div className="confirm-actions">
          <button className="secondary-button compact-button" onClick={onCancel} type="button">
            Cancel
          </button>
          <button className="primary-button compact-button" onClick={onConfirm} type="button">
            Install Requirements
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

function ProfileArtwork({ profile }: { profile: GameProfile }) {
  const [imageSourceIndex, setImageSourceIndex] = useState(0);
  const steamAppId = profile.steamAppId?.trim();
  const imageUrls =
    steamAppId && /^\d+$/.test(steamAppId)
      ? [
          `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900_2x.jpg`,
          `https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900_2x.jpg`,
          `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900.jpg`,
          `https://cdn.cloudflare.steamstatic.com/steam/apps/${steamAppId}/capsule_231x87.jpg`
        ]
      : [];
  const imageUrl = imageUrls[imageSourceIndex];
  const initials =
    profile.name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0]?.toUpperCase())
      .join("") || "G";

  useEffect(() => {
    setImageSourceIndex(0);
  }, [steamAppId]);

  return (
    <span className="profile-artwork" aria-hidden="true">
      {imageUrl ? (
        <img
          alt=""
          draggable={false}
          loading="lazy"
          onError={() => setImageSourceIndex((current) => current + 1)}
          src={imageUrl}
        />
      ) : (
        <span className="profile-artwork-fallback">
          <Gamepad2 size={19} />
          <b>{initials}</b>
        </span>
      )}
    </span>
  );
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
      <span className="health-status" title={status}>{status}</span>
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
  total: number;
  profiles: GameProfile[];
  selectedProfileId: string;
  onInstall(mod: OnlineModRecord, file?: OnlineModFileOption): void;
  onLoadFiles(mod: OnlineModRecord): Promise<OnlineModFileOption[]>;
  onNeedsAuth(): void;
  onOpenPage(url: string): void;
  onLoad(page: number, sort: OnlineSortMode, query: string): void;
  onSelectProfile(profileId: string): void;
}

function DiscoverView({
  hasLoaded,
  installingModId,
  isLoading,
  mods,
  total,
  profiles,
  selectedProfileId,
  onInstall,
  onLoadFiles,
  onNeedsAuth,
  onOpenPage,
  onLoad,
  onSelectProfile
}: DiscoverViewProps) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(1);
  const [sortMode, setSortMode] = useState<OnlineSortMode>("downloads");
  const [expandedModId, setExpandedModId] = useState("");
  const selectedProfile = profiles.find((profile) => profile.id === selectedProfileId);
  const pageCount = Math.max(1, Math.ceil(total / discoverPageSize));
  const currentPage = Math.min(page, pageCount);
  const visibleMods = mods;

  useEffect(() => {
    setPage(1);
    setQuery("");
    setSortMode("downloads");
    setExpandedModId("");
  }, [selectedProfileId]);

  useEffect(() => {
    if (expandedModId && !mods.some((mod) => mod.id === expandedModId)) {
      setExpandedModId("");
    }
  }, [expandedModId, mods]);

  useEffect(() => {
    if (!hasLoaded || !selectedProfileId) {
      return;
    }
    const timeout = window.setTimeout(() => {
      setPage(1);
      onLoad(1, sortMode, query.trim());
    }, 300);
    return () => window.clearTimeout(timeout);
  }, [query]);

  function changeSort(nextSort: OnlineSortMode) {
    setSortMode(nextSort);
    setPage(1);
    setExpandedModId("");
    onLoad(1, nextSort, query.trim());
  }

  function changePage(nextPage: number) {
    const safePage = Math.max(1, Math.min(pageCount, nextPage));
    setPage(safePage);
    setExpandedModId("");
    onLoad(safePage, sortMode, query.trim());
  }

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
          onClick={() => {
            setExpandedModId("");
            onLoad(currentPage, sortMode, query.trim());
          }}
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
            onChange={(event) => {
              setExpandedModId("");
              onSelectProfile(event.target.value);
            }}
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
            onChange={(event) => {
              setExpandedModId("");
              setQuery(event.target.value);
            }}
            placeholder="Search online mods"
          />
          {query ? (
            <button
              onClick={() => {
                setExpandedModId("");
                setQuery("");
              }}
              title="Clear search"
              type="button"
            >
              X
            </button>
          ) : null}
        </label>
        <div className="discover-provider-pill" title={`${total} total online mods`}>
          <Compass size={17} />
          <span>Total Mods</span>
          <strong>{formatCompactNumber(total)}</strong>
        </div>
        <div className="sort-toggle discover-sort" aria-label="Sort online mods">
          <button
            className={sortMode === "downloads" ? "active" : ""}
            onClick={() => changeSort("downloads")}
            type="button"
          >
            Total Downloads
          </button>
          <button
            className={sortMode === "newest" ? "active" : ""}
            onClick={() => changeSort("newest")}
            type="button"
          >
            Newest
          </button>
          <button
            className={sortMode === "oldest" ? "active" : ""}
            onClick={() => changeSort("oldest")}
            type="button"
          >
            Oldest
          </button>
        </div>
      </section>

      <section className="discover-results" aria-label="Online mod results">
        {visibleMods.map((mod) => (
          <OnlineModCard
            expanded={expandedModId === mod.id}
            installing={installingModId === mod.id}
            key={`${mod.provider}:${mod.id}:${mod.version}`}
            mod={mod}
            onInstall={onInstall}
            onLoadFiles={onLoadFiles}
            onNeedsAuth={onNeedsAuth}
            onOpenPage={onOpenPage}
            onToggle={() => setExpandedModId((current) => (current === mod.id ? "" : mod.id))}
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
        {!isLoading && total > discoverPageSize ? (
          <div className="discover-pagination">
            <button
              className="secondary-button compact-button"
              disabled={currentPage <= 1}
              onClick={() => changePage(currentPage - 1)}
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
              onClick={() => changePage(currentPage + 1)}
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
  expanded: boolean;
  installing: boolean;
  mod: OnlineModRecord;
  onInstall(mod: OnlineModRecord, file?: OnlineModFileOption): void;
  onLoadFiles(mod: OnlineModRecord): Promise<OnlineModFileOption[]>;
  onNeedsAuth(): void;
  onOpenPage(url: string): void;
  onToggle(): void;
}

function OnlineModCard({
  expanded,
  installing,
  mod,
  onInstall,
  onLoadFiles,
  onNeedsAuth,
  onOpenPage,
  onToggle
}: OnlineModCardProps) {
  const [files, setFiles] = useState<OnlineModFileOption[]>([]);
  const [fileLoadState, setFileLoadState] = useState<"idle" | "loading" | "ready" | "error">(
    "idle"
  );
  const [fileLoadError, setFileLoadError] = useState("");
  const [selectedFileId, setSelectedFileId] = useState("");
  const fileRequestSequence = useRef(0);
  const needsAuth = !mod.installSupported && mod.provider === "nexus";
  const installDisabled = mod.installed || installing || (!mod.installSupported && !needsAuth);
  const pageUrl = mod.packageUrl ?? mod.websiteUrl;
  const selectedFile = files.find((file) => file.id === selectedFileId);
  const installTitle = !mod.installSupported
    ? (mod.installNote ?? `${mod.providerLabel} direct install is not available yet.`)
    : undefined;
  const statusLabel = mod.installed
    ? "Installed"
    : installing
      ? "Installing"
      : needsAuth
        ? "Needs auth"
        : mod.installSupported
          ? "Available"
          : "Unavailable";
  const installLabel = mod.installed
    ? "Installed"
    : installing
      ? "Installing"
      : mod.installSupported
        ? "Install"
        : "Needs Auth";
  const selectedActionLabel = mod.installed
    ? "Installed"
    : installing
      ? "Installing"
      : selectedFile?.action === "browser"
        ? "Confirm & Install"
        : selectedFile
          ? "Install Selected"
          : fileLoadState === "loading"
            ? "Loading Files"
            : "Select a File";
  const selectedActionDisabled =
    mod.installed ||
    installing ||
    !selectedFile ||
    selectedFile.action === "unsupported" ||
    selectedFile.action === "auth";

  useEffect(() => {
    fileRequestSequence.current += 1;
    setFiles([]);
    setSelectedFileId("");
    setFileLoadError("");
    setFileLoadState("idle");
  }, [mod.id]);

  useEffect(() => {
    if (!expanded || needsAuth || fileLoadState !== "idle") {
      return;
    }

    const requestId = ++fileRequestSequence.current;
    setFileLoadState("loading");
    void onLoadFiles(mod)
      .then((options) => {
        if (requestId !== fileRequestSequence.current) {
          return;
        }
        setFiles(options);
        setSelectedFileId(options.find((option) => option.primary)?.id ?? options[0]?.id ?? "");
        setFileLoadState("ready");
      })
      .catch((caughtError) => {
        if (requestId !== fileRequestSequence.current) {
          return;
        }
        setFileLoadError(String(caughtError));
        setFileLoadState("error");
      });
  }, [expanded, fileLoadState, mod, needsAuth, onLoadFiles]);

  function runSummaryAction() {
    if (needsAuth) {
      onNeedsAuth();
      return;
    }
    if (mod.provider === "nexus") {
      if (!expanded) {
        onToggle();
      }
      return;
    }
    onInstall(mod);
  }

  return (
    <article
      className={`online-mod-card${mod.installed ? " installed" : ""}${expanded ? " expanded" : ""}`}
    >
      <div className="online-mod-summary">
        <div className="online-mod-icon">
          {mod.iconUrl ? (
            <img alt="" decoding="async" loading="lazy" src={mod.iconUrl} />
          ) : (
            <PackagePlus size={24} />
          )}
        </div>
        <div className="online-mod-identity">
          <p className="eyebrow">{mod.providerLabel}</p>
          <h3>{mod.name}</h3>
          <span>{mod.owner || "Unknown author"}</span>
        </div>
        <div className="online-mod-row-stat" title={`${mod.downloads.toLocaleString()} downloads`}>
          <Download size={16} />
          <strong>{formatCompactNumber(mod.downloads)}</strong>
        </div>
        <span className="online-mod-row-version">v{mod.version || "Unknown"}</span>
        <span className={mod.installed ? "online-mod-row-state installed" : "online-mod-row-state"}>
          {mod.installed ? <CheckCircle2 size={17} /> : null}
          {statusLabel}
        </span>
        <button
          aria-label={`${installLabel} ${mod.name}`}
          className={needsAuth ? "online-mod-icon-button auth" : "online-mod-icon-button"}
          disabled={installDisabled}
          onClick={runSummaryAction}
          title={
            mod.provider === "nexus" && mod.installSupported
              ? `Choose a file for ${mod.name}`
              : (installTitle ?? `${installLabel} ${mod.name}`)
          }
          type="button"
        >
          {needsAuth ? <Settings2 size={18} /> : mod.installed ? <CheckCircle2 size={18} /> : <Download size={18} />}
        </button>
        <button
          aria-expanded={expanded}
          aria-label={`${expanded ? "Hide" : "Show"} details for ${mod.name}`}
          className="online-mod-expand-button"
          onClick={(event) => {
            event.stopPropagation();
            onToggle();
          }}
          title={expanded ? "Hide details" : "Show details"}
          type="button"
        >
          {expanded ? <ChevronUp size={20} /> : <ChevronDown size={20} />}
        </button>
      </div>

      {expanded ? (
        <div className="online-mod-details">
          <div className="online-mod-description">
            <p>{mod.description || "The provider did not include a description for this mod."}</p>
            {mod.installNote ? <small>{mod.installNote}</small> : null}
          </div>
          <dl className="online-mod-detail-grid">
            <div>
              <dt>Author</dt>
              <dd>{mod.owner || "Unknown"}</dd>
            </div>
            <div>
              <dt>Provider</dt>
              <dd>{mod.providerLabel}</dd>
            </div>
            <div>
              <dt>Version</dt>
              <dd>{mod.version || "Not provided"}</dd>
            </div>
            <div>
              <dt>Downloads</dt>
              <dd>{mod.downloads.toLocaleString()}</dd>
            </div>
            <div>
              <dt>Dependencies</dt>
              <dd>{mod.dependencyCount}</dd>
            </div>
            <div>
              <dt>File size</dt>
              <dd>{mod.fileSize ? formatFileSize(mod.fileSize) : "Not provided"}</dd>
            </div>
            <div>
              <dt>Published</dt>
              <dd>{formatOnlineDate(mod.createdAt)}</dd>
            </div>
            <div>
              <dt>Updated</dt>
              <dd>{formatOnlineDate(mod.updatedAt)}</dd>
            </div>
          </dl>
          <section className="online-mod-file-section" aria-label={`Files for ${mod.name}`}>
            <div className="online-mod-file-heading">
              <div>
                <span>Install file</span>
                <strong>Choose the release or variant you want</strong>
              </div>
              {files.length > 0 ? <small>{files.length} available</small> : null}
            </div>
            {needsAuth ? (
              <div className="online-mod-file-message warning">
                Add your Nexus API key in Settings to load this mod's available files.
              </div>
            ) : fileLoadState === "loading" ? (
              <div className="online-mod-file-message">
                <RefreshCw size={15} />
                Loading available files...
              </div>
            ) : fileLoadState === "error" ? (
              <div className="online-mod-file-message error">
                <AlertTriangle size={15} />
                {fileLoadError}
              </div>
            ) : fileLoadState === "ready" && files.length === 0 ? (
              <div className="online-mod-file-message warning">
                This provider did not return any installable archive files.
              </div>
            ) : files.length > 0 ? (
              <div className="online-mod-file-picker" role="radiogroup" aria-label="Mod file">
                {files.map((file) => {
                  const selected = file.id === selectedFileId;
                  const fileMeta = [
                    file.version ? `v${file.version}` : "",
                    file.category ?? "",
                    file.fileSize ? formatFileSize(file.fileSize) : "",
                    file.uploadedAt ? formatOnlineDate(file.uploadedAt) : ""
                  ].filter(Boolean);
                  return (
                    <label
                      className={selected ? "online-mod-file-option selected" : "online-mod-file-option"}
                      key={file.id}
                    >
                      <input
                        checked={selected}
                        name={`online-file-${mod.id}`}
                        onChange={() => setSelectedFileId(file.id)}
                        type="radio"
                        value={file.id}
                      />
                      <span>
                        <strong>{file.name}</strong>
                        <small>{fileMeta.join(" / ") || file.fileName || "Provider file"}</small>
                      </span>
                      {file.primary ? <b>Recommended</b> : null}
                    </label>
                  );
                })}
              </div>
            ) : null}
            {selectedFile?.description ? (
              <p className="online-mod-file-description">{selectedFile.description}</p>
            ) : null}
            {selectedFile?.action === "browser" ? (
              <div className="online-mod-file-message warning">
                Click Slow download, then Open via UniLoader for automated installs.
              </div>
            ) : null}
          </section>
          <div className="online-mod-detail-footer">
            <div className="online-mod-categories">
              <span>Categories</span>
              <p>{mod.categories.length > 0 ? mod.categories.join(", ") : "No categories provided"}</p>
            </div>
            <div className="online-mod-detail-actions">
              {pageUrl ? (
                <button
                  className="secondary-button compact-button"
                  onClick={() => onOpenPage(pageUrl)}
                  type="button"
                >
                  <ExternalLink size={15} />
                  Page
                </button>
              ) : null}
              <button
                className={
                  needsAuth || selectedFile?.action === "browser"
                    ? "secondary-button compact-button auth-button"
                    : "primary-button compact-button"
                }
                disabled={needsAuth ? false : selectedActionDisabled}
                onClick={() =>
                  needsAuth ? onNeedsAuth() : selectedFile ? onInstall(mod, selectedFile) : undefined
                }
                title={installTitle}
                type="button"
              >
                {needsAuth ? (
                  <Settings2 size={15} />
                ) : selectedFile?.action === "browser" ? (
                  <ExternalLink size={15} />
                ) : mod.installed ? (
                  <CheckCircle2 size={15} />
                ) : (
                  <Download size={15} />
                )}
                {needsAuth ? "Add API Key" : selectedActionLabel}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </article>
  );
}

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
  onSaveNexusApiKey(apiKey: string): Promise<void>;
  onUpdateSettings(settings: AppSettings): Promise<void>;
}

function SettingsView({
  appSettings,
  nexusAttentionId,
  onOpenExternalUrl,
  onSaveNexusApiKey,
  onUpdateSettings
}: SettingsViewProps) {
  const [nexusApiKeyDraft, setNexusApiKeyDraft] = useState("");
  const [isSavingNexusKey, setIsSavingNexusKey] = useState(false);

  async function submitNexusApiKey() {
    setIsSavingNexusKey(true);
    try {
      await onSaveNexusApiKey(nexusApiKeyDraft);
      setNexusApiKeyDraft("");
    } finally {
      setIsSavingNexusKey(false);
    }
  }

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
            placeholder={appSettings.nexusApiKeyConfigured ? "API key saved securely" : "Paste Nexus API key"}
            type="password"
            value={nexusApiKeyDraft}
            onChange={(event) => setNexusApiKeyDraft(event.target.value)}
          />
        </label>
        <div className="settings-key-actions">
          <button
            className="secondary-button"
            disabled={isSavingNexusKey || (!nexusApiKeyDraft.trim() && !appSettings.nexusApiKeyConfigured)}
            onClick={() => void submitNexusApiKey()}
            type="button"
          >
            {isSavingNexusKey ? "Saving" : nexusApiKeyDraft.trim() ? "Save Key" : "Remove Key"}
          </button>
        </div>
        <p className={appSettings.nexusApiKeyConfigured ? "settings-status connected" : "settings-status"}>
          {appSettings.nexusApiKeyConfigured
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

function nexusNxmReplayKey(rawUrl: string): string | null {
  try {
    const parsed = new URL(rawUrl);
    if (parsed.protocol.toLowerCase() !== "nxm:") {
      return null;
    }
    return `${parsed.hostname.toLowerCase()}${parsed.pathname}?expires=${parsed.searchParams.get("expires") ?? ""}&user_id=${parsed.searchParams.get("user_id") ?? ""}`;
  } catch {
    return null;
  }
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

function formatOnlineDate(value?: string): string {
  if (!value) {
    return "Not provided";
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return "Not provided";
  }

  return Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric"
  }).format(parsed);
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

  if (detection.loader === "none" && detection.expectedModFolders.length === 0) {
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
        UniLoader will detect the game setup after Steam game selection.
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
  const isRuntime = Boolean(mod.runtimeId);
  const [showExtraDetails, setShowExtraDetails] = useState(false);

  return (
    <article className={`mod-card ${mod.enabled ? "enabled" : "disabled"}${isRuntime ? " runtime" : ""}`}>
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
          <small>{isRuntime ? "System Runtime" : mod.enabled ? "Enabled" : "Disabled"}</small>
        </div>
      </div>

      {showExtraDetails ? (
        <div className="mod-details">
          <span>{mod.summary}</span>
          <div className="mod-meta">
            <span>{mod.adapterId}</span>
            <span>{mod.filesWritten.length} file(s)</span>
            {isRuntime ? (
              <span>{mod.externallyManaged ? "Detected in game folder" : "Managed by UniLoader"}</span>
            ) : null}
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
        {isRuntime ? (
          <span className="runtime-protected" title="Required runtimes are protected so installed mods keep working.">
            <ShieldCheck size={15} />
            Protected
          </span>
        ) : mod.enabled ? (
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
        {!isRuntime ? (
          <button className="danger-button compact-button" onClick={onRemove}>
            <Trash2 size={15} />
            Remove
          </button>
        ) : null}
      </div>
    </article>
  );
}
