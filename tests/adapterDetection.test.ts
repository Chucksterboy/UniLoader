import { describe, expect, it } from "vitest";
import { analyzeScannedArchive } from "../src/core/adapters/adapterRegistry";
import { ScannedZipArchive } from "../src/core/archives/zipArchive";
import { GameProfile } from "../src/shared/contracts";

const baseProfile: GameProfile = {
  id: "profile-1",
  name: "Test Game",
  gamePath: "C:/Games/TestGame",
  engine: "unknown",
  loader: "none",
  createdAt: "2026-07-16T00:00:00.000Z",
  updatedAt: "2026-07-16T00:00:00.000Z"
};

describe("mod archive adapter detection", () => {
  it("detects BepInEx layouts and attaches Thunderstore manifest dependencies", () => {
    const archive = fakeArchive({
      entries: [
        "manifest.json",
        "BepInEx/plugins/CoolMod.dll",
        "BepInEx/config/CoolMod.cfg"
      ],
      manifestDependencies: ["BepInEx-BepInExPack-5.4.2100"]
    });

    const analysis = analyzeScannedArchive(archive, {
      ...baseProfile,
      engine: "unity-mono"
    });

    expect(analysis.recommendedPlan?.adapterId).toBe("bepinex");
    expect(analysis.recommendedPlan?.mappings).toHaveLength(2);
    expect(analysis.recommendedPlan?.dependencies.map((dependency) => dependency.id)).toContain(
      "thunderstore:BepInEx-BepInExPack-5.4.2100"
    );
  });

  it("detects Unreal pak archives", () => {
    const archive = fakeArchive({
      entries: ["CoolPak/Content/Paks/MyMod.pak", "CoolPak/Content/Paks/MyMod.utoc"]
    });

    const analysis = analyzeScannedArchive(archive, {
      ...baseProfile,
      engine: "unreal"
    });

    expect(analysis.recommendedPlan?.adapterId).toBe("unreal-pak");
    expect(analysis.recommendedPlan?.mappings[0].targetRelativePath).toBe(
      "Content/Paks/~mods/MyMod.pak"
    );
  });

  it("targets Windrose pak mods to the R5 pak folder", () => {
    const archive = fakeArchive({
      entries: ["More_Fast-Travel_and_Bonfires_P.pak"]
    });

    const analysis = analyzeScannedArchive(archive, {
      ...baseProfile,
      name: "Windrose",
      gamePath: "D:/Steam/steamapps/common/Windrose",
      gameId: "windrose",
      engine: "unreal"
    });

    expect(analysis.recommendedPlan?.adapterId).toBe("unreal-pak");
    expect(analysis.recommendedPlan?.mappings.map((mapping) => mapping.targetRelativePath)).toContain(
      "R5/Content/Paks/~mods/More_Fast-Travel_and_Bonfires_P.pak"
    );
    expect(analysis.recommendedPlan?.mappings.map((mapping) => mapping.targetRelativePath)).toContain(
      "R5/Builds/WindowsServer/R5/Content/Paks/~mods/More_Fast-Travel_and_Bonfires_P.pak"
    );
  });

  it("detects UE4SS runtime archives without treating UE4SS.dll as a BepInEx plugin", () => {
    const archive = fakeArchive({
      entries: ["UE4SS.dll", "UE4SS-settings.ini", "dwmapi.dll", "Mods/mods.txt"]
    });

    const analysis = analyzeScannedArchive(archive, {
      ...baseProfile,
      engine: "unreal"
    });

    expect(analysis.recommendedPlan?.adapterId).toBe("ue4ss");
    expect(analysis.recommendedPlan?.mappings.map((mapping) => mapping.targetRelativePath)).toContain(
      "Binaries/Win64/UE4SS.dll"
    );
  });

  it("detects REFramework runtime archives without treating dinput8.dll as a BepInEx plugin", () => {
    const archive = fakeArchive({
      entries: ["dinput8.dll", "reframework_revision.txt"]
    });

    const analysis = analyzeScannedArchive(archive, {
      ...baseProfile,
      engine: "re-engine"
    });

    expect(analysis.recommendedPlan?.adapterId).toBe("reframework");
    expect(analysis.recommendedPlan?.mappings.map((mapping) => mapping.targetRelativePath)).toContain(
      "dinput8.dll"
    );
  });

  it("falls back to profile staging for unrecognized loose files", () => {
    const archive = fakeArchive({
      entries: ["Textures/custom.bin"]
    });

    const analysis = analyzeScannedArchive(archive, baseProfile);

    expect(analysis.recommendedPlan?.adapterId).toBe("loose-files");
    expect(analysis.recommendedPlan?.requiresConfirmation).toBe(true);
    expect(analysis.recommendedPlan?.mappings[0].targetRoot).toBe("profile");
  });
});

function fakeArchive(input: {
  entries: string[];
  manifestDependencies?: string[];
}): ScannedZipArchive {
  return {
    archivePath: "C:/Downloads/CoolMod.zip",
    archiveName: "CoolMod.zip",
    entries: input.entries.map((entry) => ({
      path: entry,
      logicalPath: entry.replace(/^CoolPak\//, ""),
      size: 128,
      isDirectory: false
    })),
    manifest: input.manifestDependencies
      ? {
          name: "CoolMod",
          version_number: "1.0.0",
          dependencies: input.manifestDependencies
        }
      : undefined,
    zip: {} as ScannedZipArchive["zip"]
  };
}
