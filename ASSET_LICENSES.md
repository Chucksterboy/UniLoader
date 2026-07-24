# Bundled Asset Licenses

This document records the provenance and license of non-code assets distributed
inside official UniLoader builds.

## Notification Sounds

The following files were synthesized specifically for UniLoader using
deterministic additive wave generation:

- `public/sounds/mod-install-success.wav`
- `public/sounds/mod-install-failed.wav`

They contain no samples, recordings, or third-party audio. Copyright (c) 2026
Chucksterboy. They are distributed under the repository's
[MIT License](LICENSE).

Their complete generator source is
`scripts/generate-notification-sounds.mjs`. Run `pnpm assets:sounds` from the
repository root to reproduce both WAV files.

## Application Artwork

The UniLoader logo, application icons, tray icon, and associated vector artwork
under `src-tauri/icons` are project-original artwork created specifically for
UniLoader. Copyright (c) 2026 Chucksterboy. They are distributed under the
repository's [MIT License](LICENSE) to the extent copyright applies.

## Runtime-Fetched Artwork

Game artwork, mod thumbnails, and provider content downloaded while UniLoader
is running are not bundled in the source release or installer. They remain
subject to the terms and rights of their respective providers and owners.

## Dependencies

Source-code dependencies retain their own licenses. Their inclusion in an
official release must remain compatible with the project's
[Code Signing Policy](CODE_SIGNING.md).
