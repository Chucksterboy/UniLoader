import type { GameProfile } from "../shared/contracts";

type ProfileHealth = Pick<GameProfile, "setupStatus">;

export function profileNeedsAttention(
  profile: ProfileHealth | null | undefined,
  folderConnected: boolean | null | undefined,
  warningCount: number
): boolean {
  if (!profile) {
    return false;
  }

  return (
    profile.setupStatus === "needs-action" ||
    profile.setupStatus === "failed" ||
    folderConnected === false ||
    warningCount > 0
  );
}
