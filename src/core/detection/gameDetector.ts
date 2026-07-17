import fs from "node:fs/promises";
import path from "node:path";
import {
  DetectionSignal,
  GameDetectionResult,
  GameEngine,
  LoaderKind
} from "../../shared/contracts";

interface ProbeEntry {
  relativePath: string;
  name: string;
  isDirectory: boolean;
  depth: number;
}

interface RoutePreparation {
  expectedModFolders: string[];
  createdModFolders: string[];
  warnings: string[];
}

type ScoreMap<T extends string> = Record<T, number>;

const maxDepth = 4;
const maxEntries = 3000;

export async function detectGameSetup(gamePath: string): Promise<GameDetectionResult> {
  const rootStat = await fs.stat(gamePath);
  if (!rootStat.isDirectory()) {
    throw new Error("Selected game path must be a folder.");
  }

  const entries = await walkGameFolder(gamePath);
  const gameId = detectGameId(entries);
  const signals: DetectionSignal[] = [];
  const engineScores: ScoreMap<GameEngine> = {
    "unity-mono": 0,
    "unity-il2cpp": 0,
    unreal: 0,
    "re-engine": 0,
    unknown: 0
  };
  const loaderScores: ScoreMap<LoaderKind> = {
    none: 0,
    bepinex: 0,
    "bepinex-il2cpp": 0,
    ue4ss: 0,
    reframework: 0,
    "loose-files": 0
  };

  scoreEngine(entries, engineScores, signals);
  const engine = chooseHighest(engineScores, "unknown");
  scoreLoaders(entries, engine, loaderScores, signals);

  const installedLoader = chooseHighest(loaderScores, "none");
  const recommendedLoader = recommendLoader(engine);
  const loaderInstalled = installedLoader !== "none" && loaderScores[installedLoader] >= 25;
  const loader = loaderInstalled ? installedLoader : recommendedLoader;
  const warnings: string[] = [];

  if (engine === "unknown") {
    warnings.push("Engine could not be identified from this folder.");
  }

  if (!loaderInstalled && recommendedLoader !== "none") {
    warnings.push(`${formatLoader(recommendedLoader)} is recommended but not installed yet.`);
  }

  if (entries.length >= maxEntries) {
    warnings.push("Detection stopped early because the folder contains many files.");
  }

  const routePreparation = await prepareModRoutes(gamePath, gameId, engine, loader, entries);
  warnings.push(...routePreparation.warnings);

  return {
    gamePath,
    gameId,
    engine,
    loader,
    recommendedLoader,
    engineConfidence: confidenceFor(engineScores[engine]),
    loaderConfidence: loaderInstalled ? confidenceFor(loaderScores[installedLoader]) : 0,
    loaderInstalled,
    expectedModFolders: routePreparation.expectedModFolders,
    createdModFolders: routePreparation.createdModFolders,
    signals,
    warnings
  };
}

function detectGameId(entries: ProbeEntry[]): string | undefined {
  if (
    entries.some((entry) => {
      const lowerName = entry.name.toLowerCase();
      const lowerPath = entry.relativePath.toLowerCase();
      return lowerName === "valheim.exe" || lowerPath === "valheim_data" || lowerPath.endsWith("/valheim_data");
    })
  ) {
    return "valheim";
  }

  if (
    entries.some((entry) => {
      const lowerName = entry.name.toLowerCase();
      const lowerPath = entry.relativePath.toLowerCase();
      return lowerName === "windrose.exe" || lowerPath === "r5/content/paks" || lowerPath.endsWith("/r5/content/paks");
    })
  ) {
    return "windrose";
  }

  for (const entry of entries) {
    if (entry.isDirectory) {
      continue;
    }

    switch (entry.name.toLowerCase()) {
      case "re2.exe":
        return "re2";
      case "re3.exe":
        return "re3";
      case "re4.exe":
        return "re4";
      case "re7.exe":
        return "re7";
      case "re8.exe":
        return "re8";
      case "dd2.exe":
        return "dd2";
      case "dmc5.exe":
      case "devilmaycry5.exe":
        return "dmc5";
      case "mhrise.exe":
      case "monsterhunterrise.exe":
        return "mhrise";
      case "mhwilds.exe":
      case "monsterhunterwilds.exe":
        return "mhwilds";
      case "sf6.exe":
      case "streetfighter6.exe":
        return "sf6";
    }
  }

  return undefined;
}

function scoreEngine(
  entries: ProbeEntry[],
  scores: ScoreMap<GameEngine>,
  signals: DetectionSignal[]
): void {
  for (const entry of entries) {
    const lowerPath = entry.relativePath.toLowerCase();
    const lowerName = entry.name.toLowerCase();

    if (entry.isDirectory && lowerName.endsWith("_data") && entry.depth <= 2) {
      addScore(scores, signals, "unity-mono", 28, "Unity data folder", entry.relativePath);
      addScore(scores, signals, "unity-il2cpp", 28, "Unity data folder", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "unityplayer.dll" && entry.depth <= 2) {
      addScore(scores, signals, "unity-mono", 24, "Unity player runtime", entry.relativePath);
      addScore(scores, signals, "unity-il2cpp", 24, "Unity player runtime", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "gameassembly.dll") {
      addScore(scores, signals, "unity-il2cpp", 45, "Unity IL2CPP game assembly", entry.relativePath);
    }

    if (lowerPath.includes("/il2cpp_data/")) {
      addScore(scores, signals, "unity-il2cpp", 22, "Unity IL2CPP data folder", entry.relativePath);
    }

    if (!entry.isDirectory && lowerPath.endsWith("/managed/assembly-csharp.dll")) {
      addScore(scores, signals, "unity-mono", 45, "Unity managed game assembly", entry.relativePath);
    }

    if (entry.isDirectory && lowerName === "monobleedingedge") {
      addScore(scores, signals, "unity-mono", 18, "Unity Mono runtime folder", entry.relativePath);
    }

    if (
      lowerPath === "binaries/win64" ||
      lowerPath.endsWith("/binaries/win64") ||
      lowerPath.includes("/binaries/win64/")
    ) {
      addScore(scores, signals, "unreal", 32, "Unreal Win64 binaries folder", entry.relativePath);
    }

    if (
      lowerPath === "content/paks" ||
      lowerPath.endsWith("/content/paks") ||
      lowerPath.includes("/content/paks/")
    ) {
      addScore(scores, signals, "unreal", 38, "Unreal pak folder", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName.endsWith(".uproject")) {
      addScore(scores, signals, "unreal", 26, "Unreal project file", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName.endsWith(".pak") && lowerPath.includes("/content/paks/")) {
      addScore(scores, signals, "unreal", 24, "Unreal pak file", entry.relativePath);
    }

    if (!entry.isDirectory && /^re_chunk_.*\.pak/.test(lowerName)) {
      addScore(scores, signals, "re-engine", 48, "RE Engine chunk pak", entry.relativePath);
    }

    if (entry.isDirectory && lowerName === "natives") {
      addScore(scores, signals, "re-engine", 18, "RE Engine native assets folder", entry.relativePath);
    }
  }
}

function scoreLoaders(
  entries: ProbeEntry[],
  engine: GameEngine,
  scores: ScoreMap<LoaderKind>,
  signals: DetectionSignal[]
): void {
  const bepinexLoader: LoaderKind = engine === "unity-il2cpp" ? "bepinex-il2cpp" : "bepinex";

  for (const entry of entries) {
    const lowerPath = entry.relativePath.toLowerCase();
    const lowerName = entry.name.toLowerCase();

    if (lowerPath === "bepinex") {
      addScore(scores, signals, bepinexLoader, 8, "BepInEx folder", entry.relativePath);
    }

    if (!entry.isDirectory && lowerPath === "bepinex/core/bepinex.dll") {
      addScore(scores, signals, bepinexLoader, 42, "BepInEx core DLL", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "doorstop_config.ini") {
      addScore(scores, signals, bepinexLoader, 16, "Doorstop config", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "winhttp.dll" && engine.startsWith("unity")) {
      addScore(scores, signals, bepinexLoader, 16, "BepInEx bootstrap DLL", entry.relativePath);
    }

    if (lowerPath === "bepinex/interop" || lowerPath.startsWith("bepinex/interop/")) {
      addScore(scores, signals, "bepinex-il2cpp", 28, "BepInEx IL2CPP interop folder", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "ue4ss.dll") {
      addScore(scores, signals, "ue4ss", 44, "UE4SS DLL", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "ue4ss-settings.ini") {
      addScore(scores, signals, "ue4ss", 34, "UE4SS settings file", entry.relativePath);
    }

    if (lowerPath.includes("/binaries/win64/mods") || lowerPath.startsWith("mods/")) {
      addScore(scores, signals, "ue4ss", 18, "UE4SS mods folder", entry.relativePath);
    }

    if (lowerPath === "reframework") {
      addScore(scores, signals, "reframework", 8, "REFramework folder", entry.relativePath);
    }

    if (!entry.isDirectory && lowerName === "dinput8.dll" && engine === "re-engine") {
      addScore(scores, signals, "reframework", 24, "REFramework bootstrap DLL", entry.relativePath);
    }
  }
}

async function prepareModRoutes(
  gamePath: string,
  gameId: string | undefined,
  engine: GameEngine,
  loader: LoaderKind,
  entries: ProbeEntry[]
): Promise<RoutePreparation> {
  const routes: string[] = [];

  if (gameId === "valheim" || loader === "bepinex" || loader === "bepinex-il2cpp") {
    if (gameId === "valheim" || engine.startsWith("unity")) {
      pushUniqueRoute(routes, "BepInEx/plugins");
      pushUniqueRoute(routes, "BepInEx/config");
    }
  }

  if (engine === "unreal" || loader === "ue4ss" || gameId === "windrose") {
    for (const pakRoot of findUnrealPakRoots(entries)) {
      pushUniqueRoute(routes, `${pakRoot}/~mods`);
    }

    for (const win64Root of findUnrealWin64Dirs(entries)) {
      pushUniqueRoute(routes, `${win64Root}/Mods`);
    }
  }

  if (engine === "re-engine" || loader === "reframework" || isReEngineGameId(gameId)) {
    pushUniqueRoute(routes, "reframework/autorun");
    pushUniqueRoute(routes, "reframework/plugins");
  }

  const preparation: RoutePreparation = {
    expectedModFolders: [],
    createdModFolders: [],
    warnings: []
  };

  for (const route of routes) {
    preparation.expectedModFolders.push(route);
    const rootPath = path.resolve(gamePath);
    const targetPath = path.resolve(gamePath, route);
    const relativeToRoot = path.relative(rootPath, targetPath);

    if (relativeToRoot.startsWith("..") || path.isAbsolute(relativeToRoot)) {
      preparation.warnings.push(`Skipped unsafe expected mod route ${route}.`);
      continue;
    }

    try {
      const stat = await fs.stat(targetPath).catch(() => null);
      if (stat?.isDirectory()) {
        continue;
      }

      if (stat) {
        preparation.warnings.push(`Expected mod route exists as a file and was not changed: ${route}.`);
        continue;
      }

      await fs.mkdir(targetPath, { recursive: true });
      preparation.createdModFolders.push(route);
    } catch (caughtError) {
      preparation.warnings.push(`Could not create expected mod route ${route}: ${String(caughtError)}.`);
    }
  }

  return preparation;
}

function pushUniqueRoute(routes: string[], route: string): void {
  const normalized = route.replace(/\\/g, "/").replace(/^\/+/, "");
  if (!routes.some((existing) => existing.toLowerCase() === normalized.toLowerCase())) {
    routes.push(normalized);
  }
}

function findUnrealPakRoots(entries: ProbeEntry[]): string[] {
  return entries
    .filter((entry) => {
      const lowerPath = entry.relativePath.toLowerCase();
      return (
        entry.isDirectory &&
        lowerPath.endsWith("content/paks") &&
        !lowerPath.startsWith("engine/") &&
        !lowerPath.includes("/engine/content/paks")
      );
    })
    .map((entry) => entry.relativePath)
    .sort();
}

function findUnrealWin64Dirs(entries: ProbeEntry[]): string[] {
  return entries
    .filter((entry) => {
      const lowerPath = entry.relativePath.toLowerCase();
      return entry.isDirectory && (lowerPath === "binaries/win64" || lowerPath.endsWith("/binaries/win64"));
    })
    .map((entry) => entry.relativePath)
    .sort();
}

function isReEngineGameId(gameId: string | undefined): boolean {
  return ["re2", "re3", "re4", "re7", "re8", "dd2", "dmc5", "mhrise", "mhwilds", "sf6"].includes(gameId ?? "");
}

function recommendLoader(engine: GameEngine): LoaderKind {
  switch (engine) {
    case "unity-mono":
      return "bepinex";
    case "unity-il2cpp":
      return "bepinex-il2cpp";
    case "unreal":
      return "ue4ss";
    case "re-engine":
      return "reframework";
    case "unknown":
      return "none";
  }
}

function addScore<T extends string>(
  scores: ScoreMap<T>,
  signals: DetectionSignal[],
  key: T,
  weight: number,
  label: string,
  relativePath: string
): void {
  scores[key] += weight;
  signals.push({
    label,
    path: relativePath,
    weight
  });
}

function chooseHighest<T extends string>(scores: ScoreMap<T>, fallback: T): T {
  let bestKey = fallback;
  let bestScore = scores[fallback] ?? 0;

  for (const [key, score] of Object.entries(scores) as Array<[T, number]>) {
    if (score > bestScore) {
      bestKey = key;
      bestScore = score;
    }
  }

  return bestScore > 0 ? bestKey : fallback;
}

function confidenceFor(score: number): number {
  return Math.max(0, Math.min(0.98, score / 100));
}

function formatLoader(loader: LoaderKind): string {
  switch (loader) {
    case "bepinex":
      return "BepInEx";
    case "bepinex-il2cpp":
      return "BepInEx IL2CPP";
    case "ue4ss":
      return "UE4SS";
    case "reframework":
      return "REFramework";
    case "loose-files":
      return "Loose files";
    case "none":
      return "No loader";
  }
}

async function walkGameFolder(root: string): Promise<ProbeEntry[]> {
  const entries: ProbeEntry[] = [];
  const queue: Array<{ absolutePath: string; relativePath: string; depth: number }> = [
    { absolutePath: root, relativePath: "", depth: 0 }
  ];

  while (queue.length > 0 && entries.length < maxEntries) {
    const current = queue.shift();
    if (!current || current.depth > maxDepth) {
      continue;
    }

    let dirents: import("node:fs").Dirent[];
    try {
      dirents = await fs.readdir(current.absolutePath, { withFileTypes: true });
    } catch {
      continue;
    }

    for (const dirent of dirents) {
      if (entries.length >= maxEntries) {
        break;
      }

      const relativePath = toPortablePath(path.join(current.relativePath, dirent.name));
      const isDirectory = dirent.isDirectory();
      const depth = current.depth + 1;

      entries.push({
        relativePath,
        name: dirent.name,
        isDirectory,
        depth
      });

      if (isDirectory && depth < maxDepth && (depth <= 1 || shouldDescendInto(relativePath, dirent.name))) {
        queue.push({
          absolutePath: path.join(current.absolutePath, dirent.name),
          relativePath,
          depth
        });
      }
    }
  }

  return entries;
}

function shouldDescendInto(relativePath: string, name: string): boolean {
  const lowerName = name.toLowerCase();
  const lowerPath = relativePath.toLowerCase();

  if (["node_modules", ".git", "screenshots", "captures", "logs", "crash reports"].includes(lowerName)) {
    return false;
  }

  return (
    lowerName.endsWith("_data") ||
    lowerName === "managed" ||
    lowerName === "plugins" ||
    lowerName === "config" ||
    lowerName === "core" ||
    lowerName === "bepinex" ||
    lowerName === "interop" ||
    lowerName === "binaries" ||
    lowerName === "win64" ||
    lowerName === "content" ||
    lowerName === "paks" ||
    lowerName === "~mods" ||
    lowerName === "mods" ||
    lowerName === "reframework" ||
    lowerName === "autorun" ||
    lowerName === "natives" ||
    lowerPath.includes("/binaries") ||
    lowerPath.includes("/content")
  );
}

function toPortablePath(input: string): string {
  return input.replace(/\\/g, "/");
}
