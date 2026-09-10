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
  // input_tokens is an inclusive total; cache_read is a detail of that input.
  return input + output;
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
  return { ...totals, total: totals.input + totals.output };
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
  return input + output;
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
    for (const sourceModel of models) {
      const model = sourceModel.trim() || "unknown";
      totals.set(
        model,
        (totals.get(model) ?? 0n) + modelMetricValue(bucket, sourceModel, metric),
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

export type CacheHitGrade = "no_traffic" | "insufficient" | "low" | "partial" | "high";

export interface ModelCacheHitRate {
  model: string;
  input: bigint;
  cacheRead: bigint;
  basisPoints: bigint;
  grade: CacheHitGrade;
}

/// A hit rate measured over a smaller input total is dominated by the unavoidable
/// cache-miss cost of the first request in a conversation, so it carries no grade.
export const CACHE_HIT_GRADE_MIN_INPUT = 50_000n;
export const CACHE_HIT_LOW_BASIS_POINTS = 3_000n;
export const CACHE_HIT_HIGH_BASIS_POINTS = 6_000n;

function gradeCacheHitRate(input: bigint, basisPoints: bigint): CacheHitGrade {
  if (input < CACHE_HIT_GRADE_MIN_INPUT) return "insufficient";
  if (basisPoints < CACHE_HIT_LOW_BASIS_POINTS) return "low";
  if (basisPoints < CACHE_HIT_HIGH_BASIS_POINTS) return "partial";
  return "high";
}

/**
 * Ranks logical models by input Token volume and reports the prompt-cache hit rate of
 * each one. Input volume drives the ordering because it decides how much a low hit rate
 * actually costs. Rows with a zero input total are omitted: their hit rate is undefined.
 */
export function rankModelCacheHitRates(
  buckets: TokenAnalyticsBucket[],
): ModelCacheHitRate[] {
  const totals = new Map<string, { input: bigint; cacheRead: bigint }>();
  for (const bucket of buckets) {
    const models = new Set([
      ...Object.keys(bucket.input_tokens_by_model),
      ...Object.keys(bucket.cache_read_tokens_by_model),
    ]);
    for (const sourceModel of models) {
      const model = sourceModel.trim() || "unknown";
      const current = totals.get(model) ?? { input: 0n, cacheRead: 0n };
      totals.set(model, {
        input: current.input + modelMetricValue(bucket, sourceModel, "input"),
        cacheRead: current.cacheRead + modelMetricValue(bucket, sourceModel, "cache_read"),
      });
    }
  }
  return [...totals.entries()]
    .filter(([, value]) => value.input > 0n)
    .sort(([leftModel, left], [rightModel, right]) => (
      left.input === right.input
        ? compareUtf8(leftModel, rightModel)
        : left.input > right.input ? -1 : 1
    ))
    .map(([model, value]) => {
      const basisPoints = (value.cacheRead * 10_000n + value.input / 2n) / value.input;
      return {
        model,
        input: value.input,
        cacheRead: value.cacheRead,
        basisPoints,
        grade: gradeCacheHitRate(value.input, basisPoints),
      };
    });
}

/**
 * Ranks the models that carried traffic, then appends every remaining catalog model so a
 * model with no traffic in the range is still visible. A missing row and a zero-hit row look
 * identical to a reader, so the table has to distinguish them explicitly.
 *
 * `catalog` may contain duplicates and untrimmed names; both are normalized the same way
 * analytics model labels are.
 */
export function cacheHitRateTable(
  buckets: TokenAnalyticsBucket[],
  catalog: readonly string[],
): ModelCacheHitRate[] {
  const measured = rankModelCacheHitRates(buckets);
  const seen = new Set(measured.map((row) => row.model));
  const untracked = [...new Set(
    catalog.map((model) => model.trim() || "unknown").filter((model) => !seen.has(model)),
  )]
    .sort(compareUtf8)
    .map((model) => ({
      model,
      input: 0n,
      cacheRead: 0n,
      basisPoints: 0n,
      grade: "no_traffic" as CacheHitGrade,
    }));
  return [...measured, ...untracked];
}

export function formatCacheHitRate(input: bigint, cacheRead: bigint): string {
  if (input < 0n || cacheRead < 0n) {
    throw new Error("token counts must be non-negative");
  }
  if (input === 0n) return "—";
  const tenths = (cacheRead * 1_000n + input / 2n) / input;
  const whole = tenths / 10n;
  const fraction = tenths % 10n;
  return fraction === 0n ? `${whole}%` : `${whole}.${fraction}%`;
}

export function formatTokenCount(value: bigint, locale?: string): string {
  return value.toLocaleString(locale);
}
