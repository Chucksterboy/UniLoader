import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import {
  CreateProfileInput,
  GameProfile,
  InstalledModRecord
} from "../../shared/contracts";

interface StoreFile<T> {
  version: number;
  items: T[];
}

export class ProfileStore {
  private readonly profilesPath: string;
  private readonly installedModsPath: string;

  constructor(private readonly dataDir: string) {
    this.profilesPath = path.join(dataDir, "profiles.json");
    this.installedModsPath = path.join(dataDir, "installed-mods.json");
  }

  get rootDir(): string {
    return this.dataDir;
  }

  getProfileDir(profileId: string): string {
    return path.join(this.dataDir, "profiles", profileId);
  }

  getProfileBackupDir(profileId: string, installId: string): string {
    return path.join(this.getProfileDir(profileId), "backups", installId);
  }

  async initialize(): Promise<void> {
    await fs.mkdir(this.dataDir, { recursive: true });
    await fs.mkdir(path.join(this.dataDir, "profiles"), { recursive: true });
    await this.ensureStoreFile<GameProfile>(this.profilesPath);
    await this.ensureStoreFile<InstalledModRecord>(this.installedModsPath);
  }

  async listProfiles(): Promise<GameProfile[]> {
    return (await this.readStoreFile<GameProfile>(this.profilesPath)).items;
  }

  async createProfile(input: CreateProfileInput): Promise<GameProfile> {
    const now = new Date().toISOString();
    const profile: GameProfile = {
      id: randomUUID(),
      name: input.name.trim(),
      gamePath: input.gamePath,
      engine: input.engine,
      loader: input.loader,
      createdAt: now,
      updatedAt: now
    };

    if (!profile.name) {
      throw new Error("Profile name is required.");
    }

    const storeFile = await this.readStoreFile<GameProfile>(this.profilesPath);
    storeFile.items.push(profile);
    await this.writeStoreFile(this.profilesPath, storeFile);
    await fs.mkdir(this.getProfileDir(profile.id), { recursive: true });

    return profile;
  }

  async getProfile(profileId: string): Promise<GameProfile> {
    const profile = (await this.listProfiles()).find((item) => item.id === profileId);
    if (!profile) {
      throw new Error(`Profile not found: ${profileId}`);
    }

    return profile;
  }

  async listInstalledMods(profileId?: string): Promise<InstalledModRecord[]> {
    const installedMods = (await this.readStoreFile<InstalledModRecord>(this.installedModsPath)).items;
    return profileId
      ? installedMods.filter((installedMod) => installedMod.profileId === profileId)
      : installedMods;
  }

  async addInstalledMod(record: InstalledModRecord): Promise<void> {
    const storeFile = await this.readStoreFile<InstalledModRecord>(this.installedModsPath);
    storeFile.items.push(record);
    await this.writeStoreFile(this.installedModsPath, storeFile);
  }

  private async ensureStoreFile<T>(filePath: string): Promise<void> {
    try {
      await fs.access(filePath);
    } catch {
      await this.writeStoreFile<T>(filePath, { version: 1, items: [] });
    }
  }

  private async readStoreFile<T>(filePath: string): Promise<StoreFile<T>> {
    await this.ensureStoreFile<T>(filePath);
    const raw = await fs.readFile(filePath, "utf8");
    return JSON.parse(raw) as StoreFile<T>;
  }

  private async writeStoreFile<T>(filePath: string, storeFile: StoreFile<T>): Promise<void> {
    await fs.mkdir(path.dirname(filePath), { recursive: true });
    await fs.writeFile(filePath, `${JSON.stringify(storeFile, null, 2)}\n`, "utf8");
  }
}
