import { describe, expect, it } from "vitest";
import { appendCapped } from "./logbuffer";

describe("appendCapped", () => {
  it("appends under the cap", () => {
    expect(appendCapped(["a"], ["b", "c"], 5)).toEqual(["a", "b", "c"]);
  });

  it("drops the oldest lines beyond the cap", () => {
    expect(appendCapped(["a", "b", "c"], ["d", "e"], 4)).toEqual(["b", "c", "d", "e"]);
  });

  it("keeps only the newest cap lines of a huge batch", () => {
    expect(appendCapped([], ["1", "2", "3", "4"], 2)).toEqual(["3", "4"]);
  });
});
