import { describe, expect, test } from "bun:test";
import { buildSpendLimitPayloadIn, rederiveField } from "../src/lib/spend-limits";
import { createInstance } from "i18next";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { SpendLimitsEditor } from "../src/components/SpendLimitsEditor";
import { amountToNanoLimit, nanoToLimitInput } from "../src/lib/spend-limits";

const testI18n = createInstance();
await testI18n.init({
  lng: "en",
  resources: { en: { translation: {
    spendLimitsEditor: { rateUnavailable: "Exchange rate unavailable" },
  } } },
});

function renderEditor(currency: "USD" | "CNY", cnyPerUsd: string | undefined) {
  return renderToStaticMarkup(createElement(I18nextProvider, { i18n: testI18n },
    createElement(SpendLimitsEditor, {
      idPrefix: "limit-test",
      draft: { total: "", hourly: "", daily: "" },
      currency,
      cnyPerUsd,
      onChange: () => undefined,
      onCurrencyChange: () => undefined,
    }),
  ));
}

describe("Spend limit exact currency conversion", () => {
  test.each([
    ["1", "0.0000000001", "10000000000000000000"],
    ["1", "1.0000000005", "1000000000"],
    ["0.000000001", "2", "1"],
    ["0.000000001", "3", "0"],
  ])("converts %s CNY at exact rate %s to %s nano-USD", (amount, rate, expected) => {
    expect(amountToNanoLimit(amount, "CNY", rate)).toBe(expected);
  });

  test.each([
    ["9007199254740993", "USD", undefined, "9007199.254740993"],
    ["1000000000000000001", "USD", undefined, "1000000000.000000001"],
    ["1", "USD", undefined, "0.000000001"],
    ["10000000000000000000", "CNY", "0.0000000001", "1"],
    ["1000000000", "CNY", "1.0000000004", "1"],
    ["10000000000000000000", "CNY", "1.0000000004", "10000000004"],
    ["1", "CNY", "0.5", "0.000000001"],
    ["1", "CNY", "0.4", "0"],
  ] as const)("renders %s nano-USD in %s at %s", (nano, currency, rate, expected) => {
    expect(nanoToLimitInput(nano, currency, rate)).toBe(expected);
  });

  test.each([undefined, "", "0", "-1", "1e2", "0x10", "Infinity", "NaN"])(
    "rejects unusable CNY rate %s without throwing", (rate) => {
      expect(amountToNanoLimit("1", "CNY", rate)).toBeUndefined();
      expect(nanoToLimitInput("1000000000", "CNY", rate)).toBe("");
    },
  );

  test("saves a small-rate CNY draft without losing the update clear fields", () => {
    expect(buildSpendLimitPayloadIn(
      { total: "1", hourly: "", daily: "0" }, "CNY", "0.0000000001", true,
    )).toEqual({
      spend_limit_total_nano_usd: "10000000000000000000",
      spend_limit_hourly_nano_usd: "",
      spend_limit_daily_nano_usd: "0",
    });
  });
});

describe("SpendLimitsEditor rate availability", () => {
  for (const currency of ["USD", "CNY"] as const) {
    test.each([undefined, "0", "1e2", "Infinity"])(
      `disables CNY with unusable rate %s while ${currency} is selected`, (rate) => {
        const html = renderEditor(currency, rate);
        expect(html.match(/<button\b[^>]*>CNY<\/button>/)?.[0]).toContain('disabled=""');
        expect(html.match(/<button\b[^>]*>USD<\/button>/)?.[0]).not.toContain('disabled=""');
        expect(html).toContain("Exchange rate unavailable");
      },
    );
  }

  test("enables CNY for a positive rate smaller than one nano", () => {
    const html = renderEditor("USD", "0.0000000001");
    expect(html.match(/<button\b[^>]*>CNY<\/button>/)?.[0]).not.toContain('disabled=""');
    expect(html).not.toContain("Exchange rate unavailable");
  });
});

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
