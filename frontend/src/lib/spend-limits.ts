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
