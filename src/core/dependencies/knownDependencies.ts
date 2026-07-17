import { DependencySpec, GameProfile } from "../../shared/contracts";

export function createKnownRuntimeDependency(
  profile: GameProfile,
  runtime: "bepinex" | "bepinex-il2cpp" | "ue4ss" | "reframework"
): DependencySpec {
  if (runtime === "bepinex" && profile.gameId === "valheim") {
    return {
      id: "thunderstore:denikson-BepInExPack_Valheim",
      name: "denikson/BepInExPack_Valheim",
      provider: "thunderstore",
      required: true,
      status: profile.loader === "bepinex" ? "already-installed" : "missing",
      source: "adapter",
      notes: "Valheim-specific BepInEx pack."
    };
  }

  const metadata = {
    bepinex: {
      id: "runtime:bepinex",
      name: "BepInEx Mono x64",
      provider: "github-release" as const,
      source: "github-release:BepInEx/BepInEx#BepInEx_win_x64_*.zip",
      notes: "Official stable BepInEx 5 Windows x64 build for Mono Unity games."
    },
    "bepinex-il2cpp": {
      id: "runtime:bepinex-il2cpp",
      name: "BepInEx Unity IL2CPP x64",
      provider: "bepinbuilds" as const,
      source: "bepinbuilds:bepinex_be#BepInEx-Unity.IL2CPP-win-x64-*.zip",
      notes: "Official BepInEx bleeding-edge IL2CPP build for Windows x64 Unity games."
    },
    ue4ss: {
      id: "runtime:ue4ss",
      name: "UE4SS",
      provider: "github-release" as const,
      source: "github-release:UE4SS-RE/RE-UE4SS#UE4SS_v*.zip",
      notes: "Official latest UE4SS release for Unreal Engine script and hook mods."
    },
    reframework: {
      id: "runtime:reframework",
      name: "REFramework",
      provider: "github-release" as const,
      source: reframeworkReleaseSource(profile),
      notes: "Official REFramework release. Uses game-specific stable assets when UniLoader recognizes the RE Engine game."
    }
  }[runtime];

  const alreadyInstalled =
    profile.loader === runtime ||
    (runtime === "bepinex" && profile.loader === "bepinex") ||
    (runtime === "bepinex-il2cpp" && profile.loader === "bepinex-il2cpp");

  return {
    ...metadata,
    required: true,
    status: alreadyInstalled ? "already-installed" : "missing",
    source: metadata.source
  };
}

function reframeworkReleaseSource(profile: GameProfile): string {
  switch (profile.gameId) {
    case "dd2":
      return "github-release:praydog/REFramework#DD2.zip";
    case "dmc5":
      return "github-release:praydog/REFramework#DMC5.zip";
    case "mhrise":
      return "github-release:praydog/REFramework#MHRISE.zip";
    case "mhwilds":
      return "github-release:praydog/REFramework#MHWILDS.zip";
    case "re2":
      return "github-release:praydog/REFramework#RE2.zip";
    case "re3":
      return "github-release:praydog/REFramework#RE3.zip";
    case "re4":
      return "github-release:praydog/REFramework#RE4.zip";
    case "re7":
      return "github-release:praydog/REFramework#RE7.zip";
    case "re8":
      return "github-release:praydog/REFramework#RE8.zip";
    case "sf6":
      return "github-release:praydog/REFramework#SF6.zip";
    default:
      return "github-release:praydog/REFramework-nightly#REFramework.zip";
  }
}

export function parseThunderstoreDependency(dependency: string): DependencySpec {
  const parts = dependency.split("-");
  const version = parts.pop();
  const packageName = parts.pop();
  const teamName = parts.join("-");

  return {
    id: `thunderstore:${dependency}`,
    name: packageName ? `${teamName}/${packageName}` : dependency,
    version,
    provider: "thunderstore",
    required: true,
    status: "missing",
    source: "manifest.json"
  };
}

export function mergeDependencies(dependencies: DependencySpec[]): DependencySpec[] {
  const byId = new Map<string, DependencySpec>();

  for (const dependency of dependencies) {
    const existing = byId.get(dependency.id);
    if (!existing) {
      byId.set(dependency.id, dependency);
      continue;
    }

    byId.set(dependency.id, {
      ...existing,
      required: existing.required || dependency.required,
      status:
        existing.status === "already-installed" ? existing.status : dependency.status,
      notes: existing.notes ?? dependency.notes
    });
  }

  return [...byId.values()];
}
