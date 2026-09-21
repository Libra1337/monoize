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
  const rate = Number(cnyPerUsd);
  if (!Number.isFinite(rate) || rate <= 0) return undefined;
  const scaledRate = BigInt(Math.round(rate * 1e9));
  const numerator = nano * 1_000_000_000n;
  const quotient = numerator / scaledRate;
  const remainder = numerator % scaledRate;
  const roundUp = remainder * 2n >= scaledRate;
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
    return String(Number(value) / 1_000_000_000);
  }
  const rate = Number(cnyPerUsd);
  if (!Number.isFinite(rate) || rate <= 0) return "";
  const scaledRate = BigInt(Math.round(rate * 1e9));
  const cnyNano = (value * scaledRate + 500_000_000n) / 1_000_000_000n;
  return formatNanoAmount(cnyNano);
}
