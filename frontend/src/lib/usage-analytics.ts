export type TokenMetric = "total" | "input" | "cache_read" | "output";

export interface TokenAnalyticsBucket {
  label: string;
  input_tokens_by_model: Record<string, string>;
  cache_read_tokens_by_model: Record<string, string>;
  output_tokens_by_model: Record<string, string>;
}

export interface ExactTokenTotals {
  input: bigint;
  cacheRead: bigint;
  output: bigint;
  total: bigint;
}

const CANONICAL_TOKEN_COUNT = /^(0|[1-9][0-9]*)$/;

function parseTokenCount(value: string): bigint {
  if (!CANONICAL_TOKEN_COUNT.test(value)) {
    throw new Error("token count must be a canonical non-negative integer");
  }
  return BigInt(value);
}

function sumTokenMap(values: Record<string, string>): bigint {
  return Object.values(values).reduce(
    (total, value) => total + parseTokenCount(value),
    0n,
  );
}

export function tokenMetricForBucket(
  bucket: TokenAnalyticsBucket,
  metric: TokenMetric,
): bigint {
  const input = sumTokenMap(bucket.input_tokens_by_model);
  const cacheRead = sumTokenMap(bucket.cache_read_tokens_by_model);
  const output = sumTokenMap(bucket.output_tokens_by_model);
  if (metric === "input") return input;
  if (metric === "cache_read") return cacheRead;
  if (metric === "output") return output;
  return input + cacheRead + output;
}

export function aggregateTokenTotals(
  buckets: TokenAnalyticsBucket[],
): ExactTokenTotals {
  const totals = buckets.reduce(
    (current, bucket) => ({
      input: current.input + tokenMetricForBucket(bucket, "input"),
      cacheRead: current.cacheRead + tokenMetricForBucket(bucket, "cache_read"),
      output: current.output + tokenMetricForBucket(bucket, "output"),
    }),
    { input: 0n, cacheRead: 0n, output: 0n },
  );
  return { ...totals, total: totals.input + totals.cacheRead + totals.output };
}

function modelMetricValue(
  bucket: TokenAnalyticsBucket,
  model: string,
  metric: TokenMetric,
): bigint {
  const input = parseTokenCount(bucket.input_tokens_by_model[model] ?? "0");
  const cacheRead = parseTokenCount(bucket.cache_read_tokens_by_model[model] ?? "0");
  const output = parseTokenCount(bucket.output_tokens_by_model[model] ?? "0");
  if (metric === "input") return input;
  if (metric === "cache_read") return cacheRead;
  if (metric === "output") return output;
  return input + cacheRead + output;
}

function compareUtf8(left: string, right: string): number {
  const encoder = new TextEncoder();
  const leftBytes = encoder.encode(left);
  const rightBytes = encoder.encode(right);
  const length = Math.min(leftBytes.length, rightBytes.length);
  for (let index = 0; index < length; index += 1) {
    if (leftBytes[index] !== rightBytes[index]) {
      return leftBytes[index] - rightBytes[index];
    }
  }
  return leftBytes.length - rightBytes.length;
}

export function rankModelsByTokens(
  buckets: TokenAnalyticsBucket[],
  metric: TokenMetric,
): Array<{ model: string; value: bigint }> {
  const totals = new Map<string, bigint>();
  for (const bucket of buckets) {
    const models = new Set([
      ...Object.keys(bucket.input_tokens_by_model),
      ...Object.keys(bucket.cache_read_tokens_by_model),
      ...Object.keys(bucket.output_tokens_by_model),
    ]);
    for (const model of models) {
      totals.set(
        model,
        (totals.get(model) ?? 0n) + modelMetricValue(bucket, model, metric),
      );
    }
  }
  return [...totals.entries()]
    .filter(([, value]) => value > 0n)
    .sort(([leftModel, leftValue], [rightModel, rightValue]) => (
      leftValue === rightValue
        ? compareUtf8(leftModel, rightModel)
        : leftValue > rightValue ? -1 : 1
    ))
    .map(([model, value]) => ({ model, value }));
}

export function formatCacheHitRate(input: bigint, cacheRead: bigint): string {
  if (input < 0n || cacheRead < 0n) {
    throw new Error("token counts must be non-negative");
  }
  const denominator = input + cacheRead;
  if (denominator === 0n) return "—";
  const tenths = (cacheRead * 1_000n + denominator / 2n) / denominator;
  const whole = tenths / 10n;
  const fraction = tenths % 10n;
  return fraction === 0n ? `${whole}%` : `${whole}.${fraction}%`;
}

export function formatTokenCount(value: bigint, locale?: string): string {
  return value.toLocaleString(locale);
}
