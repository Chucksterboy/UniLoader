import fs from "node:fs/promises";
import path from "node:path";
import JSZip from "jszip";
import {
  ArchiveEntry,
  ThunderstoreManifest
} from "../../shared/contracts";
import {
  getCommonTopFolder,
  normalizeArchivePath,
  toLogicalPath
} from "../archiveUtils";

export interface ScannedZipArchive {
  archivePath: string;
  archiveName: string;
  entries: ArchiveEntry[];
  manifest?: ThunderstoreManifest;
  zip: JSZip;
}

export async function scanZipArchive(archivePath: string): Promise<ScannedZipArchive> {
  if (!archivePath.toLowerCase().endsWith(".zip")) {
    throw new Error("Only .zip imports are supported in this first build.");
  }

  const buffer = await fs.readFile(archivePath);
  const zip = await JSZip.loadAsync(buffer);
  const rawPaths = Object.keys(zip.files).map(normalizeArchivePath);
  const commonTopFolder = getCommonTopFolder(rawPaths);

  const entries: ArchiveEntry[] = Object.values(zip.files).map((entry) => ({
    path: normalizeArchivePath(entry.name),
    logicalPath: toLogicalPath(entry.name, commonTopFolder),
    size: entry.dir ? 0 : getUncompressedSize(entry),
    isDirectory: entry.dir
  }));

  const manifest = await readThunderstoreManifest(zip, entries);

  return {
    archivePath,
    archiveName: path.basename(archivePath),
    entries,
    manifest,
    zip
  };
}

function getUncompressedSize(entry: JSZip.JSZipObject): number {
  const maybeSizedEntry = entry as JSZip.JSZipObject & {
    _data?: { uncompressedSize?: number };
  };

  return maybeSizedEntry._data?.uncompressedSize ?? 0;
}

async function readThunderstoreManifest(
  zip: JSZip,
  entries: ArchiveEntry[]
): Promise<ThunderstoreManifest | undefined> {
  const manifestEntry = entries.find(
    (entry) => !entry.isDirectory && entry.logicalPath.toLowerCase() === "manifest.json"
  );

  if (!manifestEntry) {
    return undefined;
  }

  const zipEntry = zip.file(manifestEntry.path);
  if (!zipEntry) {
    return undefined;
  }

  try {
    return JSON.parse(await zipEntry.async("string")) as ThunderstoreManifest;
  } catch {
    return undefined;
  }
}
