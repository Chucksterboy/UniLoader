import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";

const expectedAssets = new Set([
  "public/sounds/mod-install-failed.wav",
  "public/sounds/mod-install-success.wav",
  "src-tauri/icons/icon.ico",
  "src-tauri/icons/icon.png",
  "src-tauri/icons/tray-icon.png",
  "src-tauri/icons/tray-icon.svg",
  "src-tauri/icons/uniloader-blue.ico",
  "src-tauri/icons/vault-mark.svg"
]);

const mediaExtensions = new Set([
  ".flac",
  ".gif",
  ".ico",
  ".jpeg",
  ".jpg",
  ".m4a",
  ".mp3",
  ".mp4",
  ".ogg",
  ".png",
  ".svg",
  ".wav",
  ".webm",
  ".webp"
]);

function collectMedia(directory) {
  const collected = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      collected.push(...collectMedia(entryPath));
      continue;
    }
    if (mediaExtensions.has(path.extname(entry.name).toLowerCase())) {
      collected.push(entryPath.replaceAll("\\", "/"));
    }
  }
  return collected;
}

function assertFileSignature(filePath) {
  const bytes = readFileSync(filePath);
  if (bytes.length === 0 || statSync(filePath).size === 0) {
    throw new Error(`Bundled asset is empty: ${filePath}`);
  }

  switch (path.extname(filePath).toLowerCase()) {
    case ".wav":
      if (
        bytes.toString("ascii", 0, 4) !== "RIFF" ||
        bytes.toString("ascii", 8, 12) !== "WAVE"
      ) {
        throw new Error(`Invalid WAV signature: ${filePath}`);
      }
      break;
    case ".png":
      if (!bytes.subarray(0, 8).equals(Buffer.from("89504e470d0a1a0a", "hex"))) {
        throw new Error(`Invalid PNG signature: ${filePath}`);
      }
      break;
    case ".ico":
      if (!bytes.subarray(0, 4).equals(Buffer.from([0, 0, 1, 0]))) {
        throw new Error(`Invalid ICO signature: ${filePath}`);
      }
      break;
    case ".svg":
      if (!bytes.toString("utf8").includes("<svg")) {
        throw new Error(`Invalid SVG document: ${filePath}`);
      }
      break;
    default:
      break;
  }
}

const actualAssets = [
  ...collectMedia("public"),
  ...collectMedia(path.join("src-tauri", "icons"))
].sort();

const unexpected = actualAssets.filter((asset) => !expectedAssets.has(asset));
const missing = [...expectedAssets].filter(
  (asset) => !actualAssets.includes(asset)
);

if (unexpected.length > 0) {
  throw new Error(`Unlisted bundled assets:\n${unexpected.join("\n")}`);
}
if (missing.length > 0) {
  throw new Error(`Documented bundled assets are missing:\n${missing.join("\n")}`);
}

const assetLicenses = readFileSync("ASSET_LICENSES.md", "utf8");
for (const asset of actualAssets) {
  if (!assetLicenses.includes(asset) && !asset.startsWith("src-tauri/icons/")) {
    throw new Error(`Bundled asset is absent from ASSET_LICENSES.md: ${asset}`);
  }
  assertFileSignature(asset);
}

const appSource = readFileSync(path.join("src", "renderer", "App.tsx"), "utf8");
if (/mod-install-(?:success|failed)\.mp3/i.test(appSource)) {
  throw new Error("The application still references removed MP3 notification sounds.");
}
for (const sound of [
  "mod-install-success.wav",
  "mod-install-failed.wav"
]) {
  if (!appSource.includes(sound)) {
    throw new Error(`The application does not reference ${sound}.`);
  }
}

process.stdout.write(`Audited ${actualAssets.length} bundled media assets.\n`);
