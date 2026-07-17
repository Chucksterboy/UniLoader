import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import {
  GameProfile,
  InstallPlan,
  InstallResult,
  InstalledModRecord
} from "../../shared/contracts";
import { basenameWithoutArchiveExtension, safeJoin } from "../archiveUtils";
import { scanZipArchive } from "../archives/zipArchive";
import { ProfileStore } from "../profiles/profileStore";

export async function installArchiveWithPlan(
  store: ProfileStore,
  profile: GameProfile,
  archivePath: string,
  plan: InstallPlan
): Promise<InstallResult> {
  const installId = randomUUID();
  const installedAt = new Date().toISOString();
  const scannedArchive = await scanZipArchive(archivePath);
  const profileRoot = store.getProfileDir(profile.id);
  const backupRoot = store.getProfileBackupDir(profile.id, installId);
  const filesWritten: string[] = [];
  const backupsWritten: string[] = [];
  const warnings = [...plan.warnings];

  for (const mapping of plan.mappings) {
    const zipEntry = scannedArchive.zip.file(mapping.sourcePath);
    if (!zipEntry) {
      warnings.push(`Skipped missing archive entry: ${mapping.sourcePath}`);
      continue;
    }

    const targetRoot = mapping.targetRoot === "game" ? profile.gamePath : profileRoot;
    const destinationPath = safeJoin(targetRoot, mapping.targetRelativePath);
    await fs.mkdir(path.dirname(destinationPath), { recursive: true });

    if (mapping.targetRoot === "game" && (await fileExists(destinationPath))) {
      const backupPath = safeJoin(backupRoot, mapping.targetRelativePath);
      await fs.mkdir(path.dirname(backupPath), { recursive: true });
      await fs.copyFile(destinationPath, backupPath);
      backupsWritten.push(backupPath);
    }

    const content = await zipEntry.async("nodebuffer");
    await fs.writeFile(destinationPath, content);
    filesWritten.push(destinationPath);
  }

  const record: InstalledModRecord = {
    id: installId,
    profileId: profile.id,
    archivePath,
    archiveName: path.basename(archivePath),
    displayName: scannedArchive.manifest?.name ?? basenameWithoutArchiveExtension(archivePath),
    adapterId: plan.adapterId,
    summary: plan.summary,
    installedAt,
    filesWritten,
    backupsWritten,
    dependencies: plan.dependencies
  };

  await store.addInstalledMod(record);

  await writeInstallReceipt(store, profile.id, {
    ...record,
    displayName: scannedArchive.manifest?.name ?? basenameWithoutArchiveExtension(archivePath)
  });

  return {
    profileId: profile.id,
    archivePath,
    installedModId: installId,
    installedAt,
    filesWritten,
    backupsWritten,
    warnings
  };
}

async function fileExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function writeInstallReceipt(
  store: ProfileStore,
  profileId: string,
  receipt: InstalledModRecord & { displayName: string }
): Promise<void> {
  const receiptPath = path.join(store.getProfileDir(profileId), "receipts", `${receipt.id}.json`);
  await fs.mkdir(path.dirname(receiptPath), { recursive: true });
  await fs.writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, "utf8");
}
