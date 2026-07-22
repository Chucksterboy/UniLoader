import { describe, expect, it } from "vitest";
import { rememberBoundedSetValue } from "./boundedCache";

describe("rememberBoundedSetValue", () => {
  it("evicts the oldest entries and refreshes a reused value", () => {
    const values = new Set(["one", "two", "three"]);

    rememberBoundedSetValue(values, "one", 3);
    rememberBoundedSetValue(values, "four", 3);

    expect([...values]).toEqual(["three", "one", "four"]);
  });

  it("honors an empty cache bound", () => {
    const values = new Set<string>();

    rememberBoundedSetValue(values, "discarded", 0);

    expect(values.size).toBe(0);
  });
});
