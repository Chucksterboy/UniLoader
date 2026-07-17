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

export const reframeworkAdapter: ModAdapter = {
  id: "reframework",
  name: "REFramework / RE Engine",
  createPlan(context: AdapterContext): InstallPlan | null {
    const files = installableFiles(context.entries);
    const mappings: InstallMapping[] = [];
    const warnings: string[] = [];

    for (const file of files) {
      const lowerPath = file.logicalPath.toLowerCase();

      if (isREFrameworkRootRuntimeFile(file.logicalPath)) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(path.basename(file.logicalPath)),
          reason: "REFramework bootstrap/runtime file."
        });
        continue;
      }

      if (lowerPath.startsWith("reframework/")) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(file.logicalPath),
          reason: "Archive already contains an REFramework folder layout."
        });
        continue;
      }

      if (lowerPath.endsWith(".lua")) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`reframework/autorun/${path.basename(file.logicalPath)}`),
          reason: "REFramework autorun Lua script."
        });
        continue;
      }

      if (hasExtension(file.logicalPath, [".dll"])) {
        mappings.push({
          sourcePath: file.path,
          targetRoot: "game",
          targetRelativePath: normalizeTargetPath(`reframework/plugins/${path.basename(file.logicalPath)}`),
          reason: "REFramework native plugin."
        });
      }
    }

    const hasSignals = files.some((file) => {
      const lowerPath = file.logicalPath.toLowerCase();
      return (
        lowerPath.startsWith("reframework/") ||
        lowerPath.endsWith(".lua") ||
        isREFrameworkRootRuntimeFile(file.logicalPath)
      );
    });

    if (!hasSignals || mappings.length === 0) {
      return null;
    }

    if (context.profile.engine !== "re-engine" && context.profile.engine !== "unknown") {
      warnings.push("This looks like an REFramework mod, but the selected profile is not marked as RE Engine.");
    }

    return {
      adapterId: "reframework",
      adapterName: "REFramework / RE Engine",
      confidence: context.profile.loader === "reframework" ? 0.9 : 0.76,
      summary: `Install ${mappings.length} file(s) into the REFramework layout.`,
      mappings,
      dependencies: [createKnownRuntimeDependency(context.profile, "reframework")],
      warnings,
      requiresConfirmation: false
    };
  }
};

function isREFrameworkRootRuntimeFile(inputPath: string): boolean {
  return [
    "dinput8.dll",
    "openvr_api.dll",
    "openxr_loader.dll",
    "reframework_revision.txt",
    "delete_openvr_api_dll_if_you_want_to_use_openxr"
  ].includes(path.basename(inputPath).toLowerCase());
}
