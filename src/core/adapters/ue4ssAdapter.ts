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

export const ue4ssAdapter: ModAdapter = {
  id: "ue4ss",
  name: "UE4SS / Unreal Scripts",
  createPlan(context: AdapterContext): InstallPlan | null {
    const files = installableFiles(context.entries);
    const mappings: InstallMapping[] = [];
    const warnings: string[] = [];

    for (const file of files) {
      const lowerPath = file.logicalPath.toLowerCase();

      if (isUE4SSRootRuntimeFile(file.logicalPath)) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`Binaries/Win64/${path.basename(file.logicalPath)}`),
          reason: "UE4SS runtime bootstrap file."
        });
        continue;
      }

      if (lowerPath.startsWith("mods/")) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`Binaries/Win64/${file.logicalPath}`),
          reason: "UE4SS Mods folder."
        });
        continue;
      }

      if (lowerPath.startsWith("ue4ss/")) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`Binaries/Win64/${file.logicalPath}`),
          reason: "UE4SS runtime or configuration files."
        });
        continue;
      }

      if (lowerPath.includes("/scripts/") || lowerPath.endsWith(".lua")) {
        const modFolderName = path.basename(context.archiveName).replace(/\.(zip|7z|rar)$/i, "");
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`Binaries/Win64/Mods/${modFolderName}/${file.logicalPath}`),
          reason: "UE4SS script file."
        });
      }
    }

    const hasSignals = files.some((file) => {
      const lowerPath = file.logicalPath.toLowerCase();
      return (
        lowerPath.startsWith("mods/") ||
        lowerPath.startsWith("ue4ss/") ||
        isUE4SSRootRuntimeFile(file.logicalPath) ||
        lowerPath.includes("/scripts/") ||
        lowerPath.endsWith(".lua")
      );
    });

    if (!hasSignals || mappings.length === 0) {
      return null;
    }

    if (context.profile.engine !== "unreal" && context.profile.engine !== "unknown") {
      warnings.push("This looks like a UE4SS mod, but the selected profile is not marked as Unreal.");
    }

    if (files.some((file) => hasExtension(file.logicalPath, [".pak"]))) {
      warnings.push("This archive also contains pak files; the Unreal pak adapter may be a better fit.");
    }

    return {
      adapterId: "ue4ss",
      adapterName: "UE4SS / Unreal Scripts",
      confidence: context.profile.loader === "ue4ss" ? 0.9 : 0.72,
      summary: `Install ${mappings.length} file(s) into the default UE4SS layout.`,
      mappings,
      dependencies: [createKnownRuntimeDependency(context.profile, "ue4ss")],
      warnings,
      requiresConfirmation: false
    };
  }
};

function isUE4SSRootRuntimeFile(inputPath: string): boolean {
  return ["ue4ss.dll", "ue4ss-settings.ini", "dwmapi.dll", "xinput1_3.dll"].includes(
    path.basename(inputPath).toLowerCase()
  );
}
