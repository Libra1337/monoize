import type { BillingRateRecord, RateCurrency } from "@/lib/api";
import { formatNanoPerTokenPerMillion } from "@/lib/exact-decimal";

/** The usage classes the master list summarizes per model. */
export type PriceUsageClass =
  | "input_uncached"
  | "cache_read"
  | "cache_write_5m"
  | "cache_write_1h"
  | "output";

const SUMMARY_CLASSES: PriceUsageClass[] = [
  "input_uncached",
  "cache_read",
  "cache_write_5m",
  "cache_write_1h",
  "output",
];

export interface ModelPriceSummary {
  /** The manual rate currency when a manual override exists, else the synced rate currency. */
  currency: RateCurrency;
  /** "入 $2.5 · 出 $10" style primary line; null when the model has no price rows. */
  primary: string | null;
  /** Muted detail line with cache lanes and peak markers; null when nothing extra exists. */
  detail: string | null;
  /** True when at least one effective rate row is manual (an override over sync). */
  isManual: boolean;
  /** True when any effective row carries a peak price. */
  hasPeak: boolean;
  /** Number of distinct usage classes with an effective row. */
  pricedClasses: number;
}

/** Manual rows outrank synced rows, then priority, matching the engine's find_rate. */
function effectiveRate(
  rates: BillingRateRecord[],
  usageClass: PriceUsageClass
): BillingRateRecord | undefined {
  return rates
    .filter(
      (rate) =>
        rate.enabled && rate.rate_kind === "token" && rate.usage_class === usageClass
    )
    .sort((a, b) => {
      if (a.source === "manual" && b.source !== "manual") return -1;
      if (b.source === "manual" && a.source !== "manual") return 1;
      return b.priority - a.priority;
    })[0];
}

/**
 * UI17a: collapses a model's effective rate rows into the two-line summary the
 * master list renders — one column instead of five numeric columns.
 */
export function summarizeModelPrices(
  rates: BillingRateRecord[]
): ModelPriceSummary {
  const effective = SUMMARY_CLASSES.map((usageClass) => ({
    usageClass,
    rate: effectiveRate(rates, usageClass),
  })).filter((entry): entry is { usageClass: PriceUsageClass; rate: BillingRateRecord } =>
    Boolean(entry.rate)
  );

  if (effective.length === 0) {
    return {
      currency: "CNY",
      primary: null,
      detail: null,
      isManual: false,
      hasPeak: false,
      pricedClasses: 0,
    };
  }

  const input = effective.find((e) => e.usageClass === "input_uncached");
  const output = effective.find((e) => e.usageClass === "output");
  const currency: RateCurrency = (input ?? output ?? effective[0])!.rate
    .unit_price_currency;
  const fmt = (rate: BillingRateRecord) =>
    formatNanoPerTokenPerMillion(rate.unit_price_nano, rate.unit_price_currency);

  const parts: string[] = [];
  if (input) parts.push(`入 ${fmt(input.rate)}`);
  if (output) parts.push(`出 ${fmt(output.rate)}`);
  const primary = parts.length > 0 ? parts.join(" · ") : null;

  const detailParts: string[] = [];
  const cacheRead = effective.find((e) => e.usageClass === "cache_read");
  if (cacheRead) detailParts.push(`缓存读 ${fmt(cacheRead.rate)}`);
  const cache5m = effective.find((e) => e.usageClass === "cache_write_5m");
  if (cache5m) detailParts.push(`写5m ${fmt(cache5m.rate)}`);
  const cache1h = effective.find((e) => e.usageClass === "cache_write_1h");
  if (cache1h) detailParts.push(`写1h ${fmt(cache1h.rate)}`);

  return {
    currency,
    primary,
    detail: detailParts.length > 0 ? detailParts.join(" · ") : null,
    isManual: effective.some((e) => e.rate.source === "manual"),
    hasPeak: effective.some((e) => e.rate.peak_unit_price_nano != null),
    pricedClasses: effective.length,
  };
}

/** UI17a: badge label for the price column of a model with no rows. */
export const NO_PRICE_LABEL = "—";

export function priceSymbolLabel(currency: RateCurrency): string {
  return currency === "CNY" ? "CNY (¥)" : "USD ($)";
}
