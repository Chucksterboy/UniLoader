import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { detectGameSetup } from "../src/core/detection/gameDetector";

let tempRoot = "";

beforeEach(async () => {
  tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "uniloader-detector-"));
});

afterEach(async () => {
  await fs.rm(tempRoot, { recursive: true, force: true });
});

describe("game setup detection", () => {
  it("detects Unity Mono and recommends BepInEx", async () => {
    await touch("Windrose.exe");
    await touch("UnityPlayer.dll");
    await touch("Windrose_Data/Managed/Assembly-CSharp.dll");
    await mkdir("MonoBleedingEdge");

    const result = await detectGameSetup(tempRoot);

    expect(result.engine).toBe("unity-mono");
    expect(result.loader).toBe("bepinex");
    expect(result.loaderInstalled).toBe(false);
    expect(result.createdModFolders).toEqual(
      expect.arrayContaining(["BepInEx/plugins", "BepInEx/config"])
    );
    await expect(fs.stat(path.join(tempRoot, "BepInEx/plugins"))).resolves.toBeDefined();
  });

  it("detects Unity IL2CPP and installed BepInEx IL2CPP", async () => {
    await touch("GameAssembly.dll");
    await touch("UnityPlayer.dll");
    await mkdir("Dragon_Data/il2cpp_data");
    await touch("BepInEx/core/BepInEx.dll");
    await mkdir("BepInEx/interop");

    const result = await detectGameSetup(tempRoot);

    expect(result.engine).toBe("unity-il2cpp");
    expect(result.loader).toBe("bepinex-il2cpp");
    expect(result.loaderInstalled).toBe(true);
  });

  it("detects nested Unreal layouts and recommends UE4SS", async () => {
    await touch("DragonWilds/Binaries/Win64/DragonWilds-Win64-Shipping.exe");
    await touch("DragonWilds/Content/Paks/DragonWilds.pak");

    const result = await detectGameSetup(tempRoot);

    expect(result.engine).toBe("unreal");
    expect(result.loader).toBe("ue4ss");
    expect(result.loaderInstalled).toBe(false);
    expect(result.createdModFolders).toEqual(
      expect.arrayContaining(["DragonWilds/Content/Paks/~mods", "DragonWilds/Binaries/Win64/Mods"])
    );
    await expect(fs.stat(path.join(tempRoot, "DragonWilds/Content/Paks/~mods"))).resolves.toBeDefined();
  });

  it("detects RE Engine and installed REFramework", async () => {
    await touch("re_chunk_000.pak");
    await touch("dinput8.dll");
    await mkdir("reframework/autorun");

    const result = await detectGameSetup(tempRoot);

    expect(result.engine).toBe("re-engine");
    expect(result.loader).toBe("reframework");
    expect(result.loaderInstalled).toBe(true);
  });
});

async function mkdir(relativePath: string): Promise<void> {
  await fs.mkdir(path.join(tempRoot, relativePath), { recursive: true });
}

async function touch(relativePath: string): Promise<void> {
  const targetPath = path.join(tempRoot, relativePath);
  await fs.mkdir(path.dirname(targetPath), { recursive: true });
  await fs.writeFile(targetPath, "");
}
