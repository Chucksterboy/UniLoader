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
  Library,
  LockKeyhole,
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
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import {
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
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
  OnlineInstallSelection,
  ModConfigEntry,
  ModConfigFile,
  OnlineModFileOption,
  OnlineModRecord,
  ProfileImportResult,
  ProfileRefreshResult,
  SteamGameRecord
} from "../shared/contracts";
import { desktopApi } from "./tauriApi";
import { rememberBoundedSetValue } from "./boundedCache";
import { profileNeedsAttention } from "./health";
import {
  type MotionPhase,
  motionDurationMs,
  useFadePresence,
  useFadeSwitch
} from "./motion";

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
const installSoundUrls = {
  success: "./sounds/mod-install-success.wav",
  failure: "./sounds/mod-install-failed.wav"
} as const;
const installSoundVolume = 0.72;

type SteamArtworkVariant = "hero" | "poster";

const gameArtworkSourceCache = new Map<string, string>();
const gameArtworkRequestCache = new Map<string, Promise<string | null>>();

function gameArtworkCacheKey(steamAppId: string, variant: SteamArtworkVariant): string {
  return `${steamAppId}:${variant}`;
}

function loadCachedGameArtwork(
  steamAppId: string,
  variant: SteamArtworkVariant
): Promise<string | null> {
  const cacheKey = gameArtworkCacheKey(steamAppId, variant);
  const rememberedSource = gameArtworkSourceCache.get(cacheKey);
  if (rememberedSource) {
    return Promise.resolve(rememberedSource);
  }

  const existingRequest = gameArtworkRequestCache.get(cacheKey);
  if (existingRequest) {
    return existingRequest;
  }

  const request = desktopApi
    .getCachedSteamArtwork(steamAppId, variant)
    .then((source) => {
      if (source) {
        gameArtworkSourceCache.set(cacheKey, source);
      }
      return source;
    })
    .finally(() => gameArtworkRequestCache.delete(cacheKey));
  gameArtworkRequestCache.set(cacheKey, request);
  return request;
}

type ViewMode = "manager" | "discover" | "transfer" | "settings";
type ModSortMode = "newest" | "oldest";
type OnlineSortMode = "downloads" | "newest" | "oldest";
type TransferMode = "import" | "export" | null;
type NoticeKind = "success" | "warning" | "error";
type InstallSoundKind = keyof typeof installSoundUrls;
type StartupSplashPhase = "intro" | "exiting" | "hidden";
type GameLaunchState = "idle" | "requesting" | "waiting" | "running";

interface PendingDependencyPrompt {
  profileId: string;
  mod: OnlineModRecord;
  file?: OnlineModFileOption;
  selection?: OnlineInstallSelection;
  preflight: InstallPreflightResult;
}

interface PendingNexusInstall {
  profileId: string;
  modId: string;
}

interface Notice {
  motionId: number;
  kind: NoticeKind;
  title: string;
  detail: string;
}

interface RecentActivity {
  id: number;
  kind: NoticeKind;
  label: string;
  time: string;
}

type NoticeInput = Omit<Notice, "motionId">;

interface ConfigModalState {
  mod: InstalledModRecord;
  files: ModConfigFile[];
  isLoading: boolean;
  error?: string;
}

type ForceRemovalPrompt =
  | {
      kind: "profile";
      profile: GameProfile;
      detail: string;
    }
  | {
      kind: "mod";
      profileId: string;
      mod: InstalledModRecord;
      source: "library" | "discovery";
      detail: string;
    };

const discoverPageSize = 20;
const nexusApiKeysUrl = "https://www.nexusmods.com/settings/api-keys";
const forceRemovalRequiredPrefix = "UNILOADER_FORCE_REMOVAL_REQUIRED:";
const startupSplashPulseMs = 2700;
const startupSplashFadeMs = 420;
const startupSplashMaximumMs = 8000;
const modPresentationStorageKey = "uniloader.mod-presentations.v2";
const maxInstalledModProfileCaches = 12;
const maxArtworkRefreshProfiles = 48;
const maxProcessedNxmLinks = 128;
const maxStoredModPresentations = 400;

function forceRemovalDetail(error: unknown): string | null {
  const message = String(error);
  const prefixIndex = message.indexOf(forceRemovalRequiredPrefix);
  if (prefixIndex < 0) {
    return null;
  }
  return message.slice(prefixIndex + forceRemovalRequiredPrefix.length).trim();
}

interface StoredModPresentation {
  cachedAt?: string;
  description?: string;
  iconUrl?: string;
  name?: string;
  owner?: string;
  providerLabel?: string;
  updatedAt?: string;
  version?: string;
}

type StoredModPresentations = Record<string, StoredModPresentation>;

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
  const [discoverInstalledMods, setDiscoverInstalledMods] = useState<InstalledModRecord[]>([]);
  const [discoverInstalledModsProfileId, setDiscoverInstalledModsProfileId] = useState("");
  const [isLoadingDiscoverInstalledMods, setIsLoadingDiscoverInstalledMods] = useState(false);
  const [removingDiscoverModId, setRemovingDiscoverModId] = useState("");
  const [onlineMods, setOnlineMods] = useState<OnlineModRecord[]>([]);
  const [onlineModTotal, setOnlineModTotal] = useState(0);
  const [modSortMode, setModSortMode] = useState<ModSortMode>("newest");
  const [installedModQuery, setInstalledModQuery] = useState("");
  const [expandedInstalledModId, setExpandedInstalledModId] = useState("");
  const [modPresentations, setModPresentations] = useState<StoredModPresentations>(() =>
    loadStoredModPresentations()
  );
  const [transferMode, setTransferMode] = useState<TransferMode>(null);
  const [detection, setDetection] = useState<GameDetectionResult | null>(null);
  const [isDetecting, setIsDetecting] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isDiscoveringMods, setIsDiscoveringMods] = useState(false);
  const [installingOnlineModId, setInstallingOnlineModId] = useState<string>("");
  const [onlineInstallCompletionId, setOnlineInstallCompletionId] = useState(0);
  const [isCheckingForUpdate, setIsCheckingForUpdate] = useState(false);
  const [isDownloadingUpdate, setIsDownloadingUpdate] = useState(false);
  const [isTransferringProfile, setIsTransferringProfile] = useState(false);
  const [isScanningSteamGames, setIsScanningSteamGames] = useState(false);
  const [isCreatingSteamProfile, setIsCreatingSteamProfile] = useState(false);
  const [isChangingProfileLaunchMode, setIsChangingProfileLaunchMode] = useState(false);
  const [gameLaunchState, setGameLaunchState] = useState<GameLaunchState>("idle");
  const [steamGames, setSteamGames] = useState<SteamGameRecord[]>([]);
  const [selectedSteamGameAppId, setSelectedSteamGameAppId] = useState("");
  const [profileCreatorOpen, setProfileCreatorOpen] = useState(false);
  const [steamGameSearch, setSteamGameSearch] = useState("");
  const [status, setStatus] = useState<string>("Ready");
  const [notice, setNoticeState] = useState<Notice | null>(null);
  const [recentActivities, setRecentActivities] = useState<RecentActivity[]>([]);
  const [nexusSettingsAttentionId, setNexusSettingsAttentionId] = useState(0);
  const [configModal, setConfigModal] = useState<ConfigModalState | null>(null);
  const [profilePendingRename, setProfilePendingRename] = useState<GameProfile | null>(null);
  const [profilePendingRemoval, setProfilePendingRemoval] = useState<GameProfile | null>(null);
  const [forceRemovalPrompt, setForceRemovalPrompt] = useState<ForceRemovalPrompt | null>(null);
  const [pendingDependencyPrompt, setPendingDependencyPrompt] =
    useState<PendingDependencyPrompt | null>(null);
  const selectedProfileIdRef = useRef("");
  const discoverProfileIdRef = useRef("");
  const [error, setErrorState] = useState<string>("");
  const [errorMotionId, setErrorMotionId] = useState(0);
  const noticeSequence = useRef(0);
  const recentActivitySequence = useRef(0);
  const errorSequence = useRef(0);
  const updateAnnouncementShown = useRef(false);
  const installedModsRequestSequence = useRef(0);
  const installedModsByProfile = useRef(new Map<string, InstalledModRecord[]>());
  const artworkRefreshAttemptedProfiles = useRef(new Set<string>());
  const discoverInstalledModsRequestSequence = useRef(0);
  const discoveryRequestSequence = useRef(0);
  const processedNxmLinks = useRef(new Set<string>());
  const pendingNexusInstallRef = useRef<PendingNexusInstall | null>(null);
  const pendingNexusInstallTimeoutRef = useRef<number | null>(null);
  const installSoundPlayers = useRef<Record<InstallSoundKind, HTMLAudioElement> | null>(null);
  const gameLaunchStateRef = useRef<GameLaunchState>("idle");
  const gameLaunchDeadlineRef = useRef(0);

  function setNotice(nextNotice: NoticeInput | null) {
    setNoticeState(nextNotice ? { ...nextNotice, motionId: ++noticeSequence.current } : null);
    if (nextNotice) {
      const activity: RecentActivity = {
        id: ++recentActivitySequence.current,
        kind: nextNotice.kind,
        label: nextNotice.title,
        time: new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
      };
      setRecentActivities((current) => {
        if (current[0]?.label === activity.label && current[0]?.kind === activity.kind) {
          return [{ ...activity, id: current[0].id }, ...current.slice(1)];
        }
        return [activity, ...current].slice(0, 5);
      });
    }
  }

  function setError(nextError: string) {
    if (nextError) {
      setErrorMotionId(++errorSequence.current);
    }
    setErrorState(nextError);
  }

  function updateGameLaunchState(nextState: GameLaunchState) {
    gameLaunchStateRef.current = nextState;
    setGameLaunchState(nextState);
  }

  function playInstallSound(kind: InstallSoundKind) {
    const player = installSoundPlayers.current?.[kind];
    if (!player) {
      return;
    }

    try {
      player.pause();
      player.currentTime = 0;
      void player.play().catch(() => undefined);
    } catch {
      // Sound playback must never interfere with the installation result.
    }
  }

  function rememberOnlineModPresentations(mods: OnlineModRecord[]) {
    setModPresentations((current) => {
      let changed = false;
      const next = { ...current };

      for (const mod of mods) {
        if (!mod.id) {
          continue;
        }
        const presentationKey = mod.id.trim().toLowerCase();
        const presentation: StoredModPresentation = {
          description: mod.description || undefined,
          iconUrl: mod.iconUrl || undefined,
          name: mod.name || undefined,
          owner: mod.owner || undefined,
          providerLabel: mod.providerLabel || undefined,
          updatedAt: mod.updatedAt || undefined,
          version: mod.version || undefined
        };
        const existing = current[presentationKey];
        if (!sameStoredModPresentation(existing, presentation)) {
          next[presentationKey] = {
            ...presentation,
            cachedAt: new Date().toISOString()
          };
          changed = true;
        }
      }

      if (!changed) {
        return current;
      }
      const pruned = pruneStoredModPresentations(next);
      saveStoredModPresentations(pruned);
      return pruned;
    });
  }

  function rememberInstalledMods(profileId: string, mods: InstalledModRecord[]) {
    installedModsByProfile.current.delete(profileId);
    installedModsByProfile.current.set(profileId, mods);
    while (installedModsByProfile.current.size > maxInstalledModProfileCaches) {
      const oldestProfileId = installedModsByProfile.current.keys().next().value;
      if (!oldestProfileId) {
        break;
      }
      installedModsByProfile.current.delete(oldestProfileId);
    }
  }

  function scheduleInstalledModArtworkRefresh(
    profileId: string,
    mods: InstalledModRecord[]
  ) {
    const needsArtwork = mods.some((mod) => !mod.runtimeId && !mod.iconUrl);
    if (!needsArtwork || artworkRefreshAttemptedProfiles.current.has(profileId)) {
      return;
    }

    rememberBoundedSetValue(
      artworkRefreshAttemptedProfiles.current,
      profileId,
      maxArtworkRefreshProfiles
    );
    void api
      .refreshInstalledModArtwork(profileId)
      .then((enrichedMods) => {
        rememberInstalledMods(profileId, enrichedMods);
        if (selectedProfileIdRef.current === profileId) {
          setInstalledMods(enrichedMods);
        }
        if (discoverProfileIdRef.current === profileId) {
          setDiscoverInstalledMods(enrichedMods);
          setDiscoverInstalledModsProfileId(profileId);
        }
      })
      .catch(() => {
        // Artwork is optional and must never block profile or mod operations.
      });
  }

  const api = desktopApi;
  const viewMotion = useFadeSwitch(activeView);
  const renderedView = viewMotion.value;
  const configModalPresence = useFadePresence(configModal);
  const profileRenamePresence = useFadePresence(profilePendingRename);
  const profileRemovalPresence = useFadePresence(profilePendingRemoval);
  const forceRemovalPresence = useFadePresence(forceRemovalPrompt);
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
    () => {
      const query = installedModQuery.trim().toLowerCase();
      return [...installedMods]
        .filter((mod) => {
          if (!query) {
            return true;
          }
          const presentation = getStoredModPresentation(mod, modPresentations);
          return [
            displayModName(mod),
            mod.archiveName,
            mod.adapterId,
            mod.packageId ?? "",
            presentation.owner ?? "",
            presentation.providerLabel ?? ""
          ]
            .join(" ")
            .toLowerCase()
            .includes(query);
        })
        .sort((first, second) => {
        const firstTime = Date.parse(first.installedAt) || 0;
        const secondTime = Date.parse(second.installedAt) || 0;
        return modSortMode === "newest" ? secondTime - firstTime : firstTime - secondTime;
        });
    },
    [installedModQuery, installedMods, modPresentations, modSortMode]
  );
  const dependencyChecks = useMemo(() => {
    const checks = new Map<string, { id: string; label: string; satisfied: boolean; version?: string }>();
    for (const mod of installedMods) {
      if (mod.runtimeId) {
        checks.set(`runtime:${mod.runtimeId.toLowerCase()}`, {
          id: `runtime:${mod.runtimeId.toLowerCase()}`,
          label: displayModName(mod),
          satisfied: true,
          version: mod.packageVersion
        });
      }
      for (const dependency of mod.dependencies) {
        const id = dependency.id.trim().toLowerCase();
        if (!id) {
          continue;
        }
        const existing = checks.get(id);
        const satisfied = dependency.status === "already-installed";
        checks.set(id, {
          id,
          label: dependency.name || dependency.id,
          satisfied: existing?.satisfied || satisfied,
          version: dependency.version ?? existing?.version
        });
      }
    }
    return [...checks.values()].sort((first, second) => {
      if (first.satisfied !== second.satisfied) {
        return first.satisfied ? 1 : -1;
      }
      return first.label.localeCompare(second.label);
    });
  }, [installedMods]);
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
  const selectedProfileLaunchModsEnabled = selectedProfile
    ? selectedProfile.modsEnabled
    : true;
  const profileModToggleLabel = selectedProfileLaunchModsEnabled ? "ON" : "OFF";
  const gameLaunchBusy = gameLaunchState !== "idle";
  const displayedOnlineMods =
    discoverLoadedProfileId === discoverProfileId ? onlineMods : [];
  const selectedPlan = analysis?.recommendedPlan;
  const detectionIssue = getDetectionIssue(detection);
  const profileHealthNeedsAttention = profileNeedsAttention(
    selectedProfile,
    selectedProfileFolderConnected,
    detection?.warnings.length ?? 0
  );
  const healthTone: NoticeKind =
    error || notice?.kind === "error"
      ? "error"
      : notice?.kind === "warning" || profileHealthNeedsAttention
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
    selectedProfileIdRef.current = selectedProfileId;
  }, [selectedProfileId]);

  useEffect(() => {
    discoverProfileIdRef.current = discoverProfileId;
  }, [discoverProfileId]);

  useEffect(() => {
    const players = {
      success: new Audio(installSoundUrls.success),
      failure: new Audio(installSoundUrls.failure)
    };
    Object.values(players).forEach((player) => {
      player.preload = "auto";
      player.volume = installSoundVolume;
      player.load();
    });
    installSoundPlayers.current = players;

    return () => {
      Object.values(players).forEach((player) => {
        player.pause();
        player.removeAttribute("src");
        player.load();
      });
      installSoundPlayers.current = null;
    };
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
    const profileId = selectedProfile?.id;
    const profileName = selectedProfile?.name;
    if (!profileId || !selectedProfile?.steamAppId) {
      gameLaunchDeadlineRef.current = 0;
      updateGameLaunchState("idle");
      return;
    }

    let cancelled = false;
    let timeoutId = 0;
    let consecutiveStoppedChecks = 0;
    gameLaunchDeadlineRef.current = 0;
    updateGameLaunchState("idle");

    const checkRunningState = async () => {
      try {
        const running = await api.profileGameRunning(profileId);
        if (cancelled) {
          return;
        }

        const currentState = gameLaunchStateRef.current;
        if (running) {
          consecutiveStoppedChecks = 0;
          if (currentState !== "running") {
            updateGameLaunchState("running");
            setStatus("Game running");
            if (currentState === "requesting" || currentState === "waiting") {
              setNotice({
                kind: "success",
                title: "Game launched",
                detail: `${profileName} is now running.`
              });
            }
          }
        } else if (currentState === "running") {
          consecutiveStoppedChecks += 1;
          if (consecutiveStoppedChecks >= 2) {
            updateGameLaunchState("idle");
            setStatus("Ready");
          }
        } else if (
          currentState === "waiting" &&
          gameLaunchDeadlineRef.current > 0 &&
          Date.now() >= gameLaunchDeadlineRef.current
        ) {
          gameLaunchDeadlineRef.current = 0;
          updateGameLaunchState("idle");
          setStatus("Launch not detected");
          setNotice({
            kind: "warning",
            title: "Launch not detected",
            detail: `Steam accepted the request, but UniLoader could not detect ${profileName} running.`
          });
        } else {
          consecutiveStoppedChecks = 0;
        }
      } catch {
        // A protected process can briefly reject inspection while a game starts or exits.
      }

      if (!cancelled) {
        timeoutId = window.setTimeout(checkRunningState, 1500);
      }
    };

    void checkRunningState();
    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [selectedProfile?.id, selectedProfile?.name, selectedProfile?.steamAppId]);

  useEffect(() => {
    if (activeView !== "discover") {
      return;
    }

    const next =
      discoverProfileId && discoveryProfiles.some((profile) => profile.id === discoverProfileId)
        ? discoverProfileId
        : selectedProfile?.steamAppId
          ? selectedProfileId
          : discoveryProfiles[0]?.id || "";
    if (next !== discoverProfileId) {
      selectDiscoverProfile(next);
    }
  }, [
    activeView,
    discoverProfileId,
    discoveryProfiles,
    selectedProfile?.steamAppId,
    selectedProfileId
  ]);

  useEffect(() => {
    if (activeView !== "discover") {
      return;
    }

    if (!discoverProfileId) {
      discoverInstalledModsRequestSequence.current += 1;
      setDiscoverInstalledMods([]);
      setDiscoverInstalledModsProfileId("");
      setIsLoadingDiscoverInstalledMods(false);
      return;
    }

    void refreshDiscoverInstalledMods(discoverProfileId).catch(() => undefined);
  }, [activeView, discoverProfileId, onlineInstallCompletionId]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void Promise.resolve()
      .then(() =>
        getCurrentWebview().onDragDropEvent((event) => {
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
      )
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
      if (!selectedProfileIdRef.current) {
        selectProfile(loadedProfiles[0]?.id || "");
      }
      setAppSettings(loadedSettings);
    } catch (caughtError) {
      setError(String(caughtError));
    } finally {
      setBootstrapReady(true);
    }
  }

  function selectProfile(profileId: string) {
    if (selectedProfileIdRef.current !== profileId) {
      installedModsRequestSequence.current += 1;
      selectedProfileIdRef.current = profileId;
      setInstalledMods(installedModsByProfile.current.get(profileId) ?? []);
      setExpandedInstalledModId("");
    }
    setSelectedProfileId(profileId);
  }

  function selectDiscoverProfile(profileId: string) {
    if (discoverProfileIdRef.current !== profileId) {
      discoveryRequestSequence.current += 1;
      discoverInstalledModsRequestSequence.current += 1;
      discoverProfileIdRef.current = profileId;
      setIsDiscoveringMods(false);
      setIsLoadingDiscoverInstalledMods(false);
    }
    setDiscoverProfileId(profileId);
  }

  async function refreshInstalledMods(profileId: string) {
    const requestId = ++installedModsRequestSequence.current;
    const mods = await api.listInstalledMods(profileId);
    rememberInstalledMods(profileId, mods);
    if (
      requestId === installedModsRequestSequence.current &&
      selectedProfileIdRef.current === profileId
    ) {
      setInstalledMods(mods);
      scheduleInstalledModArtworkRefresh(profileId, mods);
    }
  }

  async function refreshDiscoverInstalledMods(profileId: string) {
    const requestId = ++discoverInstalledModsRequestSequence.current;
    setIsLoadingDiscoverInstalledMods(true);
    try {
      const mods = await api.listInstalledMods(profileId);
      if (
        requestId === discoverInstalledModsRequestSequence.current &&
        discoverProfileIdRef.current === profileId
      ) {
        setDiscoverInstalledMods(mods);
        setDiscoverInstalledModsProfileId(profileId);
      }
    } catch (caughtError) {
      if (
        requestId === discoverInstalledModsRequestSequence.current &&
        discoverProfileIdRef.current === profileId
      ) {
        setDiscoverInstalledMods([]);
        setDiscoverInstalledModsProfileId(profileId);
      }
      throw caughtError;
    } finally {
      if (
        requestId === discoverInstalledModsRequestSequence.current &&
        discoverProfileIdRef.current === profileId
      ) {
        setIsLoadingDiscoverInstalledMods(false);
      }
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
      rememberBoundedSetValue(processedNxmLinks.current, replayKey, maxProcessedNxmLinks);
      const pendingInstall = pendingNexusInstallRef.current;
      const activeModId = pendingInstall?.modId ?? nexusModIdFromNxmUrl(nxmUrl) ?? "nexus-handoff";
      setError("");
      setInstallingOnlineModId(activeModId);
      setStatus("Installing Nexus download");
      setNotice({
        kind: "warning",
        title: "Nexus download authorized",
        detail: "UniLoader received the selected file and is checking it before installation."
      });

      try {
        const result = await api.installNexusNxmLink(nxmUrl);
        playInstallSound("success");
        setOnlineInstallCompletionId((current) => current + 1);
        selectProfile(result.installResult.profileId);
        setOnlineMods((current) =>
          current.map((item) =>
            item.id === result.modId ? { ...item, installed: true } : item
          )
        );
        await refreshInstalledMods(result.installResult.profileId).catch(() => undefined);
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
        playInstallSound("failure");
        processedNxmLinks.current.delete(replayKey);
        setError(String(caughtError));
        setStatus("Nexus install failed");
        setNotice({
          kind: "error",
          title: "Nexus install failed",
          detail: String(caughtError)
        });
      } finally {
        if (pendingNexusInstallTimeoutRef.current !== null) {
          window.clearTimeout(pendingNexusInstallTimeoutRef.current);
          pendingNexusInstallTimeoutRef.current = null;
        }
        pendingNexusInstallRef.current = null;
        setInstallingOnlineModId("");
      }
    }
  }

  async function updateAppSetting(nextSettings: AppSettings) {
    const previousSettings = appSettings;
    setAppSettings(nextSettings);
    try {
      const savedSettings = await api.updateAppSettings(nextSettings);
      setAppSettings(savedSettings);
      setStatus("Settings saved");
    } catch (caughtError) {
      setAppSettings(previousSettings);
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
          updateInfo.installerName,
          updateInfo.installerSize,
          updateInfo.installerSha256
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
      const result = await api.refreshProfile(selectedProfile.id);

      setProfiles((current) =>
        current.map((profile) => (profile.id === result.profile.id ? result.profile : profile))
      );
      selectProfile(result.profile.id);
      setDetection(result.detection);
      rememberInstalledMods(result.profile.id, result.installedMods);
      setInstalledMods(result.installedMods);
      scheduleInstalledModArtworkRefresh(result.profile.id, result.installedMods);
      setAnalysis(null);
      const hasWarnings = result.warnings.length > 0;
      const updatedRuntime = (result.runtimeUpdates?.length ?? 0) > 0;
      setStatus(hasWarnings ? "Needs attention" : "Ready");
      setNotice({
        kind: hasWarnings ? "warning" : "success",
        title: hasWarnings
          ? "Refresh found issues"
          : updatedRuntime
            ? "Runtime updated"
            : "Profile refreshed",
        detail: refreshSummary(result)
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

  async function removeProfile(profile: GameProfile, forceForgetModified = false) {
    setError("");
    setStatus("Removing profile");
    try {
      const result = await api.removeProfile(profile.id, forceForgetModified);
      const nextProfiles = profiles.filter((item) => item.id !== profile.id);
      setProfiles(nextProfiles);

      if (selectedProfileIdRef.current === profile.id) {
        const nextSelectedProfileId = nextProfiles[0]?.id ?? "";
        selectProfile(nextSelectedProfileId);
        setInstalledMods([]);
        setAnalysis(null);
        setNotice(null);
        if (nextSelectedProfileId) {
          await refreshInstalledMods(nextSelectedProfileId);
        }
      }

      setStatus("Profile removed");
      setProfilePendingRemoval(null);
      setForceRemovalPrompt(null);
      const baseDetail =
        result.removedModRecords > 0
          ? `${result.name} was removed with ${result.removedModRecords} UniLoader mod record(s).`
          : `${result.name} was removed.`;
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Profile removed",
        detail: result.warnings.length > 0 ? `${baseDetail} ${result.warnings[0]}` : baseDetail
      });
    } catch (caughtError) {
      const detail = forceRemovalDetail(caughtError);
      if (detail && !forceForgetModified) {
        setProfilePendingRemoval(null);
        setForceRemovalPrompt({ kind: "profile", profile, detail });
        setStatus("Removal confirmation required");
        return;
      }
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
      selectProfile(existingProfile.id);
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
      void loadCachedGameArtwork(game.appId, "poster").catch(() => undefined);
      void loadCachedGameArtwork(game.appId, "hero").catch(() => undefined);
      setProfiles((current) => [...current, profile]);
      selectProfile(profile.id);
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
    updateGameLaunchState("requesting");
    gameLaunchDeadlineRef.current = Date.now() + 60_000;
    const launchWithMods = selectedProfileLaunchModsEnabled;
    setStatus(launchWithMods ? "Launching with mods" : "Launching without mods");
    try {
      await api.launchProfileGame(selectedProfile.id, launchWithMods);
      if (gameLaunchStateRef.current !== "running") {
        updateGameLaunchState("waiting");
      }
      setStatus("Waiting for game");
      setNotice({
        kind: "warning",
        title: "Starting game",
        detail: launchWithMods
          ? `Steam is starting ${selectedProfile.name} with the enabled library mods.`
          : `Steam is starting ${selectedProfile.name} without user mods.`
      });
    } catch (caughtError) {
      gameLaunchDeadlineRef.current = 0;
      updateGameLaunchState("idle");
      setError(String(caughtError));
      setStatus("Launch failed");
      setNotice({
        kind: "error",
        title: "Launch failed",
        detail: String(caughtError)
      });
    }
  }

  async function setProfileLaunchModsEnabled(enabled: boolean) {
    if (!selectedProfile) {
      return;
    }

    const profile = selectedProfile;
    setError("");
    setIsChangingProfileLaunchMode(true);
    setStatus(enabled ? "Restoring managed components" : "Preparing unmodded state");
    try {
      const result = await api.setProfileModLaunchMode(profile.id, enabled);
      setProfiles((current) =>
        current.map((item) =>
          item.id === profile.id ? { ...item, modsEnabled: result.modsEnabled } : item
        )
      );
      setStatus(enabled ? "Mods enabled for launch" : "Unmodded state ready");
      setNotice({
        kind: "success",
        title: enabled ? "Managed components restored" : "Unmodded state ready",
        detail: enabled
          ? `Restored ${result.changedComponents} UniLoader-managed component${result.changedComponents === 1 ? "" : "s"}.`
          : `Suspended ${result.changedComponents} UniLoader-managed component${result.changedComponents === 1 ? "" : "s"}, including applicable runtimes. Pre-existing game files were preserved.`
      });
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus(enabled ? "Restore failed" : "Clean mode failed");
      setNotice({
        kind: "error",
        title: enabled
          ? "Could not restore managed components"
          : "Could not prepare unmodded state",
        detail: String(caughtError)
      });
    } finally {
      setIsChangingProfileLaunchMode(false);
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

  function applyImportedProfileResult(result: ProfileImportResult) {
    setProfiles((current) => [...current, result.profile]);
    selectProfile(result.profile.id);
    rememberInstalledMods(result.profile.id, result.installedMods);
    setInstalledMods(result.installedMods);
    setAnalysis(null);
    setDetection(null);
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
      applyImportedProfileResult(result);
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
  ): Promise<boolean> {
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
        query,
        requestId
      );
      if (
        requestId !== discoveryRequestSequence.current ||
        discoverProfileIdRef.current !== profileId
      ) {
        return false;
      }
      rememberOnlineModPresentations(result.items);
      setOnlineMods(result.items);
      setOnlineModTotal(result.total);
      setDiscoverLoadedProfileId(profileId);
      const providerWarning = result.providerWarnings[0];
      setStatus(providerWarning ? "Discovery ready with warnings" : "Discovery ready");
      const profile = profiles.find((item) => item.id === profileId);
      setNotice({
        kind: providerWarning ? "warning" : "success",
        title: providerWarning ? "Discovery partially updated" : "Discovery updated",
        detail: [
          `${profile?.name ?? "Selected profile"}: found ${result.total} online mod(s).`,
          providerWarning
        ]
          .filter(Boolean)
          .join(" ")
      });
      return true;
    } catch (caughtError) {
      if (
        requestId !== discoveryRequestSequence.current ||
        discoverProfileIdRef.current !== profileId
      ) {
        return false;
      }
      setError(String(caughtError));
      setStatus("Discovery failed");
      setNotice({
        kind: "error",
        title: "Discovery failed",
        detail: String(caughtError)
      });
      return false;
    } finally {
      if (
        requestId === discoveryRequestSequence.current &&
        discoverProfileIdRef.current === profileId
      ) {
        setIsDiscoveringMods(false);
      }
    }
  }

  async function loadOnlineModFiles(mod: OnlineModRecord) {
    const profileId = discoverProfileIdRef.current;
    if (!profileId) {
      throw new Error("Select a profile before loading mod files.");
    }
    return api.listDiscoveredModFiles(profileId, mod);
  }

  async function installOnlineMod(
    mod: OnlineModRecord,
    file?: OnlineModFileOption,
    selection?: OnlineInstallSelection,
    skipDependencyPrompt = false,
    requestedProfileId?: string
  ) {
    const targetProfileId = requestedProfileId ?? discoverProfileIdRef.current;
    if (!targetProfileId) {
      setNotice({
        kind: "warning",
        title: "Select a profile",
        detail: "Choose the profile you want to install this mod into."
      });
      return;
    }

    if (mod.provider === "nexus" && !skipDependencyPrompt) {
      try {
        const preflight = await api.preflightDiscoveredModInstall(targetProfileId, mod);
        if (preflight.confirmationRequired) {
          setPendingDependencyPrompt({ profileId: targetProfileId, mod, file, selection, preflight });
          setStatus("Dependency confirmation needed");
          return;
        }
      } catch (caughtError) {
        playInstallSound("failure");
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
      setError("");
      setInstallingOnlineModId(mod.id);
      pendingNexusInstallRef.current = { profileId: targetProfileId, modId: mod.id };
      if (pendingNexusInstallTimeoutRef.current !== null) {
        window.clearTimeout(pendingNexusInstallTimeoutRef.current);
      }
      pendingNexusInstallTimeoutRef.current = window.setTimeout(() => {
        if (pendingNexusInstallRef.current?.modId === mod.id) {
          pendingNexusInstallRef.current = null;
          setInstallingOnlineModId("");
          setStatus("Nexus confirmation expired");
        }
        pendingNexusInstallTimeoutRef.current = null;
      }, 10 * 60 * 1000);
      try {
        const downloadPageUrl = await api.beginNexusBrowserDownload(
          targetProfileId,
          mod,
          file,
          selection
        );
        await openExternalUrl(downloadPageUrl);
        setStatus("Waiting for Nexus confirmation");
        setNotice({
          kind: "warning",
          title: "Continue in Nexus",
          detail: "Click Slow download, then Open via UniLoader for automated installs."
        });
      } catch (caughtError) {
        if (pendingNexusInstallTimeoutRef.current !== null) {
          window.clearTimeout(pendingNexusInstallTimeoutRef.current);
          pendingNexusInstallTimeoutRef.current = null;
        }
        pendingNexusInstallRef.current = null;
        setInstallingOnlineModId("");
        playInstallSound("failure");
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
      const result = await api.installDiscoveredMod(targetProfileId, mod, file, selection);
      playInstallSound("success");
      setOnlineInstallCompletionId((current) => current + 1);
      if (discoverProfileIdRef.current === targetProfileId) {
        setOnlineMods((current) =>
          current.map((item) => (item.id === mod.id ? { ...item, installed: true } : item))
        );
      }
      if (targetProfileId === selectedProfileIdRef.current) {
        await refreshInstalledMods(targetProfileId).catch(() => undefined);
      }
      setStatus("Mod installed");
      setNotice({
        kind: result.warnings.length > 0 ? "warning" : "success",
        title: "Online mod installed",
        detail: installSuccessDetail(mod.name, result.filesWritten.length, result.warnings)
      });
    } catch (caughtError) {
      playInstallSound("failure");
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
          prompt.profileId,
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
        playInstallSound("failure");
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

    await installOnlineMod(prompt.mod, prompt.file, prompt.selection, true, prompt.profileId);
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

  async function openDiagnosticsFolder() {
    setError("");
    try {
      await api.openDiagnosticsFolder();
      setStatus("Diagnostics folder opened");
    } catch (caughtError) {
      setError(String(caughtError));
      setStatus("Diagnostics unavailable");
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
      playInstallSound("failure");
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
      playInstallSound("failure");
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
      playInstallSound("failure");
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
      playInstallSound("failure");
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
      playInstallSound("success");
      await refreshInstalledMods(selectedProfile.id).catch(() => undefined);
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
    mod: InstalledModRecord,
    forceForgetModified = false
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
            : await api.removeMod(selectedProfile.id, mod.id, forceForgetModified);
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
      const detail = forceRemovalDetail(caughtError);
      if (action === "remove" && detail && !forceForgetModified) {
        setForceRemovalPrompt({
          kind: "mod",
          profileId: selectedProfile.id,
          mod,
          source: "library",
          detail
        });
        setStatus("Removal confirmation required");
        return;
      }
      setError(String(caughtError));
      setStatus("Action failed");
      setNotice({
        kind: "error",
        title: "Action failed",
        detail: String(caughtError)
      });
    }
  }

  async function removeDiscoverInstalledMod(
    mod: InstalledModRecord,
    forceForgetModified = false
  ) {
    const profileId = discoverProfileIdRef.current;
    if (!profileId || mod.runtimeId) {
      return;
    }

    setError("");
    setRemovingDiscoverModId(mod.id);
    setStatus("Removing mod");
    try {
      const result = await api.removeMod(profileId, mod.id, forceForgetModified);
      const refreshes: Promise<void>[] = [refreshDiscoverInstalledMods(profileId)];
      if (profileId === selectedProfileIdRef.current) {
        refreshes.push(refreshInstalledMods(profileId));
      }
      await Promise.all(refreshes);
      setOnlineMods((current) =>
        current.map((onlineMod) =>
          installedRecordMatchesOnlineMod(mod, onlineMod)
            ? { ...onlineMod, installed: false }
            : onlineMod
        )
      );
      setStatus("Mod removed");
      setNotice({
        kind: "warning",
        title: "Mod removed",
        detail: `${displayModName(mod)}: ${result.filesChanged.length} file(s) changed.`
      });
    } catch (caughtError) {
      const detail = forceRemovalDetail(caughtError);
      if (detail && !forceForgetModified) {
        setForceRemovalPrompt({
          kind: "mod",
          profileId,
          mod,
          source: "discovery",
          detail
        });
        setStatus("Removal confirmation required");
        return;
      }
      setError(String(caughtError));
      setStatus("Remove failed");
      setNotice({
        kind: "error",
        title: "Remove failed",
        detail: String(caughtError)
      });
    } finally {
      setRemovingDiscoverModId("");
    }
  }

  function confirmForceRemoval(prompt: ForceRemovalPrompt) {
    setForceRemovalPrompt(null);
    if (prompt.kind === "profile") {
      void removeProfile(prompt.profile, true);
      return;
    }
    if (prompt.source === "discovery") {
      void removeDiscoverInstalledMod(prompt.mod, true);
      return;
    }
    void handleModAction("remove", prompt.mod, true);
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
  const forceRemoval = forceRemovalPresence.value;
  const dependencyPrompt = dependencyPromptPresence.value;

  function startTitlebarDrag(event: ReactMouseEvent<HTMLElement>) {
    if (event.button !== 0 || event.defaultPrevented) {
      return;
    }

    const target = event.target as HTMLElement;
    if (
      target.closest(
        "button, a, input, select, textarea, label, [role='button'], [contenteditable='true'], [data-no-window-drag]"
      )
    ) {
      return;
    }

    event.preventDefault();
    void getCurrentWindow().startDragging().catch(() => undefined);
  }

  return (
    <>
    {startupSplashPhase !== "hidden" ? <StartupSplash phase={startupSplashPhase} /> : null}
    <main
      className={renderedView === "manager" ? "app-shell" : "app-shell settings-shell"}
      data-theme={defaultThemeId}
    >
      <header className="vault-titlebar" onMouseDown={startTitlebarDrag}>
        <div className="vault-brand">
          <span className="vault-brand-mark"><UniLoaderMark /></span>
          <strong>UniLoader</strong>
        </div>
        <nav className="vault-primary-nav" aria-label="Primary navigation">
          <button
            className={activeView === "manager" ? "active" : ""}
            onClick={() => setActiveView("manager")}
            type="button"
          >
            <Library size={16} />
            Library
          </button>
          <button
            className={activeView === "discover" ? "active" : ""}
            onClick={() => setActiveView("discover")}
            type="button"
          >
            <Compass size={16} />
            Discover
          </button>
        </nav>
        <div className="vault-titlebar-actions">
          <button
            aria-label="Settings"
            className={activeView === "settings" ? "vault-settings-button active" : "vault-settings-button"}
            onClick={() => setActiveView("settings")}
            title="Settings"
            type="button"
          >
            <Settings2 size={18} />
          </button>
          <WindowControls
            onClose={() => void api.closeWindow()}
            onMaximize={() => void api.toggleMaximizeWindow()}
            onMinimize={() => void api.minimizeWindow()}
          />
        </div>
      </header>
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
              const profileStatus = profile.setupStatus === "setting-up"
                ? "Setting up"
                : profile.setupStatus === "failed" || profile.setupStatus === "needs-action"
                  ? "Needs attention"
                  : profile.engine === "unknown"
                    ? "Game not identified"
                    : profile.loader === "none" && profile.engine !== "unreal"
                      ? "Loader not detected"
                      : "Ready";
              return (
                <div
                  className={`profile${isSelected ? " active" : ""}${isExpanded ? " expanded" : ""}`}
                  key={profile.id}
                >
                  <div className="profile-summary">
                    <button
                      aria-pressed={isSelected}
                      className="profile-select"
                      onClick={() => {
                        if (!isSelected) {
                          selectProfile(profile.id);
                          setExpandedProfileId("");
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
                    </button>
                    <button
                      aria-expanded={isExpanded}
                      aria-label={`${isExpanded ? "Hide" : "Show"} controls for ${profile.name}`}
                      className="profile-expand-button"
                      onClick={() => {
                        if (!isSelected) {
                          selectProfile(profile.id);
                          setAnalysis(null);
                          setNotice(null);
                        }
                        setExpandedProfileId((current) =>
                          current === profile.id ? "" : profile.id
                        );
                      }}
                      title={isExpanded ? "Hide profile controls" : "Show profile controls"}
                      type="button"
                    >
                      <ChevronDown aria-hidden="true" size={18} />
                    </button>
                  </div>
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

        <section className="profile-command-panel vault-game-hero">
          <GameHeroArtwork profile={selectedProfile} />
          <div className="vault-hero-overlay" />
          <div className="vault-hero-title">
            <p className="eyebrow">Active game</p>
            <h1>{selectedProfile?.name ?? "Select a Steam game"}</h1>
          </div>
          <div className="vault-hero-metadata">
            <div>
              <Gamepad2 size={20} />
              <span><small>Steam ID</small><strong>{selectedProfile?.steamAppId ?? "Not connected"}</strong></span>
            </div>
            <div title={selectedProfile?.gamePath}>
              <FolderOpen size={20} />
              <span><small>Folder</small><strong>{selectedProfileFolderConnected ? "Connected" : "Unavailable"}</strong></span>
            </div>
            <div>
              <LockKeyhole size={20} />
              <span><small>Runtime</small><strong>{selectedProfile ? loaderDisplayName(selectedProfile.loader, selectedProfile.engine) : "Waiting"}</strong></span>
            </div>
            <div className={`vault-hero-health ${healthTone}`}>
              <Heart size={20} />
              <span><small>Health</small><strong>{healthMessage}</strong></span>
            </div>
          </div>
          <div className="profile-command-actions vault-hero-actions">
            <label
              className="master-mod-toggle"
              title={
                selectedProfileLaunchModsEnabled
                  ? "Launch with the individual mod choices from the library"
                  : "Launch without user mods while preserving the library choices"
              }
            >
              <input
                checked={selectedProfileLaunchModsEnabled}
                disabled={!selectedProfile || gameLaunchBusy || isChangingProfileLaunchMode}
                onChange={(event) => void setProfileLaunchModsEnabled(event.currentTarget.checked)}
                type="checkbox"
              />
              <span className="master-mod-track"><span /></span>
              <span className="master-mod-copy"><strong>Mods {profileModToggleLabel}</strong></span>
            </label>
            <button
              className="secondary-button vault-hero-refresh"
              disabled={!selectedProfile || isRefreshing}
              onClick={() => void refreshSelectedProfile()}
              title="Rescan selected profile"
              type="button"
            >
              <RefreshCw className={isRefreshing ? "spin-icon" : ""} size={16} />
              Refresh
            </button>
            <button
              aria-live="polite"
              className={`primary-button vault-launch-button ${
                gameLaunchState === "running"
                  ? "is-running"
                  : gameLaunchState === "requesting" || gameLaunchState === "waiting"
                    ? "is-pending"
                    : ""
              }`}
              disabled={!selectedProfile || !selectedProfile.steamAppId || gameLaunchBusy}
              onClick={() => void launchSelectedProfileGame()}
              title={
                gameLaunchState === "running"
                  ? `${selectedProfile?.name ?? "Game"} is running`
                  : gameLaunchState === "requesting" || gameLaunchState === "waiting"
                    ? `Waiting for ${selectedProfile?.name ?? "the game"} to start`
                    : "Launch selected game through Steam"
              }
              type="button"
            >
              {gameLaunchState === "running" ? (
                <CheckCircle2 size={17} />
              ) : gameLaunchState === "requesting" || gameLaunchState === "waiting" ? (
                <RefreshCw className="spin-icon" size={17} />
              ) : (
                <Play size={17} />
              )}
              {gameLaunchState === "running"
                ? "Launched"
                : gameLaunchState === "requesting" || gameLaunchState === "waiting"
                  ? "Launching"
                  : "Launch Game"}
            </button>
          </div>
        </section>

          <div className="main-grid">
            <section className="work-panel analysis-panel">
              <div className="vault-insight-stack">
                <section className="vault-insight-section" aria-label="Dependency checks">
                  <div className="vault-insight-heading">
                    <div>
                      <p className="eyebrow">Dependency checks</p>
                      <strong>Requirements</strong>
                    </div>
                    <span>{dependencyChecks.length}</span>
                  </div>
                  <div className="vault-insight-list">
                    {dependencyChecks.length > 0 ? (
                      dependencyChecks.slice(0, 3).map((dependency) => (
                        <div
                          className={dependency.satisfied ? "vault-insight-item success" : "vault-insight-item warning"}
                          key={dependency.id}
                        >
                          {dependency.satisfied ? <CheckCircle2 size={14} /> : <AlertTriangle size={14} />}
                          <span title={dependency.label}>{dependency.label}</span>
                          <small>{dependency.version ? `v${dependency.version}` : dependency.satisfied ? "OK" : "Missing"}</small>
                        </div>
                      ))
                    ) : (
                      <div className="vault-insight-item success empty">
                        <CheckCircle2 size={14} />
                        <span>No missing requirements</span>
                        <small>OK</small>
                      </div>
                    )}
                    {dependencyChecks.length > 3 ? (
                      <small className="vault-insight-more">+{dependencyChecks.length - 3} more checked</small>
                    ) : null}
                  </div>
                </section>

                <section className="vault-insight-section" aria-label="Recent actions">
                  <div className="vault-insight-heading">
                    <div>
                      <p className="eyebrow">Recent actions</p>
                      <strong>Session activity</strong>
                    </div>
                    <Activity size={16} />
                  </div>
                  <div className="vault-insight-list">
                    {recentActivities.length > 0 ? (
                      recentActivities.slice(0, 4).map((activity) => (
                        <div className={`vault-activity-item ${activity.kind}`} key={activity.id}>
                          {activity.kind === "success" ? <CheckCircle2 size={14} /> : <AlertTriangle size={14} />}
                          <span title={activity.label}>{activity.label}</span>
                          <time>{activity.time}</time>
                        </div>
                      ))
                    ) : (
                      <div className="vault-activity-item idle">
                        <Activity size={14} />
                        <span>No recent actions</span>
                      </div>
                    )}
                  </div>
                </section>
              </div>

              <div className="panel-title-row">
                <div>
                  <p className="eyebrow">Install vault</p>
                  <h3>Import a Mod</h3>
                  <small className="vault-panel-note">Drag a mod or folder into the vault.</small>
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
                  <p className="eyebrow">Library</p>
                  <h3>Mods ({installedMods.length})</h3>
                </div>
                <div className="library-toolbar">
                  <label className="vault-mod-search">
                    <Search size={15} />
                    <input
                      aria-label="Search installed mods"
                      onChange={(event) => setInstalledModQuery(event.target.value)}
                      placeholder="Search mods"
                      value={installedModQuery}
                    />
                  </label>
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
                    expanded={expandedInstalledModId === mod.id}
                    key={mod.id}
                    mod={mod}
                    presentation={getStoredModPresentation(mod, modPresentations)}
                    onConfigure={() => void openModConfig(mod)}
                    onEnable={() => void handleModAction("enable", mod)}
                    onDisable={() => void handleModAction("disable", mod)}
                    onRemove={() => void handleModAction("remove", mod)}
                    onToggleDetails={() =>
                      setExpandedInstalledModId((current) => current === mod.id ? "" : mod.id)
                    }
                  />
                ))}
                {installedMods.length === 0 ? (
                  <div className="empty-mods">
                    <SlidersHorizontal size={22} />
                    <p>No mods installed yet.</p>
                  </div>
                ) : sortedInstalledMods.length === 0 ? (
                  <div className="empty-mods">
                    <Search size={22} />
                    <p>No installed mods match this search.</p>
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
            installCompletionId={onlineInstallCompletionId}
            installedMods={
              discoverInstalledModsProfileId === discoverProfileId ? discoverInstalledMods : []
            }
            installingModId={installingOnlineModId}
            isLoadingInstalledMods={
              isLoadingDiscoverInstalledMods ||
              Boolean(discoverProfileId && discoverInstalledModsProfileId !== discoverProfileId)
            }
            isLoading={isDiscoveringMods}
            mods={displayedOnlineMods}
            modPresentations={modPresentations}
            removingModId={removingDiscoverModId}
            total={discoverLoadedProfileId === discoverProfileId ? onlineModTotal : 0}
            profiles={profiles}
            selectedProfileId={discoverProfileId}
            onInstall={(mod, file) => void installOnlineMod(mod, file)}
            onLoadFiles={loadOnlineModFiles}
            onNeedsAuth={openNexusAuthSettings}
            onOpenPage={(url) => void openExternalUrl(url)}
            onLoad={(page, sort, query) => loadOnlineMods(discoverProfileId, page, sort, query)}
            onRemoveInstalledMod={(mod) => void removeDiscoverInstalledMod(mod)}
            onSelectProfile={selectDiscoverProfile}
          />
        ) : (
          <SettingsView
            appSettings={appSettings}
            nexusAttentionId={nexusSettingsAttentionId}
            onOpenDiagnostics={() => void openDiagnosticsFolder()}
            onOpenExternalUrl={(url) => void openExternalUrl(url)}
            onSaveNexusApiKey={saveNexusApiKey}
            onUpdateSettings={updateAppSetting}
          />
        )}
      </section>
      <footer className="vault-statusbar">
        <UpdateRailIndicator
          isChecking={isCheckingForUpdate}
          isDownloading={isDownloadingUpdate}
          updateInfo={updateInfo}
          onClick={() => void showUpdateDetails()}
        />
        <button
          aria-label="Support Me"
          className="vault-support-button"
          onClick={() => void openSupportPage()}
          type="button"
        >
          <Heart size={19} />
          <span>Support Me</span>
        </button>
        <div className="vault-version" title={`UniLoader ${appVersion || "loading"}`}>
          <span className="rail-status" />
          {appVersion ? `v${appVersion}` : ""}
        </div>
        <HealthPanel
          healthMessage={healthMessage}
          healthTone={healthTone}
          motionClassName={viewMotion.className}
          status={status}
        />
        <button
          className={appSettings.nexusApiKeyConfigured ? "vault-footer-service connected" : "vault-footer-service"}
          onClick={() => setActiveView("settings")}
          type="button"
        >
          <Compass className="vault-nexus-compass" size={18} />
          <span><small>Nexus Mods</small><strong>{appSettings.nexusApiKeyConfigured ? "Connected" : "Connect account"}</strong></span>
        </button>
        <button
          className={activeView === "transfer" ? "vault-footer-service active" : "vault-footer-service"}
          onClick={() => setActiveView("transfer")}
          type="button"
        >
          <Upload size={18} />
          <span><small>Profiles</small><strong>Import / Export</strong></span>
        </button>
      </footer>
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
    {forceRemoval ? (
      <ForceRemovalDialog
        detail={forceRemoval.detail}
        itemName={
          forceRemoval.kind === "profile"
            ? forceRemoval.profile.name
            : displayModName(forceRemoval.mod)
        }
        itemType={forceRemoval.kind}
        motionClassName={forceRemovalPresence.className}
        onCancel={() => setForceRemovalPrompt(null)}
        onConfirm={() => confirmForceRemoval(forceRemoval)}
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
                  <SteamGameArtwork game={game} />
                  <span className="steam-creator-game-copy">
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
            This removes every UniLoader-managed mod, dependency, and runtime, then restores any
            game files they replaced. Components that existed before UniLoader are preserved.
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

interface ForceRemovalDialogProps {
  detail: string;
  itemName: string;
  itemType: "profile" | "mod";
  motionClassName: string;
  onCancel(): void;
  onConfirm(): void;
}

function ForceRemovalDialog({
  detail,
  itemName,
  itemType,
  motionClassName,
  onCancel,
  onConfirm
}: ForceRemovalDialogProps) {
  return (
    <div className={`modal-backdrop ${motionClassName}`} onMouseDown={onCancel}>
      <section
        aria-label={`Forget ${itemName} anyway`}
        className="confirm-modal"
        onMouseDown={(event) => event.stopPropagation()}
        role="dialog"
      >
        <div className="confirm-icon">
          <AlertTriangle size={22} />
        </div>
        <div className="confirm-copy">
          <p className="eyebrow">Modified Files Found</p>
          <h3>Forget {itemType === "profile" ? "profile" : "mod"} anyway?</h3>
          <p>
            UniLoader will remove every file that still matches its installation receipt, leave
            modified or unverified files untouched, and forget {itemName}.
          </p>
          <p>{detail}</p>
        </div>
        <div className="confirm-actions">
          <button className="secondary-button compact-button" onClick={onCancel} type="button">
            Cancel
          </button>
          <button className="danger-button compact-button" onClick={onConfirm} type="button">
            Forget Anyway
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
  const initials =
    profile.name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0]?.toUpperCase())
      .join("") || "G";

  return (
    <span className="profile-artwork" aria-hidden="true">
      <SteamArtworkImage
        cacheLocally
        fallback={
          <span className="profile-artwork-fallback">
            <Gamepad2 size={19} />
            <b>{initials}</b>
          </span>
        }
        steamAppId={profile.steamAppId}
        variant="poster"
      />
    </span>
  );
}

function GameHeroArtwork({ profile }: { profile?: GameProfile }) {
  return (
    <div className="vault-hero-artwork" aria-hidden="true">
      {profile ? (
        <SteamArtworkImage
          cacheLocally
          fallback={<div className="vault-hero-fallback"><Gamepad2 size={54} /></div>}
          key={`${profile.steamAppId ?? profile.id}:hero`}
          steamAppId={profile.steamAppId}
          variant="hero"
        />
      ) : (
        <div className="vault-hero-fallback"><Gamepad2 size={54} /></div>
      )}
    </div>
  );
}

interface SteamArtworkImageProps {
  cacheLocally?: boolean;
  fallback: ReactNode;
  steamAppId?: string;
  variant: SteamArtworkVariant;
}

function SteamArtworkImage({
  cacheLocally = false,
  fallback,
  steamAppId: rawSteamAppId,
  variant
}: SteamArtworkImageProps) {
  const [imageSourceIndex, setImageSourceIndex] = useState(0);
  const steamAppId = rawSteamAppId?.trim();
  const cacheKey = steamAppId && cacheLocally
    ? gameArtworkCacheKey(steamAppId, variant)
    : undefined;
  const rememberedSource = cacheKey ? gameArtworkSourceCache.get(cacheKey) : undefined;
  const [cachedImageSource, setCachedImageSource] = useState<string | undefined>(rememberedSource);
  const [cacheLookupComplete, setCacheLookupComplete] = useState(
    !cacheKey || Boolean(rememberedSource)
  );
  const imageUrls = steamArtworkUrls(steamAppId, variant);
  const imageUrl = cachedImageSource ?? (cacheLookupComplete ? imageUrls[imageSourceIndex] : undefined);

  useEffect(() => {
    setImageSourceIndex(0);

    if (!cacheKey || !steamAppId) {
      setCachedImageSource(undefined);
      setCacheLookupComplete(true);
      return;
    }

    const remembered = gameArtworkSourceCache.get(cacheKey);
    if (remembered) {
      setCachedImageSource(remembered);
      setCacheLookupComplete(true);
      return;
    }

    let cancelled = false;
    setCachedImageSource(undefined);
    setCacheLookupComplete(false);
    void loadCachedGameArtwork(steamAppId, variant)
      .then((source) => {
        if (!cancelled) {
          setCachedImageSource(source ?? undefined);
          setCacheLookupComplete(true);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setCacheLookupComplete(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [cacheKey, steamAppId, variant]);

  return (
    <>
      {fallback}
      {imageUrl ? (
        <img
          alt=""
          className="steam-artwork-image"
          decoding="async"
          draggable={false}
          loading={cacheLocally || variant === "hero" ? "eager" : "lazy"}
          onError={() => {
            if (cachedImageSource) {
              if (cacheKey) {
                gameArtworkSourceCache.delete(cacheKey);
              }
              setCachedImageSource(undefined);
              setCacheLookupComplete(true);
              return;
            }
            setImageSourceIndex((current) => current + 1);
          }}
          onLoad={() => {
            if (cacheKey) {
              gameArtworkSourceCache.set(cacheKey, imageUrl);
            }
          }}
          src={imageUrl}
        />
      ) : null}
    </>
  );
}

function SteamGameArtwork({ game }: { game: SteamGameRecord }) {
  const initials =
    game.name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0]?.toUpperCase())
      .join("") || "G";

  return (
    <span className="steam-creator-game-artwork" aria-hidden="true">
      <SteamArtworkImage
        fallback={
          <span className="steam-creator-game-fallback">
            <Gamepad2 size={18} />
            <b>{initials}</b>
          </span>
        }
        steamAppId={game.appId}
        variant="poster"
      />
    </span>
  );
}

function steamArtworkUrls(
  steamAppId: string | undefined,
  variant: "hero" | "poster"
): string[] {
  if (!steamAppId || !/^\d+$/.test(steamAppId)) {
    return [];
  }

  if (variant === "hero") {
    return steamWideArtworkUrls(steamAppId);
  }

  return [
    `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900_2x.jpg`,
    `https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900_2x.jpg`,
    `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900.jpg`,
    `https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_600x900.jpg`,
    `https://cdn.cloudflare.steamstatic.com/steam/apps/${steamAppId}/capsule_231x87.jpg`,
    ...steamWideArtworkUrls(steamAppId)
  ];
}

function steamWideArtworkUrls(steamAppId: string): string[] {
  return [
    `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_hero.jpg`,
    `https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/library_hero.jpg`,
    `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${steamAppId}/header.jpg`,
    `https://cdn.cloudflare.steamstatic.com/steam/apps/${steamAppId}/header.jpg`
  ];
}

function ModArtwork({
  mod,
  name,
  presentation
}: {
  mod: InstalledModRecord;
  name: string;
  presentation: StoredModPresentation;
}) {
  const [imageFailed, setImageFailed] = useState(false);
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");

  useEffect(() => {
    setImageFailed(false);
  }, [presentation.iconUrl]);

  return (
    <div className={`vault-mod-artwork adapter-${mod.adapterId}`} aria-hidden="true">
      {presentation.iconUrl && !imageFailed ? (
        <img
          alt=""
          decoding="async"
          draggable={false}
          loading="lazy"
          onError={() => setImageFailed(true)}
          src={presentation.iconUrl}
        />
      ) : (
        <span>
          {mod.runtimeId ? <LockKeyhole size={24} /> : <PackagePlus size={24} />}
          <b>{initials || "MOD"}</b>
        </span>
      )}
    </div>
  );
}

function loaderDisplayName(loader: GameProfile["loader"], engine: GameProfile["engine"]): string {
  const loaderLabels: Record<GameProfile["loader"], string> = {
    none: engine === "unknown" ? "Detecting" : `${humanizeModName(engine)} native`,
    bepinex: "BepInEx",
    "bepinex-il2cpp": "BepInEx IL2CPP",
    ue4ss: "UE4SS",
    reframework: "REFramework",
    "loose-files": "Native files"
  };
  return loaderLabels[loader];
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
  motionClassName?: string;
  status: string;
}

function HealthPanel({
  healthMessage,
  healthTone,
  motionClassName = "",
  status
}: HealthPanelProps) {
  return (
    <div className={`health-panel health-dock ${healthTone} ${motionClassName}`}>
      <div className="health-orb">
        <Activity className="health-wave-track" size={22} strokeWidth={2.35} />
        <Activity className="health-wave-signal" size={22} strokeWidth={2.35} />
      </div>
      <div className="health-copy">
        <span>Health</span>
        <strong>{healthMessage}</strong>
      </div>
      <span className="health-status" title={status}>{status}</span>
    </div>
  );
}

function UniLoaderMark() {
  return (
    <svg aria-hidden="true" viewBox="0 0 48 48">
      <path
        d="M24 3 41 10v14c0 10.2-6.2 16.7-17 21C13.2 40.7 7 34.2 7 24V10L24 3Z"
        fill="currentColor"
        fillOpacity="0.13"
        stroke="currentColor"
        strokeWidth="1.7"
      />
      <path
        d="M13 12h7v14.2c0 3.7 1.4 6 4 7.4 2.6-1.4 4-3.7 4-7.4V12h7v14.2c0 7.6-4.4 12.1-11 14.9-6.6-2.8-11-7.3-11-14.9V12Z"
        fill="currentColor"
      />
      <path d="m20 12 4-4 4 4-4 4-4-4Z" fill="#fff" fillOpacity="0.5" />
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
  installCompletionId: number;
  installedMods: InstalledModRecord[];
  installingModId: string;
  isLoadingInstalledMods: boolean;
  isLoading: boolean;
  mods: OnlineModRecord[];
  modPresentations: StoredModPresentations;
  removingModId: string;
  total: number;
  profiles: GameProfile[];
  selectedProfileId: string;
  onInstall(mod: OnlineModRecord, file?: OnlineModFileOption): void;
  onLoadFiles(mod: OnlineModRecord): Promise<OnlineModFileOption[]>;
  onNeedsAuth(): void;
  onOpenPage(url: string): void;
  onLoad(page: number, sort: OnlineSortMode, query: string): Promise<boolean>;
  onRemoveInstalledMod(mod: InstalledModRecord): void;
  onSelectProfile(profileId: string): void;
}

function DiscoverView({
  hasLoaded,
  installCompletionId,
  installedMods,
  installingModId,
  isLoadingInstalledMods,
  isLoading,
  mods,
  modPresentations,
  removingModId,
  total,
  profiles,
  selectedProfileId,
  onInstall,
  onLoadFiles,
  onNeedsAuth,
  onOpenPage,
  onLoad,
  onRemoveInstalledMod,
  onSelectProfile
}: DiscoverViewProps) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(1);
  const [sortMode, setSortMode] = useState<OnlineSortMode>("downloads");
  const [expandedModId, setExpandedModId] = useState("");
  const onLoadRef = useRef(onLoad);
  const sortModeRef = useRef(sortMode);
  const selectedProfile = profiles.find((profile) => profile.id === selectedProfileId);
  const pageCount = Math.max(1, Math.ceil(total / discoverPageSize));
  const currentPage = Math.min(page, pageCount);
  const visibleMods = mods;

  useEffect(() => {
    onLoadRef.current = onLoad;
  }, [onLoad]);

  useEffect(() => {
    sortModeRef.current = sortMode;
  }, [sortMode]);

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
    if (installCompletionId > 0) {
      setExpandedModId("");
    }
  }, [installCompletionId]);

  useEffect(() => {
    if (!hasLoaded || !selectedProfileId) {
      return;
    }
    const timeout = window.setTimeout(() => {
      setPage(1);
      void onLoadRef.current(1, sortModeRef.current, query.trim());
    }, 300);
    return () => window.clearTimeout(timeout);
  }, [hasLoaded, query, selectedProfileId]);

  async function changeSort(nextSort: OnlineSortMode) {
    setExpandedModId("");
    if (await onLoad(1, nextSort, query.trim())) {
      setSortMode(nextSort);
      setPage(1);
    }
  }

  async function changePage(nextPage: number) {
    const safePage = Math.max(1, Math.min(pageCount, nextPage));
    setExpandedModId("");
    if (await onLoad(safePage, sortMode, query.trim())) {
      setPage(safePage);
    }
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
            void onLoad(currentPage, sortMode, query.trim());
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
            onClick={() => void changeSort("downloads")}
            type="button"
          >
            Total Downloads
          </button>
          <button
            className={sortMode === "newest" ? "active" : ""}
            onClick={() => void changeSort("newest")}
            type="button"
          >
            Newest
          </button>
          <button
            className={sortMode === "oldest" ? "active" : ""}
            onClick={() => void changeSort("oldest")}
            type="button"
          >
            Oldest
          </button>
        </div>
      </section>

      <div className="discover-content-grid">
        <section className="discover-results" aria-label="Online mod results">
          {total > discoverPageSize ? (
            <DiscoveryPagination
              currentPage={currentPage}
              isLoading={isLoading}
              pageCount={pageCount}
              placement="top"
              onChange={changePage}
            />
          ) : null}
          {visibleMods.map((mod) => {
            const installedMod = installedMods.find((item) =>
              installedRecordMatchesOnlineMod(item, mod)
            );
            const displayedMod = installedMod && !mod.installed ? { ...mod, installed: true } : mod;
            return (
              <OnlineModCard
                expanded={expandedModId === mod.id}
                installing={installingModId === mod.id}
                key={`${mod.provider}:${mod.id}:${mod.version}`}
                mod={displayedMod}
                onInstall={onInstall}
                onLoadFiles={onLoadFiles}
                onNeedsAuth={onNeedsAuth}
                onOpenPage={onOpenPage}
                onToggle={() => setExpandedModId((current) => (current === mod.id ? "" : mod.id))}
              />
            );
          })}
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
          {total > discoverPageSize ? (
            <DiscoveryPagination
              currentPage={currentPage}
              isLoading={isLoading}
              pageCount={pageCount}
              placement="bottom"
              onChange={changePage}
            />
          ) : null}
        </section>

        <DiscoveryInstalledRail
          isLoading={isLoadingInstalledMods}
          mods={installedMods}
          presentations={modPresentations}
          removingModId={removingModId}
          onRemove={onRemoveInstalledMod}
        />
      </div>
    </div>
  );
}

interface DiscoveryInstalledRailProps {
  isLoading: boolean;
  mods: InstalledModRecord[];
  presentations: StoredModPresentations;
  removingModId: string;
  onRemove(mod: InstalledModRecord): void;
}

function DiscoveryInstalledRail({
  isLoading,
  mods,
  presentations,
  removingModId,
  onRemove
}: DiscoveryInstalledRailProps) {
  const orderedMods = [...mods].sort((first, second) => {
    if (Boolean(first.runtimeId) !== Boolean(second.runtimeId)) {
      return first.runtimeId ? 1 : -1;
    }
    return (Date.parse(second.installedAt) || 0) - (Date.parse(first.installedAt) || 0);
  });

  return (
    <aside className="discover-installed-rail" aria-label="Currently installed mods">
      <div className="discover-installed-header">
        <div>
          <p className="eyebrow">Installed</p>
          <h3>Current Mods</h3>
        </div>
        <span aria-live="polite">{mods.length}</span>
      </div>

      <div className="discover-installed-list">
        {orderedMods.map((mod) => {
          const modName = displayModName(mod);
          const presentation = getStoredModPresentation(mod, presentations);
          const isRuntime = Boolean(mod.runtimeId);
          const isRemoving = removingModId === mod.id;

          return (
            <article
              className={`discover-installed-item${isRuntime ? " runtime" : ""}`}
              key={mod.id}
            >
              <ModArtwork mod={mod} name={modName} presentation={presentation} />
              <div className="discover-installed-copy">
                <strong title={modName}>{modName}</strong>
                <span>{isRuntime ? "System runtime" : presentation.providerLabel ?? adapterDisplayName(mod.adapterId)}</span>
              </div>
              {isRuntime ? (
                <span
                  className="discover-installed-protected"
                  title="Required runtime"
                >
                  <ShieldCheck size={16} />
                </span>
              ) : (
                <button
                  aria-label={`Remove ${modName}`}
                  className="discover-installed-remove"
                  disabled={Boolean(removingModId)}
                  onClick={() => onRemove(mod)}
                  title={`Remove ${modName}`}
                  type="button"
                >
                  {isRemoving ? <RefreshCw className="spin-icon" size={15} /> : <X size={16} />}
                </button>
              )}
            </article>
          );
        })}

        {isLoading && mods.length === 0 ? (
          <div className="discover-installed-empty">
            <RefreshCw className="spin-icon" size={18} />
            <span>Loading</span>
          </div>
        ) : !isLoading && mods.length === 0 ? (
          <div className="discover-installed-empty">
            <PackagePlus size={19} />
            <span>No installed mods</span>
          </div>
        ) : null}
      </div>
    </aside>
  );
}

interface DiscoveryPaginationProps {
  currentPage: number;
  isLoading: boolean;
  pageCount: number;
  placement: "top" | "bottom";
  onChange(page: number): Promise<void>;
}

function DiscoveryPagination({
  currentPage,
  isLoading,
  pageCount,
  placement,
  onChange
}: DiscoveryPaginationProps) {
  return (
    <div className={`discover-pagination discover-pagination-${placement}`}>
      <button
        className="secondary-button compact-button"
        disabled={isLoading || currentPage <= 1}
        onClick={() => void onChange(currentPage - 1)}
        type="button"
      >
        Previous
      </button>
      <span aria-live="polite">
        {isLoading ? (
          <>
            <RefreshCw className="spin-icon" size={13} />
            Loading
          </>
        ) : (
          <>Page {currentPage} / {pageCount}</>
        )}
      </span>
      <button
        className="secondary-button compact-button"
        disabled={isLoading || currentPage >= pageCount}
        onClick={() => void onChange(currentPage + 1)}
        type="button"
      >
        Next
      </button>
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

  const modeTitle =
    mode === "import"
      ? "Restore Profile Bundle"
      : "Create Profile Bundle";
  const modeLabel = mode === "import" ? "Import" : "Export";

  return (
    <div className="transfer-layout">
      <div className="transfer-hero">
        <p className="eyebrow">Profile transfer</p>
        <h2>Import / Export</h2>
        <span>Move complete profiles between PCs using portable UniLoader bundle files.</span>
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
              <p className="eyebrow">{modeLabel}</p>
              <h3>{modeTitle}</h3>
            </div>
            {mode === "import" ? (
              <Download size={20} />
            ) : (
              <Upload size={20} />
            )}
          </div>

          {mode === "import" ? (
            <>
              <div className="transfer-steps">
                <span>1. Choose a `.uniloader-profile` bundle.</span>
                <span>2. UniLoader finds and verifies the matching installed Steam game.</span>
                <span>3. The profile, enabled mods, and configuration files are restored.</span>
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
  onOpenDiagnostics(): void;
  onOpenExternalUrl(url: string): void;
  onSaveNexusApiKey(apiKey: string): Promise<void>;
  onUpdateSettings(settings: AppSettings): Promise<void>;
}

function SettingsView({
  appSettings,
  nexusAttentionId,
  onOpenDiagnostics,
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

      <section className="work-panel settings-panel">
        <div className="panel-title-row">
          <div>
            <p className="eyebrow">Support</p>
            <h3>Diagnostics</h3>
          </div>
          <Activity size={20} />
        </div>
        <p className="muted">
          UniLoader keeps a small rotating operation log without API keys or mod contents.
        </p>
        <div className="settings-key-actions">
          <button className="secondary-button" onClick={onOpenDiagnostics} type="button">
            <FolderOpen size={16} />
            Open Diagnostics
          </button>
        </div>
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

function nexusModIdFromNxmUrl(rawUrl: string): string | null {
  try {
    const parsed = new URL(rawUrl);
    if (parsed.protocol.toLowerCase() !== "nxm:") {
      return null;
    }
    const parts = parsed.pathname.split("/").filter(Boolean);
    const modsIndex = parts.findIndex((part) => part.toLowerCase() === "mods");
    const modId = modsIndex >= 0 ? parts[modsIndex + 1] : undefined;
    return modId && /^\d+$/.test(modId)
      ? `nexus:${parsed.hostname.toLowerCase()}/${modId}`
      : null;
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
  const suspendedFileCount = result.modFileHealth.reduce(
    (total, health) => total + (health.suspendedFiles?.length ?? 0),
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

  for (const update of result.runtimeUpdates ?? []) {
    parts.push(
      update.previousVersion
        ? `Updated ${update.name} from ${update.previousVersion} to ${update.installedVersion}.`
        : `Updated ${update.name} to ${update.installedVersion}.`
    );
  }

  parts.push(...(result.runtimeUpdateNotes ?? []));

  if (missingFileCount > 0) {
    parts.push(`${missingFileCount} expected installed file(s) are missing.`);
  }

  if (suspendedFileCount > 0) {
    parts.push(
      `${suspendedFileCount} mod file(s) are safely suspended for a clean launch.`
    );
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

function actionableDependencyWarnings(warnings: string[]): string[] {
  return warnings.filter((warning) => !warning.startsWith("Installed dependency "));
}

function loadStoredModPresentations(): StoredModPresentations {
  try {
    const raw = window.localStorage.getItem(modPresentationStorageKey);
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? pruneStoredModPresentations(parsed as StoredModPresentations)
      : {};
  } catch {
    return {};
  }
}

function sameStoredModPresentation(
  first: StoredModPresentation | undefined,
  second: StoredModPresentation
): boolean {
  if (!first) {
    return false;
  }
  return (
    first.description === second.description &&
    first.iconUrl === second.iconUrl &&
    first.name === second.name &&
    first.owner === second.owner &&
    first.providerLabel === second.providerLabel &&
    first.updatedAt === second.updatedAt &&
    first.version === second.version
  );
}

function pruneStoredModPresentations(
  presentations: StoredModPresentations
): StoredModPresentations {
  const entries = Object.entries(presentations);
  if (entries.length <= maxStoredModPresentations) {
    return presentations;
  }

  return Object.fromEntries(
    entries
      .sort(([, first], [, second]) => {
        const firstTime = Date.parse(first.cachedAt || first.updatedAt || "") || 0;
        const secondTime = Date.parse(second.cachedAt || second.updatedAt || "") || 0;
        return secondTime - firstTime;
      })
      .slice(0, maxStoredModPresentations)
  );
}

function saveStoredModPresentations(presentations: StoredModPresentations) {
  try {
    window.localStorage.setItem(modPresentationStorageKey, JSON.stringify(presentations));
  } catch {
    // Artwork metadata is optional; installs must keep working if storage is unavailable.
  }
}

function getStoredModPresentation(
  mod: InstalledModRecord,
  presentations: StoredModPresentations
): StoredModPresentation {
  const persistedIconUrl = mod.iconUrl?.trim() || undefined;
  const packageKey = mod.packageId?.trim().toLowerCase();

  if (mod.runtimeId) {
    return {
      iconUrl: persistedIconUrl,
      owner: runtimeDisplayOwner(mod.runtimeId),
      providerLabel: "System runtime",
      version: dependencyVersion(mod.dependencyString)
    };
  }

  const stored = packageKey ? presentations[packageKey] : undefined;
  if (stored) {
    return {
      ...stored,
      iconUrl: persistedIconUrl ?? stored.iconUrl
    };
  }

  const nameMatch = findStoredPresentationByName(mod, presentations);
  if (nameMatch) {
    return {
      ...nameMatch,
      iconUrl: persistedIconUrl ?? nameMatch.iconUrl
    };
  }

  if (packageKey?.startsWith("thunderstore:")) {
    const reference = packageKey.slice("thunderstore:".length);
    return {
      iconUrl: persistedIconUrl,
      owner: humanizeModName(reference.split("/")[0] ?? "Thunderstore"),
      providerLabel: "Thunderstore",
      version: dependencyVersion(mod.dependencyString)
    };
  }

  if (packageKey?.startsWith("nexus:")) {
    return {
      iconUrl: persistedIconUrl,
      providerLabel: "Nexus Mods",
      version: dependencyVersion(mod.dependencyString)
    };
  }

  return {
    iconUrl: persistedIconUrl,
    providerLabel: adapterDisplayName(mod.adapterId),
    version: dependencyVersion(mod.dependencyString)
  };
}

function installedRecordMatchesOnlineMod(
  installedMod: InstalledModRecord,
  onlineMod: OnlineModRecord
): boolean {
  const packageId = installedMod.packageId?.trim().toLowerCase();
  return Boolean(packageId && packageId === onlineMod.id.trim().toLowerCase());
}

function findStoredPresentationByName(
  mod: InstalledModRecord,
  presentations: StoredModPresentations
): StoredModPresentation | undefined {
  const installedName = mod.displayName || mod.archiveName;
  const strictKey = modPresentationNameKey(installedName, false);
  const variantKey = modPresentationNameKey(installedName, true);
  if (!strictKey) {
    return undefined;
  }

  const exactMatches = Object.values(presentations).filter(
    (presentation) =>
      presentation.name && modPresentationNameKey(presentation.name, false) === strictKey
  );
  const exactMatch = uniquePresentation(exactMatches);
  if (exactMatch) {
    return exactMatch;
  }

  if (variantKey.length < 6) {
    return undefined;
  }
  const variantMatches = Object.values(presentations).filter(
    (presentation) =>
      presentation.name && modPresentationNameKey(presentation.name, true) === variantKey
  );
  return uniquePresentation(variantMatches);
}

function uniquePresentation(
  matches: StoredModPresentation[]
): StoredModPresentation | undefined {
  if (matches.length === 1) {
    return matches[0];
  }
  if (matches.length === 0) {
    return undefined;
  }

  const iconUrls = new Set(matches.map((match) => match.iconUrl).filter(Boolean));
  return iconUrls.size === 1 ? matches[0] : undefined;
}

function modPresentationNameKey(value: string, stripVariants: boolean): string {
  const tokens = humanizeModName(value)
    .toLowerCase()
    .replace(/\.[a-z0-9]{2,5}$/i, "")
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);

  const normalized = stripVariants
    ? tokens.filter((token) => !isModVariantToken(token))
    : tokens;
  return normalized.join(" ");
}

function isModVariantToken(token: string): boolean {
  return /^(?:v?\d+(?:\.\d+)*|\d+x|x\d+|\d+(?:m|h)|other|minute|minutes|hour|hours|min|mins|hr|hrs)$/i.test(
    token
  );
}

function dependencyVersion(value?: string): string | undefined {
  if (!value) {
    return undefined;
  }
  const match = value.match(/(?:^|[-#@])v?(\d+(?:\.\d+){1,3}(?:[-+][a-z0-9.-]+)?)$/i);
  return match?.[1];
}

function runtimeDisplayOwner(runtimeId: string): string {
  const lower = runtimeId.toLowerCase();
  if (lower.includes("bepinex")) return "BepInEx Team";
  if (lower.includes("ue4ss")) return "UE4SS Team";
  if (lower.includes("reframework")) return "REFramework";
  return "Runtime provider";
}

function adapterDisplayName(adapterId: InstalledModRecord["adapterId"]): string {
  const labels: Record<InstalledModRecord["adapterId"], string> = {
    bepinex: "BepInEx",
    ue4ss: "UE4SS",
    reframework: "REFramework",
    "re-engine-native": "RE Engine",
    "unreal-pak": "Unreal Pak",
    "loose-files": "Loose files",
    "script-files": "Script mod"
  };
  return labels[adapterId];
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
  expanded: boolean;
  mod: InstalledModRecord;
  presentation: StoredModPresentation;
  onConfigure(): void;
  onEnable(): void;
  onDisable(): void;
  onRemove(): void;
  onToggleDetails(): void;
}

interface InstalledModTarget {
  id: string;
  label: string;
  path: string;
}

function normalizedInstallPath(value: string): string {
  return value.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

function installTargetDirectory(value: string): string {
  const normalized = normalizedInstallPath(value);
  const separator = normalized.lastIndexOf("/");
  return separator > 0 ? normalized.slice(0, separator) : normalized;
}

function installTargetLabel(path: string): string {
  const normalized = `/${normalizedInstallPath(path).toLowerCase()}/`;
  if (normalized.includes("/builds/windowsserver/") || normalized.includes("/windowsserver/")) {
    return "Local Server Mods";
  }
  if (normalized.includes("/dedicated") || normalized.includes("dedicated/")) {
    return "Dedicated Server Mods";
  }
  if (normalized.includes("/engine/binaries/")) {
    return "Engine Mods";
  }
  if (normalized.includes("/ue4ss/mods/")) {
    return "Game Mods (UE4SS)";
  }
  return "Game Mods";
}

function installedModTargets(mod: InstalledModRecord): InstalledModTarget[] {
  if (mod.runtimeId || !mod.plan?.mappings.length) {
    return [];
  }

  const targetsBySource = new Map<string, Set<string>>();
  for (const mapping of mod.plan.mappings) {
    const source = normalizedInstallPath(mapping.sourcePath).toLowerCase();
    const directory = installTargetDirectory(mapping.targetRelativePath);
    if (!source || !directory) {
      continue;
    }
    const targets = targetsBySource.get(source) ?? new Set<string>();
    targets.add(directory);
    targetsBySource.set(source, targets);
  }

  const alternativePaths = new Set<string>();
  for (const targets of targetsBySource.values()) {
    if (targets.size > 1) {
      targets.forEach((target) => alternativePaths.add(target));
    }
  }

  const sortedPaths = [...alternativePaths].sort((first, second) => {
    const firstServer = /(?:^|\/)windowsserver(?:\/|$)/i.test(first);
    const secondServer = /(?:^|\/)windowsserver(?:\/|$)/i.test(second);
    if (firstServer !== secondServer) {
      return firstServer ? 1 : -1;
    }
    return first.localeCompare(second);
  });

  // Different route aliases can resolve to the same user-facing destination.
  const targetsByPurpose = new Map<string, InstalledModTarget>();
  for (const path of sortedPaths) {
    const normalizedPath = normalizedInstallPath(path);
    const label = installTargetLabel(normalizedPath);
    const purpose = label.toLowerCase();
    if (!targetsByPurpose.has(purpose)) {
      targetsByPurpose.set(purpose, {
        id: normalizedPath.toLowerCase(),
        label,
        path: normalizedPath
      });
    }
  }

  return [...targetsByPurpose.values()];
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

function ModCard({
  expanded,
  mod,
  presentation,
  onConfigure,
  onEnable,
  onDisable,
  onRemove,
  onToggleDetails
}: ModCardProps) {
  const configFiles = mod.configFiles ?? [];
  const dependencies = mod.dependencies ?? [];
  const modName = displayModName(mod);
  const isRuntime = Boolean(mod.runtimeId);
  const targets = installedModTargets(mod);
  const description =
    presentation.description?.trim() ||
    mod.summary?.trim() ||
    "No description was included with this installed mod.";

  return (
    <article
      className={`mod-card vault-mod-row ${mod.enabled ? "enabled" : "disabled"}${isRuntime ? " runtime" : ""}${expanded ? " expanded" : ""}`}
    >
      <div className="vault-mod-summary">
        <ModArtwork mod={mod} name={modName} presentation={presentation} />
        <div className="vault-mod-identity">
          <strong title={mod.archiveName}>{modName}</strong>
          <div className="vault-mod-byline">
            <span>{presentation.owner ?? "Local package"}</span>
            {presentation.version ? <span>v{presentation.version}</span> : null}
            <span>{presentation.providerLabel ?? adapterDisplayName(mod.adapterId)}</span>
          </div>
          <small title={mod.summary}>
            {isRuntime
              ? mod.externallyManaged ? "Detected runtime" : "Managed runtime"
              : dependencies.length > 0
                ? `${dependencies.length} dependenc${dependencies.length === 1 ? "y" : "ies"} satisfied`
                : `${mod.filesWritten.length} managed file${mod.filesWritten.length === 1 ? "" : "s"}`}
          </small>
        </div>
        <label
          className="vault-mod-toggle"
          title={isRuntime ? "Required runtimes remain enabled" : `${mod.enabled ? "Disable" : "Enable"} ${modName}`}
        >
          <span>{mod.enabled ? "ON" : "OFF"}</span>
          <input
            aria-label={`${mod.enabled ? "Disable" : "Enable"} ${modName}`}
            checked={mod.enabled}
            disabled={isRuntime}
            onChange={(event) => event.currentTarget.checked ? onEnable() : onDisable()}
            type="checkbox"
          />
          <i><b /></i>
        </label>
        <div className="vault-mod-actions">
          {isRuntime ? (
            <span className="vault-mod-protected" title="Required runtimes are protected so installed mods keep working.">
              <ShieldCheck size={17} />
            </span>
          ) : (
            <button aria-label={`Remove ${modName}`} className="vault-mod-remove" onClick={onRemove} title="Remove mod" type="button">
              <X size={18} />
            </button>
          )}
          {configFiles.length > 0 ? (
            <button aria-label={`Configure ${modName}`} className="vault-mod-config" onClick={onConfigure} title="Configure mod" type="button">
              <Settings2 size={18} />
            </button>
          ) : null}
          <button
            aria-expanded={expanded}
            aria-label={`${expanded ? "Hide" : "Show"} details for ${modName}`}
            className="vault-mod-expand"
            onClick={onToggleDetails}
            title={expanded ? "Hide details" : "Show details"}
            type="button"
          >
            {expanded ? <ChevronUp size={18} /> : <ChevronDown size={18} />}
          </button>
        </div>
      </div>

      {expanded ? (
        <div className="vault-mod-details">
          <section className="vault-mod-detail-section">
            <p className="vault-mod-detail-label">Description</p>
            <p className="vault-mod-description">{description}</p>
          </section>

          <section className="vault-mod-detail-section">
            <p className="vault-mod-detail-label">Requirements</p>
            <div className="vault-mod-requirements">
              {dependencies.length > 0 ? dependencies.map((dependency) => {
                const satisfied = dependency.status === "already-installed";
                return (
                  <div
                    className={satisfied ? "vault-mod-requirement satisfied" : "vault-mod-requirement attention"}
                    key={dependency.id}
                  >
                    {satisfied ? <CheckCircle2 size={17} /> : <AlertTriangle size={17} />}
                    <span>
                      <strong>{dependency.name || dependency.id}</strong>
                      <small>
                        {dependency.version ? `v${dependency.version} / ` : ""}
                        {satisfied ? "Installed" : dependency.status === "manual" ? "Manual requirement" : "Needs attention"}
                      </small>
                    </span>
                  </div>
                );
              }) : (
                <div className="vault-mod-requirement satisfied">
                  <CheckCircle2 size={17} />
                  <span>
                    <strong>No additional requirements</strong>
                    <small>Ready</small>
                  </span>
                </div>
              )}
            </div>
          </section>

          {targets.length > 1 ? (
            <section className="vault-mod-detail-section">
              <p className="vault-mod-detail-label">Install targets</p>
              <div className="vault-mod-targets">
                {targets.map((target, index) => (
                  <div className="vault-mod-target" key={target.id}>
                    <CheckCircle2 size={17} />
                    <span>
                      <strong>{target.label}</strong>
                      <small>{target.path}</small>
                    </span>
                    {index === 0 ? <b>Primary</b> : <b>Installed</b>}
                  </div>
                ))}
              </div>
            </section>
          ) : null}
        </div>
      ) : null}
    </article>
  );
}
