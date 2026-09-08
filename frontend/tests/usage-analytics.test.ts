import { describe, expect, test } from "bun:test";
import {
  aggregateTokenTotals,
  formatCacheHitRate,
  rankModelsByTokens,
  tokenMetricForBucket,
} from "../src/lib/usage-analytics";

const buckets = [
  {
    label: "first",
    input_tokens_by_model: { alpha: "10", beta: "3" },
    cache_read_tokens_by_model: { alpha: "2", beta: "1" },
    output_tokens_by_model: { alpha: "5", beta: "1" },
  },
  {
    label: "second",
    input_tokens_by_model: { alpha: "4" },
    cache_read_tokens_by_model: { alpha: "3" },
    output_tokens_by_model: { alpha: "2" },
  },
];

describe("Usage analytics helpers", () => {
  test("aggregates exact token totals without JavaScript Number", () => {
    expect(aggregateTokenTotals(buckets)).toEqual({
      input: 17n,
      cacheRead: 6n,
      output: 8n,
      total: 25n,
    });
    expect(tokenMetricForBucket(buckets[0], "total")).toBe(19n);
  });

  test("formats cache hit rate after exact rational arithmetic", () => {
    expect(formatCacheHitRate(17n, 6n)).toBe("35.3%");
    expect(formatCacheHitRate(0n, 0n)).toBe("—");
  });

  test("ranks model totals by exact value and then model name", () => {
    expect(rankModelsByTokens(buckets, "total")).toEqual([
      { model: "alpha", value: 21n },
      { model: "beta", value: 4n },
    ]);
  });

  test("treats cache-read tokens as an input detail, not an additional total", () => {
    expect(tokenMetricForBucket({
      label: "inclusive-input",
      input_tokens_by_model: { model: "100" },
      cache_read_tokens_by_model: { model: "90" },
      output_tokens_by_model: { model: "10" },
    }, "total")).toBe(110n);
    expect(formatCacheHitRate(100n, 90n)).toBe("90%");
  });

  test("never emits an empty model label", () => {
    expect(rankModelsByTokens([{
      label: "legacy",
      input_tokens_by_model: { "": "7" },
      cache_read_tokens_by_model: {},
      output_tokens_by_model: {},
    }], "total")).toEqual([{ model: "unknown", value: 7n }]);
  });

  test("retains integers above the JavaScript safe range", () => {
    expect(aggregateTokenTotals([{
      label: "large",
      input_tokens_by_model: { alpha: "9007199254740993" },
      cache_read_tokens_by_model: {},
      output_tokens_by_model: { alpha: "1" },
    }]).total).toBe(9_007_199_254_740_994n);
  });
});
