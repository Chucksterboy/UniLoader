import {
  ArchiveEntry,
  GameProfile,
  InstallPlan,
  ThunderstoreManifest
} from "../../shared/contracts";

export interface AdapterContext {
  archiveName: string;
  entries: ArchiveEntry[];
  manifest?: ThunderstoreManifest;
  profile: GameProfile;
}

export interface ModAdapter {
  id: InstallPlan["adapterId"];
  name: string;
  createPlan(context: AdapterContext): InstallPlan | null;
}

export function installableFiles(entries: ArchiveEntry[]): ArchiveEntry[] {
  return entries.filter((entry) => !entry.isDirectory && !isPackageMetadata(entry.logicalPath));
}

export function isPackageMetadata(logicalPath: string): boolean {
  const lowerPath = logicalPath.toLowerCase();
  return (
    lowerPath === "manifest.json" ||
    lowerPath === "readme.md" ||
    lowerPath === "readme.txt" ||
    lowerPath === "icon.png" ||
    lowerPath === "changelog.md" ||
    lowerPath === "license" ||
    lowerPath === "license.txt"
  );
}

export function normalizeTargetPath(path: string): string {
  return path.replace(/\\/g, "/").replace(/^\/+/, "");
}
