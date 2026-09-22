import { describe, expect, test } from "bun:test";
import { rederiveField } from "../src/components/SpendLimitsEditor";

describe("SpendLimitsEditor currency re-derivation", () => {
  const rate = "7.0";

  test("keeps a user edit by converting it through nano-USD", () => {
    // 14 CNY with rate 7 -> 2 USD.
    expect(rederiveField("14", "2000000000", "USD", "CNY", rate)).toBe("2");
    // 2 USD with rate 7 -> 14 CNY.
    expect(rederiveField("2", null, "CNY", "USD", rate)).toBe("14");
  });

  test("falls back to the stored window when the input is blank", () => {
    // The stored 3 USD limit renders as 3 after switching, not as a blank that
    // a later save would submit as an explicit unlimited.
    expect(rederiveField("", "3000000000", "CNY", "USD", rate)).toBe("21");
    expect(rederiveField("", "3000000000", "USD", "CNY", rate)).toBe("3");
  });

  test("stays blank when nothing is stored and nothing was typed", () => {
    expect(rederiveField("", null, "CNY", "USD", rate)).toBe("");
    expect(rederiveField("", undefined, "USD", "CNY", rate)).toBe("");
  });

  test("keeps an unparseable edit verbatim so the user can fix it", () => {
    expect(rederiveField("1.2.3", "0", "USD", "CNY", rate)).toBe("1.2.3");
  });
});
