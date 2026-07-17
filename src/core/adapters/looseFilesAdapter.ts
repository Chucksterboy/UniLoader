import { InstallMapping, InstallPlan } from "../../shared/contracts";
import { AdapterContext, installableFiles, ModAdapter, normalizeTargetPath } from "./adapter";

export const looseFilesAdapter: ModAdapter = {
  id: "loose-files",
  name: "Loose Files",
  createPlan(context: AdapterContext): InstallPlan | null {
    const files = installableFiles(context.entries);

    if (files.length === 0) {
      return null;
    }

    const mappings: InstallMapping[] = files.map((file) => ({
      sourcePath: file.path,
      targetRoot: "profile",
      targetRelativePath: normalizeTargetPath(`staged/${file.logicalPath}`),
      reason: "Unrecognized loose file staged in the profile instead of copied into the game."
    }));

    return {
      adapterId: "loose-files",
      adapterName: "Loose Files",
      confidence: 0.25,
      summary: `Stage ${mappings.length} unrecognized file(s) for manual review.`,
      mappings,
      dependencies: [],
      warnings: [
        "UniLoader could not identify a safe game-specific install layout.",
        "Files will be staged in the profile data folder instead of deployed into the game."
      ],
      requiresConfirmation: true
    };
  }
};
