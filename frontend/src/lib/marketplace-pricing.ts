import {
  formatNanoUsdDecimal,
  formatPerMillionTokenRate,
  type StoreCurrency,
} from "@/lib/store-money";

export interface MarketplaceRateRange {
  min: string;
  max: string;
  unit: string;
}

function isTokenUnit(unit: string): boolean {
  return unit.toLowerCase() === "token";
}

export function formatMarketplaceRate(
  nanoUsd: string,
  unit: string,
  currency: StoreCurrency,
  cnyPerUsd: string,
): string {
  if (isTokenUnit(unit)) {
    return formatPerMillionTokenRate(nanoUsd, currency, cnyPerUsd);
  }
  return `${formatNanoUsdDecimal(nanoUsd, currency, cnyPerUsd)} / ${unit}`;
}

export function formatMarketplaceRateRange(
  range: MarketplaceRateRange,
  currency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const minimum = formatMarketplaceRate(range.min, range.unit, currency, cnyPerUsd);
  if (range.min === range.max) return minimum;

  const maximum = formatMarketplaceRate(range.max, range.unit, currency, cnyPerUsd);
  const suffix = isTokenUnit(range.unit) ? " / 1M tokens" : ` / ${range.unit}`;
  return `${minimum.slice(0, -suffix.length)}–${maximum}`;
}
