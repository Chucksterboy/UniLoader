import path from "node:path";
import { InstallMapping, InstallPlan } from "../../shared/contracts";
import { hasExtension } from "../archiveUtils";
import { createKnownRuntimeDependency } from "../dependencies/knownDependencies";
import {
  AdapterContext,
  installableFiles,
  ModAdapter,
  normalizeTargetPath
} from "./adapter";

export const bepinexAdapter: ModAdapter = {
  id: "bepinex",
  name: "BepInEx / Thunderstore",
  createPlan(context: AdapterContext): InstallPlan | null {
    const files = installableFiles(context.entries);
    const mappings: InstallMapping[] = [];
    const warnings: string[] = [];

    for (const file of files) {
      const lowerPath = file.logicalPath.toLowerCase();
      const bepinexRelativePath = pathAfterSegment(file.logicalPath, "bepinex");
      const doorstopRelativePath = pathAfterSegment(file.logicalPath, "doorstop_libs");
      const corlibRelativePath = pathAfterSegment(file.logicalPath, "unstripped_corlib");
      const dotnetRelativePath = pathAfterSegment(file.logicalPath, "dotnet");

      if (bepinexRelativePath) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`BepInEx/${bepinexRelativePath}`),
          reason: "Archive contains a BepInEx folder layout."
        });
        continue;
      }

      if (doorstopRelativePath) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`doorstop_libs/${doorstopRelativePath}`),
          reason: "Doorstop runtime support file."
        });
        continue;
      }

      if (corlibRelativePath) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`unstripped_corlib/${corlibRelativePath}`),
          reason: "BepInEx runtime support file."
        });
        continue;
      }

      if (dotnetRelativePath) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`dotnet/${dotnetRelativePath}`),
          reason: "BepInEx bundled runtime file."
        });
        continue;
      }

      if (isBepInExRootRuntimeFile(file.logicalPath)) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(path.basename(file.logicalPath)),
          reason: "BepInEx bootstrap file."
        });
        continue;
      }

      if (lowerPath.startsWith("plugins/")) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`BepInEx/${file.logicalPath}`),
          reason: "Plugin folder maps into BepInEx/plugins."
        });
        continue;
      }

      if (lowerPath.startsWith("config/") || lowerPath.endsWith(".cfg")) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`BepInEx/config/${path.basename(file.logicalPath)}`),
          reason: "BepInEx config file."
        });
        continue;
      }

      if (isProbableBepInExPluginDll(file.logicalPath, context.profile.engine)) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`BepInEx/plugins/${path.basename(file.logicalPath)}`),
          reason: "Managed plugin DLL."
        });
      }
    }

    const hasBepInExSignals = files.some((file) => {
      const lowerPath = file.logicalPath.toLowerCase();
      return (
        pathAfterSegment(file.logicalPath, "bepinex") !== null ||
        lowerPath.startsWith("plugins/") ||
        isProbableBepInExPluginDll(file.logicalPath, context.profile.engine) ||
        isBepInExRootRuntimeFile(file.logicalPath)
      );
    });

    if (!hasBepInExSignals || mappings.length === 0) {
      return null;
    }

    const runtime =
      context.profile.engine === "unity-il2cpp" || context.profile.loader === "bepinex-il2cpp"
        ? "bepinex-il2cpp"
        : "bepinex";

    if (files.some((file) => hasExtension(file.logicalPath, [".dll"])) && context.profile.engine === "unknown") {
      warnings.push("This looks like a BepInEx mod, but the profile engine is unknown.");
    }

    return {
      adapterId: "bepinex",
      adapterName: "BepInEx / Thunderstore",
      confidence: context.manifest || context.profile.loader.includes("bepinex") ? 0.92 : 0.78,
      summary: `Install ${mappings.length} file(s) into the BepInEx layout.`,
      mappings,
      dependencies: [createKnownRuntimeDependency(context.profile, runtime)],
      warnings,
      requiresConfirmation: false
    };
  }
};

function pathAfterSegment(inputPath: string, segment: string): string | null {
  const parts = inputPath.replace(/\\/g, "/").split("/").filter(Boolean);
  const index = parts.findIndex((part) => part.toLowerCase() === segment.toLowerCase());
  if (index < 0 || index === parts.length - 1) {
    return null;
  }

  return parts.slice(index + 1).join("/");
}

function isBepInExRootRuntimeFile(inputPath: string): boolean {
  return [
    "doorstop_config.ini",
    "winhttp.dll",
    "doorstop_config_il2cpp.ini",
    "winhttp_il2cpp.dll",
    "start_game_bepinex.sh",
    "run_bepinex.sh"
  ].includes(path.basename(inputPath).toLowerCase());
}

function isProbableBepInExPluginDll(inputPath: string, engine: string): boolean {
  const fileName = path.basename(inputPath).toLowerCase();
  if (!hasExtension(fileName, [".dll"]) || isKnownNativeBootstrapFile(fileName)) {
    return false;
  }

  return engine.startsWith("unity") || engine === "unknown";
}

function isKnownNativeBootstrapFile(fileName: string): boolean {
  return [
    "dinput8.dll",
    "xinput1_3.dll",
    "dwmapi.dll",
    "ue4ss.dll",
    "openvr_api.dll",
    "openxr_loader.dll",
    "version.dll",
    "winmm.dll"
  ].includes(fileName);
}
