import { ArchiveAnalysis, GameProfile } from "../shared/contracts";
import { analyzeScannedArchive } from "./adapters/adapterRegistry";
import { scanZipArchive } from "./archives/zipArchive";

export async function analyzeArchiveForProfile(
  archivePath: string,
  profile: GameProfile
): Promise<ArchiveAnalysis> {
  const scannedArchive = await scanZipArchive(archivePath);
  return analyzeScannedArchive(scannedArchive, profile);
}
