export type StoreCurrency = "CNY" | "USD";

const CANONICAL_MINOR = /^(0|[1-9][0-9]*)$/;
const POSITIVE_DECIMAL = /^(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$/;

function parseMinor(value: string): bigint {
  if (!CANONICAL_MINOR.test(value)) {
    throw new Error("amount must be a canonical minor-unit string");
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
