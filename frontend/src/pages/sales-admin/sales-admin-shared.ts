import { formatCoinFromMinor } from "@/lib/store-money";

/** Formats Coin minor units, keeping a negative balance signed rather than absolute. */
export function coin(minor: string): string {
  const negative = minor.startsWith("-");
  const formatted = formatCoinFromMinor(negative ? minor.slice(1) : minor, "CNY", "1");
  return negative ? `-${formatted}` : formatted;
}

/** Percent input stored as basis points, so no float ever touches a rate. */
export function percentToBasisPoints(value: string): number | null {
  const match = /^(\d{1,2})(?:\.(\d{1,2}))?$/.exec(value.trim());
  if (!match) return null;
  const whole = Number(match[1]);
  const fraction = (match[2] ?? "").padEnd(2, "0");
  return whole * 100 + Number(fraction);
}
