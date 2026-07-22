import { useEffect, useState } from "react";

export type MotionPhase = "entering" | "exiting";

export interface MotionState<T> {
  className: string;
  phase: MotionPhase;
  value: T;
}

export interface MotionPresence<T> {
  className: string;
  phase: MotionPhase;
  value: T | null;
}

export const motionDurationMs = 180;

export function motionClassName(phase: MotionPhase): string {
  return phase === "exiting" ? "ui-motion-exit" : "ui-motion-enter";
}

export function useFadePresence<T>(
  value: T | null,
  durationMs = motionDurationMs
): MotionPresence<T> {
  const [renderedValue, setRenderedValue] = useState<T | null>(value);
  const [phase, setPhase] = useState<MotionPhase>("entering");

  useEffect(() => {
    if (value !== null) {
      setRenderedValue(value);
      setPhase("entering");
      return undefined;
    }

    if (renderedValue === null) {
      return undefined;
    }

    setPhase("exiting");
    const timeoutId = window.setTimeout(() => {
      setRenderedValue(null);
      setPhase("entering");
    }, durationMs);

    return () => window.clearTimeout(timeoutId);
  }, [durationMs, renderedValue, value]);

  return {
    className: motionClassName(phase),
    phase,
    value: renderedValue
  };
}

export function useFadeSwitch<T>(value: T, durationMs = motionDurationMs): MotionState<T> {
  const [renderedValue, setRenderedValue] = useState<T>(value);
  const [phase, setPhase] = useState<MotionPhase>("entering");

  useEffect(() => {
    if (Object.is(value, renderedValue)) {
      setPhase("entering");
      return undefined;
    }

    setPhase("exiting");
    const timeoutId = window.setTimeout(() => {
      setRenderedValue(value);
      setPhase("entering");
    }, durationMs);

    return () => window.clearTimeout(timeoutId);
  }, [durationMs, renderedValue, value]);

  return {
    className: motionClassName(phase),
    phase,
    value: renderedValue
  };
}
