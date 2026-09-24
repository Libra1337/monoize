import { describe, expect, test } from "bun:test";
import {
  aggregateTokenTotals,
  cacheHitRateForTotals,
  cacheHitRateTable,
  formatCacheHitRate,
  rankGroupedModelCacheHitRates,
  rankModelCacheHitRates,
  rankModelsByTokens,
  tokenMetricForBucket,
} from "../src/lib/usage-analytics";

const buckets = [
  {
    label: "first",
    input_tokens_by_model: { alpha: "10", beta: "3" },
    cache_read_tokens_by_model: { alpha: "2", beta: "1" },
    output_tokens_by_model: { alpha: "5", beta: "1" },
    calls_by_model: { alpha: 4, beta: 1 },
  },
  {
    label: "second",
    input_tokens_by_model: { alpha: "4" },
    cache_read_tokens_by_model: { alpha: "3" },
    output_tokens_by_model: { alpha: "2" },
    calls_by_model: { alpha: 2 },
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

  test("ranks cache hit rates by input volume, not by rate", () => {
    expect(rankModelCacheHitRates([{
      label: "mixed",
      input_tokens_by_model: { busy: "200000", quiet: "60000" },
      cache_read_tokens_by_model: { busy: "20000", quiet: "54000" },
      output_tokens_by_model: {},
      calls_by_model: { busy: 30, quiet: 8 },
    }])).toEqual([
      { model: "busy", calls: 30n, input: 200_000n, cacheRead: 20_000n, basisPoints: 1_000n, grade: "low" },
      { model: "quiet", calls: 8n, input: 60_000n, cacheRead: 54_000n, basisPoints: 9_000n, grade: "high" },
    ]);
  });

  test("grades a hit rate only once the input total can support one", () => {
    const graded = rankModelCacheHitRates([{
      label: "grades",
      input_tokens_by_model: { small: "49999", floor: "50000", middle: "50001" },
      cache_read_tokens_by_model: { small: "0", floor: "14999", middle: "22501" },
      output_tokens_by_model: {},
      calls_by_model: { small: 1, floor: 1, middle: 1 },
    }]);
    expect(graded.map((row) => [row.model, row.basisPoints, row.grade])).toEqual([
      ["middle", 4_500n, "partial"],
      ["floor", 3_000n, "partial"],
      ["small", 0n, "insufficient"],
    ]);
  });

  test("omits a model with no input tokens and sums across buckets", () => {
    expect(rankModelCacheHitRates(buckets)).toEqual([
      { model: "alpha", calls: 6n, input: 14n, cacheRead: 5n, basisPoints: 3_571n, grade: "insufficient" },
      { model: "beta", calls: 1n, input: 3n, cacheRead: 1n, basisPoints: 3_333n, grade: "insufficient" },
    ]);
    expect(rankModelCacheHitRates([{
      label: "output-only",
      input_tokens_by_model: {},
      cache_read_tokens_by_model: {},
      output_tokens_by_model: { gamma: "9" },
    }])).toEqual([]);
  });

  test("keeps a called model that carries no token usage", () => {
    // A model whose every request-log row lacks usage (image/video upstreams)
    // stays ranked: it was called, so its hit rate is undefined, not absent.
    expect(rankModelCacheHitRates([{
      label: "usage-less",
      input_tokens_by_model: {},
      cache_read_tokens_by_model: {},
      output_tokens_by_model: {},
      calls_by_model: { imagen: 12 },
    }])).toEqual([
      { model: "imagen", calls: 12n, input: 0n, cacheRead: 0n, basisPoints: 0n, grade: "no_token_usage" },
    ]);
  });

  test("distinguishes no-token-usage from no traffic for pre-aggregated totals", () => {
    expect(cacheHitRateForTotals(0n, 0n, 5n).grade).toBe("no_token_usage");
    expect(cacheHitRateForTotals(0n, 0n, 0n).grade).toBe("no_traffic");
    expect(cacheHitRateForTotals(0n, 0n).grade).toBe("no_traffic");
  });

  test("lists every catalog model, measured rows first", () => {
    const table = cacheHitRateTable(buckets, ["zeta", "alpha", "  gamma  ", "beta"]);
    expect(table.map((row) => [row.model, row.grade])).toEqual([
      ["alpha", "insufficient"],
      ["beta", "insufficient"],
      ["gamma", "no_traffic"],
      ["zeta", "no_traffic"],
    ]);
    // UA-33: an untracked row carries no counts at all, so the page can render an em dash
    // rather than a zero that reads like a real measurement.
    const untracked = table.find((row) => row.model === "zeta");
    expect(untracked).toEqual({
      model: "zeta",
      calls: 0n,
      input: 0n,
      cacheRead: 0n,
      basisPoints: 0n,
      grade: "no_traffic",
    });
    expect(formatCacheHitRate(untracked!.input, untracked!.cacheRead)).toBe("—");
  });

  test("does not duplicate a catalog entry or a model that already has traffic", () => {
    const table = cacheHitRateTable(buckets, ["alpha", "alpha", "delta", "delta", ""]);
    expect(table.map((row) => row.model)).toEqual(["alpha", "beta", "delta", "unknown"]);
  });

  test("keeps the measured order when the catalog is empty", () => {
    expect(cacheHitRateTable(buckets, [])).toEqual(rankModelCacheHitRates(buckets));
  });

  test("retains integers above the JavaScript safe range", () => {
    expect(aggregateTokenTotals([{
      label: "large",
      input_tokens_by_model: { alpha: "9007199254740993" },
      cache_read_tokens_by_model: {},
      output_tokens_by_model: { alpha: "1" },
    }]).total).toBe(9_007_199_254_740_994n);
  });

  test("splits the same model across groups instead of diluting one rate (UA-27b)", () => {
    const sep = "\u2063";
    const buckets = [{
      label: "b",
      input_tokens_by_model: { glm: "200000" },
      cache_read_tokens_by_model: { glm: "100000" },
      output_tokens_by_model: {},
      calls_by_model: { glm: 10 },
      input_tokens_by_model_and_group: {
        [`low-group${sep}glm`]: "100000",
        [`high-group${sep}glm`]: "100000",
      },
      cache_read_tokens_by_model_and_group: {
        [`low-group${sep}glm`]: "1000",
        [`high-group${sep}glm`]: "99000",
      },
      calls_by_model_and_group: {
        [`low-group${sep}glm`]: 5,
        [`high-group${sep}glm`]: 5,
      },
    }];
    const rows = rankGroupedModelCacheHitRates(buckets);
    expect(rows).toHaveLength(2);
    const high = rows.find((row) => row.group === "high-group")!;
    const low = rows.find((row) => row.group === "low-group")!;
    // The merged model rate would be 50%, but the split rows carry 99% and 1%.
    expect(Number(high.basisPoints)).toBe(9900);
    expect(Number(low.basisPoints)).toBe(100);
    // Equal input sorts by Group name in byte order.
    expect(rows[0].group).toBe("high-group");
    expect(rows.map((row) => row.group)).toEqual(["high-group", "low-group"]);
  });

  test("falls back to unknown for keys without the separator", () => {
    const rows = rankGroupedModelCacheHitRates([{
      label: "b",
      input_tokens_by_model: {},
      cache_read_tokens_by_model: {},
      output_tokens_by_model: {},
      calls_by_model_and_group: { "legacy-key": 3 },
    }]);
    expect(rows).toEqual([{
      group: "unknown",
      model: "legacy-key",
      calls: 3n,
      input: 0n,
      cacheRead: 0n,
      basisPoints: 0n,
      grade: "no_token_usage",
    }]);
  });
});
