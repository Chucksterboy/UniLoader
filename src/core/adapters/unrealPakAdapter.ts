import path from "node:path";
import { InstallMapping, InstallPlan } from "../../shared/contracts";
import { hasExtension } from "../archiveUtils";
import { AdapterContext, installableFiles, ModAdapter, normalizeTargetPath } from "./adapter";

export const unrealPakAdapter: ModAdapter = {
  id: "unreal-pak",
  name: "Unreal Pak Files",
  createPlan(context: AdapterContext): InstallPlan | null {
    const pakFiles = installableFiles(context.entries).filter((file) =>
      hasExtension(file.logicalPath, [".pak", ".ucas", ".utoc"])
    );

    if (pakFiles.length === 0) {
      return null;
    }

    const pakTargetDirs = unrealPakTargetDirs(context.profile);
    const mappings: InstallMapping[] = pakFiles.flatMap((file) =>
      pakTargetDirs.map((pakTargetDir) => ({
        sourcePath: file.path,
        targetRoot: "game",
        targetRelativePath: normalizeTargetPath(`${pakTargetDir}/${path.basename(file.logicalPath)}`),
        reason: "Generic Unreal Engine pak-style mod file."
      }))
    );

    const warnings =
      context.profile.engine === "unreal" || context.profile.engine === "unknown"
        ? []
        : ["Pak mods are normally Unreal Engine mods; verify this profile before installing."];

    return {
      adapterId: "unreal-pak",
      adapterName: "Unreal Pak Files",
      confidence: context.profile.engine === "unreal" ? 0.88 : 0.66,
      summary: `Deploy ${pakFiles.length} pak file(s) to ${joinHumanList(pakTargetDirs)}.`,
      mappings,
      dependencies: [],
      warnings,
      requiresConfirmation: false
    };
  }
};

function unrealPakTargetDirs(profile: { gameId?: string; name: string; gamePath: string }): string[] {
  const isWindrose =
    profile.gameId === "windrose" ||
    profile.name.toLowerCase() === "windrose" ||
    profile.gamePath.toLowerCase().includes("windrose");

  return isWindrose
    ? ["R5/Content/Paks/~mods", "R5/Builds/WindowsServer/R5/Content/Paks/~mods"]
    : ["Content/Paks/~mods"];
}

function joinHumanList(items: string[]): string {
  if (items.length <= 1) {
    return items[0] ?? "";
  }

  if (items.length === 2) {
    return `${items[0]} and ${items[1]}`;
  }

  return `${items.slice(0, -1).join(", ")}, and ${items[items.length - 1]}`;
}
