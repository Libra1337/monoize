import { parseRate } from "@/lib/store-money";

/**
 * ORGL-3/18: the three spend-limit windows shared by org keys and personal
 * keys. Inputs are USD amounts; storage is canonical nano-USD strings where
 * null means unlimited.
 */
export type SpendWindowKey = "total_nano_usd" | "hourly_nano_usd" | "daily_nano_usd";

export const SPEND_WINDOWS: { key: SpendWindowKey; labelKey: string }[] = [
  { key: "total_nano_usd", labelKey: "orgLimits.windowTotal" },
  { key: "hourly_nano_usd", labelKey: "orgLimits.windowHourly" },
  { key: "daily_nano_usd", labelKey: "orgLimits.windowDaily" },
];

/** USD input -> nano-USD string; empty -> null (unlimited); invalid -> undefined (reject). */
export function usdToNanoLimit(raw: string): string | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const usd = Number(trimmed);
  if (!Number.isFinite(usd) || usd < 0) return undefined;
  return String(Math.round(usd * 1_000_000_000));
}

export function nanoToUsdInput(nano: string | null | undefined): string {
  if (!nano) return "";
  try {
    return String(Number(BigInt(nano)) / 1_000_000_000);
  } catch {
    return "";
  }
}

/** A limit draft typed in the editor's current currency. */
export type SpendLimitDraft = { total: string; hourly: string; daily: string };

/** Parses a non-negative decimal amount ("12", "0.5") into nano units (1e9). */
function parseAmountToNano(raw: string): bigint | null {
  const trimmed = raw.trim();
  if (!/^\d+(?:\.\d{1,9})?$/.test(trimmed)) return null;
  const [whole, frac = ""] = trimmed.split(".");
  const padded = frac.padEnd(9, "0");
  return BigInt(whole) * 1_000_000_000n + BigInt(padded);
}

function formatNanoAmount(nano: bigint): string {
  const whole = nano / 1_000_000_000n;
  const frac = (nano % 1_000_000_000n).toString().padStart(9, "0").replace(/0+$/, "");
  return frac ? `${whole}.${frac}` : `${whole}`;
}

/**
 * ORGL-20: converts a typed amount in the chosen currency to a canonical
 * nano-USD string. USD passes through; CNY divides by the cny_per_usd
 * snapshot with BigInt-only arithmetic (the same contract as the wallet's
 * CNY top-up). Returns null for an empty field and undefined for an invalid
 * amount or a missing rate.
 */
export function amountToNanoLimit(
  raw: string,
  currency: "USD" | "CNY",
  cnyPerUsd: string | undefined,
): string | null | undefined {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const nano = parseAmountToNano(trimmed);
  if (nano === null) return undefined;
  if (currency === "USD") return nano.toString();
  const rate = parseSpendLimitRate(cnyPerUsd);
  if (!rate) return undefined;
  const numerator = nano * rate.denominator;
  const quotient = numerator / rate.numerator;
  const remainder = numerator % rate.numerator;
  const roundUp = remainder * 2n >= rate.numerator;
  return (roundUp ? quotient + 1n : quotient).toString();
}

/** Renders a stored nano-USD window in the editor's current currency. */
export function nanoToLimitInput(
  nano: string | null | undefined,
  currency: "USD" | "CNY",
  cnyPerUsd: string | undefined,
): string {
  if (!nano) return "";
  let value: bigint;
  try {
    value = BigInt(nano);
  } catch {
    return "";
  }
  if (currency === "USD") {
    return formatNanoAmount(value);
  }
  const rate = parseSpendLimitRate(cnyPerUsd);
  if (!rate) return "";
  const numerator = value * rate.numerator;
  const quotient = numerator / rate.denominator;
  const remainder = numerator % rate.denominator;
  const cnyNano = remainder * 2n >= rate.denominator ? quotient + 1n : quotient;
  return formatNanoAmount(cnyNano);
}

/** Returns the exact positive rate, or undefined when the snapshot is unusable. */
export function parseSpendLimitRate(value: string | undefined): ReturnType<typeof parseRate> | undefined {
  if (value === undefined) return undefined;
  try {
    return parseRate(value);
  } catch {
    return undefined;
  }
}

/**
 * Re-derives one draft field for a currency switch. A user edit survives the
 * switch via old-currency → nano → next-currency conversion; an empty field
 * falls back to the stored window (empty only when the stored limit is truly
 * unlimited). When no stored window is available an empty field stays empty.
 */
export function rederiveField(
  raw: string,
  storedNano: string | null | undefined,
  next: "USD" | "CNY",
  current: "USD" | "CNY",
  cnyPerUsd: string | undefined,
): string {
  if (raw.trim() !== "") {
    const nano = amountToNanoLimit(raw, current, cnyPerUsd);
    if (nano != null) {
      return nanoToLimitInput(nano, next, cnyPerUsd);
    }
    return raw;
  }
  return nanoToLimitInput(storedNano ?? null, next, cnyPerUsd);
}

/** ORGL-20 payload build in the chosen currency; see amountToNanoLimit for semantics. */
export function buildSpendLimitPayloadIn(
  draft: SpendLimitDraft,
  currency: "USD" | "CNY",
  cnyPerUsd: string | undefined,
  alwaysSubmit: boolean,
): Pick<
  import("@/lib/api").CreateApiKeyInput,
  "spend_limit_total_nano_usd" | "spend_limit_hourly_nano_usd" | "spend_limit_daily_nano_usd"
> {
  const payload: Record<string, string> = {};
  for (const window of ["total", "hourly", "daily"] as const) {
    const nano = amountToNanoLimit(draft[window], currency, cnyPerUsd);
    if (nano === undefined) {
      throw new Error(tLimitError(window));
    }
    if (nano !== null || alwaysSubmit) {
      payload[`spend_limit_${window}_nano_usd`] = nano ?? "";
    }
  }
  return payload as Pick<
    import("@/lib/api").CreateApiKeyInput,
    "spend_limit_total_nano_usd" | "spend_limit_hourly_nano_usd" | "spend_limit_daily_nano_usd"
  >;
}

function tLimitError(window: string): string {
  return `Spend limit (${window}) must be a non-negative amount`;
}
