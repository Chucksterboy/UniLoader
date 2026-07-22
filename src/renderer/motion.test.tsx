import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  motionClassName,
  useFadePresence,
  useFadeSwitch
} from "./motion";

afterEach(() => {
  vi.useRealTimers();
});

describe("motion lifecycle", () => {
  it("retains presence until the exit duration completes", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ value }) => useFadePresence(value, 180),
      { initialProps: { value: "notice" as string | null } }
    );

    rerender({ value: null });
    expect(result.current.value).toBe("notice");
    expect(result.current.phase).toBe("exiting");

    act(() => vi.advanceTimersByTime(180));
    expect(result.current.value).toBeNull();
    expect(result.current.phase).toBe("entering");
  });

  it("switches content only after the old view exits", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ value }) => useFadeSwitch(value, 180),
      { initialProps: { value: "library" } }
    );

    rerender({ value: "discover" });
    expect(result.current.value).toBe("library");
    expect(result.current.className).toBe("ui-motion-exit");

    act(() => vi.advanceTimersByTime(180));
    expect(result.current.value).toBe("discover");
    expect(result.current.className).toBe("ui-motion-enter");
  });

  it("cancels stale transitions during rapid view changes", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ value }) => useFadeSwitch(value, 180),
      { initialProps: { value: "library" } }
    );

    rerender({ value: "discover" });
    rerender({ value: "settings" });
    act(() => vi.advanceTimersByTime(180));

    expect(result.current.value).toBe("settings");
    expect(result.current.phase).toBe("entering");
  });

  it("maps motion phases to stable CSS hooks", () => {
    expect(motionClassName("entering")).toBe("ui-motion-enter");
    expect(motionClassName("exiting")).toBe("ui-motion-exit");
  });
});
