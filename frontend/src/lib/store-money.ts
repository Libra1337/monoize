export type StoreCurrency = "CNY" | "USD";

const CANONICAL_MINOR = /^(0|[1-9][0-9]*)$/;
const CANONICAL_SIGNED_MINOR = /^-?(0|[1-9][0-9]*)$/;
const POSITIVE_DECIMAL = /^(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$/;

function parseNonnegativeDecimal(value: string): { numerator: bigint; denominator: bigint } {
  if (!POSITIVE_DECIMAL.test(value)) {
    throw new Error("value must be a non-negative decimal string");
  }
  const [whole, fractional = ""] = value.split(".");
  const denominator = 10n ** BigInt(fractional.length);
  return {
    numerator: BigInt(whole) * denominator + BigInt(fractional || "0"),
    denominator,
  };
}

function parseMinor(value: string): bigint {
  if (!CANONICAL_MINOR.test(value)) {
    throw new Error("amount must be a canonical minor-unit string");
  }
  return BigInt(value);
}

function parseSignedMinor(value: string): bigint {
  if (!CANONICAL_SIGNED_MINOR.test(value)) {
    throw new Error("amount must be a canonical signed minor-unit string");
  }
  return BigInt(value);
}

function parseRate(value: string): { numerator: bigint; denominator: bigint } {
  if (!POSITIVE_DECIMAL.test(value)) {
    throw new Error("exchange rate must be a positive decimal string");
  }

  const [whole, fractional = ""] = value.split(".");
  const denominator = 10n ** BigInt(fractional.length);
  const numerator = BigInt(whole) * denominator + BigInt(fractional || "0");
  if (numerator === 0n) {
    throw new Error("exchange rate must be greater than zero");
  }
  return { numerator, denominator };
}

function divideRoundHalfAwayFromZero(numerator: bigint, denominator: bigint): bigint {
  if (denominator <= 0n) {
    throw new Error("denominator must be greater than zero");
  }

  const negative = numerator < 0n;
  const absolute = negative ? -numerator : numerator;
  let quotient = absolute / denominator;
  const remainder = absolute % denominator;
  if (remainder * 2n >= denominator) {
    quotient += 1n;
  }
  return negative ? -quotient : quotient;
}

export function formatMinor(minor: string, currency: StoreCurrency): string {
  const amount = parseMinor(minor);
  const whole = amount / 100n;
  const fraction = (amount % 100n).toString().padStart(2, "0");
  return `${currency === "CNY" ? "¥" : "$"}${whole}.${fraction}`;
}

function formatSignedMinor(minor: bigint, currency: StoreCurrency): string {
  const negative = minor < 0n;
  const formatted = formatMinor((negative ? -minor : minor).toString(), currency);
  return negative ? `-${formatted}` : formatted;
}

function formatCoinMinor(minor: bigint): string {
  const negative = minor < 0n;
  const absolute = negative ? -minor : minor;
  const whole = absolute / 100n;
  const fraction = (absolute % 100n).toString().padStart(2, "0");
  return `${negative ? "-" : ""}C${whole}.${fraction}`;
}

/** Format a payment amount as Coin in the selected display currency. */
export function formatCoinFromMinorForCurrency(
  minor: string,
  sourceCurrency: StoreCurrency,
  displayCurrency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const amount = parseMinor(minor);
  const cnyMinor = sourceCurrency === "CNY"
    ? amount
    : BigInt(convertMinor(minor, "USD", "CNY", cnyPerUsd));
  const displayMinor = displayCurrency === "CNY"
    ? cnyMinor
    : sourceCurrency === "USD"
      ? amount
      : BigInt(convertMinor(cnyMinor.toString(), "CNY", "USD", cnyPerUsd));
  return formatCoinMinor(displayMinor);
}

/** Format a payment-currency minor amount as Coin (1 C = 1 CNY). */
export function formatCoinFromMinor(
  minor: string,
  currency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const amount = parseMinor(minor);
  const cnyMinor = currency === "CNY"
    ? amount
    : BigInt(convertMinor(minor, "USD", "CNY", cnyPerUsd));
  return formatCoinMinor(cnyMinor);
}

/** Format an internal nano-USD amount as Coin using the current CNY/USD snapshot. */
export function formatCoinFromNanoUsd(nanoUsd: string, cnyPerUsd: string): string {
  const nano = parseSignedMinor(nanoUsd);
  const rate = parseRate(cnyPerUsd);
  const cnyMinor = divideRoundHalfAwayFromZero(
    nano * rate.numerator,
    10_000_000n * rate.denominator,
  );
  return formatCoinMinor(cnyMinor);
}

/** Format an internal nano-USD amount as Coin in the selected display currency. */
export function formatCoinFromNanoUsdForCurrency(
  nanoUsd: string,
  displayCurrency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const nano = parseSignedMinor(nanoUsd);
  if (displayCurrency === "USD") {
    return formatCoinMinor(divideRoundHalfAwayFromZero(nano, 10_000_000n));
  }
  return formatCoinFromNanoUsd(nanoUsd, cnyPerUsd);
}

/** Format a USD-basis model rate as Coin in the selected display currency. */
export function formatCoinRate(
  nanoUsdPerUnit: string,
  unit: string,
  displayCurrency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const amount = parseNonnegativeDecimal(nanoUsdPerUnit);
  let numerator = amount.numerator;
  let denominator = amount.denominator;
  if (displayCurrency === "CNY") {
    const exchange = parseRate(cnyPerUsd);
    numerator *= exchange.numerator;
    denominator *= exchange.denominator;
  }
  const multiplier = unit.toLowerCase() === "token" ? 1_000_000n : 1n;
  const minor = divideRoundHalfAwayFromZero(
    numerator * multiplier,
    denominator * 10_000_000n,
  );
  const suffix = unit.toLowerCase() === "token" ? " / 1M tokens" : ` / ${unit}`;
  return `${formatCoinMinor(minor)}${suffix}`;
}

/** Format a USD-basis model rate as Coin without applying the wallet FX rate. */
export function formatCoinPerMillionUsd(nanoUsdPerToken: string): string {
  const rate = parseNonnegativeDecimal(nanoUsdPerToken);
  const minor = divideRoundHalfAwayFromZero(
    rate.numerator * 1_000_000n,
    rate.denominator * 10_000_000n,
  );
  return `${formatCoinMinor(minor)} / 1M tokens`;
}

export function formatCoinDecimalUsd(nanoUsd: string, unit: string): string {
  const amount = parseNonnegativeDecimal(nanoUsd);
  const minor = divideRoundHalfAwayFromZero(
    amount.numerator,
    amount.denominator * 10_000_000n,
  );
  return `${formatCoinMinor(minor)} / ${unit}`;
}

export function convertMinor(
  minor: string,
  source: StoreCurrency,
  target: StoreCurrency,
  cnyPerUsd: string,
): string {
  const amount = parseMinor(minor);
  if (source === target) {
    return amount.toString();
  }

  const rate = parseRate(cnyPerUsd);
  const converted = source === "USD"
    ? divideRoundHalfAwayFromZero(amount * rate.numerator, rate.denominator)
    : divideRoundHalfAwayFromZero(amount * rate.denominator, rate.numerator);
  return converted.toString();
}

export function addMinor(left: string, right: string): string {
  return (parseMinor(left) + parseMinor(right)).toString();
}

export function formatPlanQuota(
  quotaFenCny: string,
  currency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const quota = parseMinor(quotaFenCny);
  if (currency === "CNY") {
    return `¥${divideRoundHalfAwayFromZero(quota, 100n)}`;
  }

  const rate = parseRate(cnyPerUsd);
  const wholeUsd = divideRoundHalfAwayFromZero(
    quota * rate.denominator,
    100n * rate.numerator,
  );
  return `$${wholeUsd}`;
}

/** Format a CNY-denominated plan quota as whole Coin units. */
export function formatPlanQuotaCoin(quotaFenCny: string): string {
  const quota = parseMinor(quotaFenCny);
  return `C${divideRoundHalfAwayFromZero(quota, 100n)}`;
}

const DECIMAL_MINOR_INPUT = /^(?:0|[1-9][0-9]*)(?:\.([0-9]{0,2}))?$/;

export function decimalToMinor(value: string): string | null {
  const normalized = value.trim();
  const match = DECIMAL_MINOR_INPUT.exec(normalized);
  if (!match) return null;
  const [whole, fraction = ""] = normalized.split(".");
  return (BigInt(whole) * 100n + BigInt(fraction.padEnd(2, "0") || "0")).toString();
}

export function minorToDecimal(minor: string): string {
  const amount = parseMinor(minor);
  const whole = amount / 100n;
  const fraction = (amount % 100n).toString().padStart(2, "0");
  return `${whole}.${fraction}`;
}

export function formatNanoUsd(
  nanoUsd: string,
  currency: StoreCurrency = "USD",
  cnyPerUsd = "1",
): string {
  const nano = parseSignedMinor(nanoUsd);
  const nanoPerMinor = 10_000_000n;
  if (currency === "USD") {
    return formatSignedMinor(divideRoundHalfAwayFromZero(nano, nanoPerMinor), "USD");
  }

  const rate = parseRate(cnyPerUsd);
  const cnyMinor = divideRoundHalfAwayFromZero(
    nano * rate.numerator,
    nanoPerMinor * rate.denominator,
  );
  return formatSignedMinor(cnyMinor, "CNY");
}

export function formatNanoUsdDecimal(
  nanoUsd: string,
  currency: StoreCurrency = "USD",
  cnyPerUsd = "1",
): string {
  const amount = parseNonnegativeDecimal(nanoUsd);
  const nanoPerMinor = 10_000_000n;
  let minor: bigint;
  if (currency === "USD") {
    minor = divideRoundHalfAwayFromZero(
      amount.numerator,
      amount.denominator * nanoPerMinor,
    );
  } else {
    const rate = parseRate(cnyPerUsd);
    minor = divideRoundHalfAwayFromZero(
      amount.numerator * rate.numerator,
      amount.denominator * nanoPerMinor * rate.denominator,
    );
  }
  return formatMinor(minor.toString(), currency);
}

export function formatPerMillionTokenRate(
  nanoUsdPerToken: string,
  currency: StoreCurrency,
  cnyPerUsd: string,
): string {
  const ratePerToken = parseNonnegativeDecimal(nanoUsdPerToken);
  const millionNanoNumerator = ratePerToken.numerator * 1_000_000n;
  let minor: bigint;
  if (currency === "USD") {
    minor = divideRoundHalfAwayFromZero(
      millionNanoNumerator,
      ratePerToken.denominator * 10_000_000n,
    );
  } else {
    const exchange = parseRate(cnyPerUsd);
    minor = divideRoundHalfAwayFromZero(
      millionNanoNumerator * exchange.numerator,
      ratePerToken.denominator * 10_000_000n * exchange.denominator,
    );
  }
  return `${formatMinor(minor.toString(), currency)} / 1M tokens`;
}
