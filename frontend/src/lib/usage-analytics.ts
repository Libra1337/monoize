export type TokenMetric = "total" | "input" | "cache_read" | "output";

export interface TokenAnalyticsBucket {
  label: string;
  input_tokens_by_model: Record<string, string>;
  cache_read_tokens_by_model: Record<string, string>;
  output_tokens_by_model: Record<string, string>;
  /** Present in every dashboard analytics response; omitted by legacy fixtures. */
  calls_by_model?: Record<string, number>;
  /** UA-27a: Group-dimension maps keyed "<group>\u2063<model>". */
  calls_by_model_and_group?: Record<string, number>;
  input_tokens_by_model_and_group?: Record<string, string>;
  cache_read_tokens_by_model_and_group?: Record<string, string>;
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

export type CacheHitGrade =
  | "no_traffic"
  | "no_token_usage"
  | "insufficient"
  | "low"
  | "partial"
  | "high";

export interface ModelCacheHitRate {
  model: string;
  calls: bigint;
  input: bigint;
  cacheRead: bigint;
  basisPoints: bigint;
  grade: CacheHitGrade;
}

/** The key separator of the UA-27a Group-dimension maps. */
export const MODEL_GROUP_SEPARATOR = "\u2063";

export interface GroupedModelCacheHitRate {
  group: string;
  model: string;
  calls: bigint;
  input: bigint;
  cacheRead: bigint;
  basisPoints: bigint;
  grade: CacheHitGrade;
}

function splitGroupedKey(key: string): { group: string; model: string } {
  const at = key.indexOf(MODEL_GROUP_SEPARATOR);
  if (at < 0) return { group: "unknown", model: key };
  return { group: key.slice(0, at), model: key.slice(at + MODEL_GROUP_SEPARATOR.length) };
}

/// A hit rate measured over a smaller input total is dominated by the unavoidable
/// cache-miss cost of the first request in a conversation, so it carries no grade.
export const CACHE_HIT_GRADE_MIN_INPUT = 50_000n;
export const CACHE_HIT_LOW_BASIS_POINTS = 3_000n;
export const CACHE_HIT_HIGH_BASIS_POINTS = 6_000n;

function gradeCacheHitRate(
  input: bigint,
  calls: bigint,
  basisPoints: bigint,
): CacheHitGrade {
  // UA-34: calls without token usage are traffic whose hit rate is undefined,
  // which is distinct from no traffic at all.
  if (input <= 0n) return calls > 0n ? "no_token_usage" : "no_traffic";
  if (input < CACHE_HIT_GRADE_MIN_INPUT) return "insufficient";
  if (basisPoints < CACHE_HIT_LOW_BASIS_POINTS) return "low";
  if (basisPoints < CACHE_HIT_HIGH_BASIS_POINTS) return "partial";
  return "high";
}

/**
 * UA-32/UA-34 for a pre-aggregated pair of token totals (one user, one model,
 * any scope): basis points under half-away rounding and the matching grade.
 * `calls` distinguishes rows with traffic that carries no token usage.
 */
export function cacheHitRateForTotals(
  input: bigint,
  cacheRead: bigint,
  calls = 0n,
): { basisPoints: bigint; grade: CacheHitGrade } {
  if (input <= 0n) {
    return { basisPoints: 0n, grade: gradeCacheHitRate(input, calls, 0n) };
  }
  const basisPoints = (cacheRead * 10_000n + input / 2n) / input;
  return { basisPoints, grade: gradeCacheHitRate(input, calls, basisPoints) };
}

function bucketCalls(bucket: TokenAnalyticsBucket, model: string): bigint {
  return BigInt(bucket.calls_by_model?.[model] ?? 0);
}

/**
 * Ranks logical models that carried calls by input Token volume and reports the
 * prompt-cache hit rate of each one. Input volume drives the ordering because it
 * decides how much a low hit rate actually costs. A model whose rows carry no
 * token usage stays ranked (it was called, so its hit rate is undefined rather
 * than absent); a model with neither calls nor input tokens is omitted.
 */
export function rankModelCacheHitRates(
  buckets: TokenAnalyticsBucket[],
): ModelCacheHitRate[] {
  const totals = new Map<string, { input: bigint; cacheRead: bigint; calls: bigint }>();
  for (const bucket of buckets) {
    const models = new Set([
      ...Object.keys(bucket.input_tokens_by_model),
      ...Object.keys(bucket.cache_read_tokens_by_model),
      ...Object.keys(bucket.calls_by_model ?? {}),
    ]);
    for (const sourceModel of models) {
      const model = sourceModel.trim() || "unknown";
      const current = totals.get(model) ?? { input: 0n, cacheRead: 0n, calls: 0n };
      totals.set(model, {
        input: current.input + modelMetricValue(bucket, sourceModel, "input"),
        cacheRead: current.cacheRead + modelMetricValue(bucket, sourceModel, "cache_read"),
        calls: current.calls + bucketCalls(bucket, sourceModel),
      });
    }
  }
  return [...totals.entries()]
    .filter(([, value]) => value.input > 0n || value.calls > 0n)
    .sort(([leftModel, left], [rightModel, right]) => (
      left.input === right.input
        ? compareUtf8(leftModel, rightModel)
        : left.input > right.input ? -1 : 1
    ))
    .map(([model, value]) => {
      const basisPoints = value.input > 0n
        ? (value.cacheRead * 10_000n + value.input / 2n) / value.input
        : 0n;
      return {
        model,
        calls: value.calls,
        input: value.input,
        cacheRead: value.cacheRead,
        basisPoints,
        grade: gradeCacheHitRate(value.input, value.calls, basisPoints),
      };
    });
}

/**
 * UA-27b: ranks (Group, model) pairs that carried calls, so the same model in
 * two Groups renders two rows instead of merging into one diluted rate. Sorting
 * matches the model ranker: input descending, then Group and model in UTF-8
 * byte order.
 */
export function rankGroupedModelCacheHitRates(
  buckets: TokenAnalyticsBucket[],
): GroupedModelCacheHitRate[] {
  const totals = new Map<string, { group: string; model: string; input: bigint; cacheRead: bigint; calls: bigint }>();
  for (const bucket of buckets) {
    const keys = new Set([
      ...Object.keys(bucket.input_tokens_by_model_and_group ?? {}),
      ...Object.keys(bucket.cache_read_tokens_by_model_and_group ?? {}),
      ...Object.keys(bucket.calls_by_model_and_group ?? {}),
    ]);
    for (const key of keys) {
      const { group, model } = splitGroupedKey(key);
      const current = totals.get(key) ?? { group, model, input: 0n, cacheRead: 0n, calls: 0n };
      current.input += parseTokenCount(bucket.input_tokens_by_model_and_group?.[key] ?? "0");
      current.cacheRead += parseTokenCount(bucket.cache_read_tokens_by_model_and_group?.[key] ?? "0");
      current.calls += BigInt(bucket.calls_by_model_and_group?.[key] ?? 0);
      totals.set(key, current);
    }
  }
  return [...totals.values()]
    .filter((value) => value.input > 0n || value.calls > 0n)
    .sort((left, right) => {
      if (left.input !== right.input) return left.input > right.input ? -1 : 1;
      const byGroup = compareUtf8(left.group, right.group);
      if (byGroup !== 0) return byGroup;
      return compareUtf8(left.model, right.model);
    })
    .map((value) => {
      const basisPoints = value.input > 0n
        ? (value.cacheRead * 10_000n + value.input / 2n) / value.input
        : 0n;
      return {
        group: value.group,
        model: value.model,
        calls: value.calls,
        input: value.input,
        cacheRead: value.cacheRead,
        basisPoints,
        grade: gradeCacheHitRate(value.input, value.calls, basisPoints),
      };
    });
}

/**
 * Ranks the models that carried calls, then appends every remaining catalog
 * model so a model with no traffic in the range is still visible. A missing row
 * and a zero-hit row look identical to a reader, so the table has to
 * distinguish them explicitly.
 *
 * `catalog` may contain duplicates and untrimmed names; both are normalized the
 * same way analytics model labels are.
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
      calls: 0n,
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
