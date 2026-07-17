import { ArchiveAnalysis, GameProfile } from "../../shared/contracts";
import { attachManifestDependencies } from "../dependencies/dependencyResolver";
import { ScannedZipArchive } from "../archives/zipArchive";
import { bepinexAdapter } from "./bepinexAdapter";
import { looseFilesAdapter } from "./looseFilesAdapter";
import { ModAdapter } from "./adapter";
import { reframeworkAdapter } from "./reframeworkAdapter";
import { ue4ssAdapter } from "./ue4ssAdapter";
import { unrealPakAdapter } from "./unrealPakAdapter";

export const modAdapters: ModAdapter[] = [
  bepinexAdapter,
  ue4ssAdapter,
  reframeworkAdapter,
  unrealPakAdapter,
  looseFilesAdapter
];

export function analyzeScannedArchive(
  scannedArchive: ScannedZipArchive,
  profile: GameProfile
): ArchiveAnalysis {
  const plans = modAdapters
    .map((adapter) =>
      adapter.createPlan({
        archiveName: scannedArchive.archiveName,
        entries: scannedArchive.entries,
        manifest: scannedArchive.manifest,
        profile
      })
    )
    .filter((plan) => plan !== null)
    .map((plan) => attachManifestDependencies(plan, scannedArchive.manifest))
    .sort((a, b) => b.confidence - a.confidence);

  return {
    archivePath: scannedArchive.archivePath,
    archiveName: scannedArchive.archiveName,
    entries: scannedArchive.entries,
    manifest: scannedArchive.manifest,
    plans,
    recommendedPlan: plans[0]
  };
}
