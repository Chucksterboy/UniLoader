import { describe, expect, it } from "vitest";
import { profileNeedsAttention } from "./health";

describe("profileNeedsAttention", () => {
  it("keeps a ready connected profile healthy", () => {
    expect(profileNeedsAttention({ setupStatus: "ready" }, true, 0)).toBe(false);
  });

  it.each([
    [{ setupStatus: "needs-action" } as const, true, 0],
    [{ setupStatus: "failed" } as const, true, 0],
    [{ setupStatus: "ready" } as const, false, 0],
    [{ setupStatus: "ready" } as const, true, 1]
  ])("flags actionable profile state", (profile, connected, warnings) => {
    expect(profileNeedsAttention(profile, connected, warnings)).toBe(true);
  });

  it("does not invent an error when no profile is selected", () => {
    expect(profileNeedsAttention(null, false, 3)).toBe(false);
  });
});
