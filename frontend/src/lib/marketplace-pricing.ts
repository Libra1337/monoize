import { formatCoinPerUnitCny } from "@/lib/store-money";

export interface MarketplaceRateRange {
  min: string;
  max: string;
  unit: string;
}

function isTokenUnit(unit: string): boolean {
  return unit.toLowerCase() === "token";
}

/**
 * Formats a published Marketplace rate as Coin.
 *
 * MM-P2a delivers `display_rate_nano` in nano-CNY, and CN-3 fixes `1 C = 1 CNY`, so no
 * exchange rate is applied here and the display-currency selection does not change the
 * amount.
 */
export function formatMarketplaceRate(nanoCny: string, unit: string): string {
  return formatCoinPerUnitCny(nanoCny, unit);
}

export function formatMarketplaceRateRange(range: MarketplaceRateRange): string {
  const minimum = formatMarketplaceRate(range.min, range.unit);
  if (range.min === range.max) return minimum;

  const maximum = formatMarketplaceRate(range.max, range.unit);
  const suffix = isTokenUnit(range.unit) ? " / 1M tokens" : ` / ${range.unit}`;
  return `${minimum.slice(0, -suffix.length)}–${maximum.replace(/^[¥$]/, "")}`;
}
