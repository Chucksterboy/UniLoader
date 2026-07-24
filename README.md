# UniLoader

[Download releases](https://github.com/Chucksterboy/UniLoader/releases) |
[Privacy policy](PRIVACY.md) |
[Code signing policy](CODE_SIGNING.md) |
[Bundled asset licenses](ASSET_LICENSES.md) |
[MIT license](LICENSE)

UniLoader is a desktop mod manager prototype built around a practical universal model:

- One app and profile system.
- Per-game install adapters.
- Dependency planning before files are written.
- Automatic runtime dependency downloads from official providers.
- Safe installs with backups and uninstall records.
- Tauri shell with a React frontend and Rust backend commands.
- Managed per-profile package copies so mods can be disabled and re-enabled even if the original download is removed.
- Data-driven game definitions for known game IDs, runtime overrides, and config roots.
- Startup update checks against GitHub Releases, with a sidebar indicator when a newer release is available.
- Profile import/export bundles for sharing a profile's managed mods and config files with another player.

The first implementation supports ZIP archives, 7Z archives, and normal folder imports for these install families:

- BepInEx / Thunderstore-style Unity mods.
- UE4SS / Unreal mod structures.
- REFramework / RE Engine scripts and plugins.
- Generic Unreal `.pak` style mods.
- Loose-file fallback with warnings.

## Runtime Providers

UniLoader can automatically resolve these loader/runtime dependencies:

- Thunderstore package dependencies from `manifest.json`.
- Valheim BepInEx from Thunderstore (`denikson/BepInExPack_Valheim`).
- BepInEx Mono x64 from official GitHub releases.
- BepInEx IL2CPP x64 from official BepInBuilds bleeding-edge artifacts.
- UE4SS from official GitHub releases.
- REFramework from official GitHub releases, using game-specific stable assets when a supported RE Engine game is detected and the generic nightly asset as a fallback.

Source platforms such as Nexus Mods, CurseForge/Overwolf, Thunderstore, and Mod.io are not all equivalent. Thunderstore archives expose dependency metadata directly in the ZIP. Nexus and CurseForge integrations need their API/auth/download rules handled explicitly rather than scraped or bypassed.

### Runtime Registry Extensions

Bundled runtime definitions live in `src-tauri/src/runtime_definitions.json`. A local extension registry can add a runtime, or override a bundled runtime with the same `id`, without recompiling UniLoader:

```text
%APPDATA%\com.uniloader.desktop\registry\runtime-definitions.json
```

The extension file uses the same JSON array shape as the bundled registry. UniLoader validates runtime IDs, provider capabilities, detection marker types, and every relative detection path before accepting the registry. An invalid extension blocks startup with a specific registry error instead of silently falling back to unsafe or partial behavior. New provider resolvers still require an audited native implementation and an entry in `src-tauri/src/dependency_provider_definitions.json`; registry data cannot turn an unsupported provider into an automatic downloader.

## Run

UniLoader now uses Tauri, so running from source requires Rust/Cargo plus the bundled Codex Node runtime or a local Node.js install.

```powershell
pnpm install
pnpm dev
```

The Desktop shortcut points at `Start-UniLoader.cmd`. It starts a release build when one exists, otherwise it launches Tauri dev mode. If Rust is not installed, it will show a clear message instead of failing silently.

## Current State

This is a working foundation, not a finished public mod manager. It can create game profiles, detect game engines/loaders from a selected game folder, scan imported ZIP archives, detect common mod layouts, build an install plan, download supported runtime dependencies, and install files with backups. Nexus, CurseForge/Overwolf, and Mod.io source-provider integrations should be added through their official API/auth flows.

The React frontend lives in `src/renderer`. The Tauri/Rust backend lives in `src-tauri`.

The transfer tab exports a selected profile into a `.uniloader-profile` bundle. Importing that bundle prompts for the local game folder, recreates the profile, restores bundled config files, and redeploys enabled mods from the managed package copies.

## Releases and Updates

UniLoader displays its current app version in the bottom-left rail. On startup, it checks the latest GitHub Release for `Chucksterboy/UniLoader`. If a newer release exists, a pulsing download indicator appears above the health dot in the left rail.

Official Windows installers and their SHA-256 checksum files are published on
the [GitHub Releases page](https://github.com/Chucksterboy/UniLoader/releases).

To publish an installer build, push a version tag:

```powershell
git tag v0.2.1
git push origin v0.2.1
```

The GitHub Actions release workflow validates the project, builds the Windows
installer, publishes the release, and uploads SHA-256 checksum files.

## Future Game Support

Known-game rules live in `src-tauri/src/game_definitions.json`. Add future games there first:

- `executableNames` and `pathMarkers` identify the game.
- `bootstrapRuntimes` declares loaders UniLoader should install after profile creation.
- `runtimeDependencies` maps runtimes to official providers and release assets.
- `configRoots` adds game-specific config discovery paths.

Keep generic engine/loader detection in Rust, but prefer extending this manifest for game-specific behavior.

## Safety Rules

- Never bypass a platform's login, API rules, rate limits, DRM, or anti-cheat.
- Prefer game-specific adapters over blind file-copy installs.
- Back up overwritten files before deploying into a game directory.
- Resolve dependencies per game profile, while caching downloads globally.
- Store imported mod sources in UniLoader's profile package library before deployment.

## Privacy

UniLoader is local-first and does not include project-operated telemetry,
analytics, advertising, accounts, or cloud synchronization. It does connect to
documented third-party services for updates, mod discovery, artwork, and
runtime downloads. See the [Privacy Policy](PRIVACY.md) for the complete data
and network disclosure.

## Code Signing Policy

Official Windows releases follow the project's
[Code signing policy](CODE_SIGNING.md).

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

## License

UniLoader is available under the [MIT License](LICENSE). Third-party
dependencies retain their respective licenses. Game artwork, provider content,
and mods fetched at runtime are not redistributed under the UniLoader license.
See [Bundled Asset Licenses](ASSET_LICENSES.md) for the provenance and license
of the notification sounds and application artwork included in release builds.
