import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";

function source(relativePath: string): string {
  const url = new URL(relativePath, import.meta.url);
  return existsSync(url) ? readFileSync(url, "utf8") : "";
}

const appSource = source("../src/App.tsx");
const layoutSource = source("../src/pages/layout.tsx");
const dashboardSource = source("../src/pages/dashboard.tsx");
const usageSource = source("../src/pages/usage-analysis.tsx");
const dashboardApiDocsSource = source("../src/pages/dashboard-api-docs.tsx");
const tokenSummarySource = source("../src/components/usage/token-summary.tsx");
const usageTrendSource = source("../src/components/usage/usage-trend-chart.tsx");
const modelDistributionSource = source("../src/components/usage/model-distribution.tsx");
const selectionDatasetSource = source("../src/hooks/use-selection-dataset.ts");
const publicStatusSource = source("../src/pages/public-status.tsx");
const apiKeysSource = source("../src/pages/api-keys.tsx");
const adminUsageSource = source("../src/pages/admin-usage.tsx");
const rankingBreakdownSource = source("../src/components/usage/ranking-token-breakdown.tsx");
const userSettingsSource = source("../src/pages/user-settings.tsx");
const publicUsageRankingSource = source("../src/pages/public-usage-ranking.tsx");
const apiSource = source("../src/lib/api.ts");
const userCenterSource = source("../src/components/user-center-menu.tsx");
const currencySource = source("../src/hooks/use-store-currency.tsx");
const locales = ["en", "zh", "zh-TW", "ja"].map((locale) =>
  JSON.parse(source(`../src/locales/${locale}.json`)),
);

describe("authenticated Dashboard route ownership", () => {
  test("mounts usage, Marketplace, and API Docs under DashboardLayout", () => {
    expect(appSource).toContain('path="usage"');
    expect(appSource).toContain('path="marketplace"');
    expect(appSource).toContain('path="api-docs"');
    expect(appSource).toContain("<UsageAnalysisPage />");
    expect(appSource).toContain("<ModelMarketplacePage />");
    expect(appSource).toContain("<DashboardApiDocsPage />");
  });

  test("keeps request logs at their existing independent route", () => {
    expect(appSource).toContain('path="logs" element={<RequestLogsPage />}');
    expect(usageSource).not.toContain("useRequestLogs");
    expect(usageSource).not.toContain("RequestLogsPage");
  });
});

describe("Public usage ranking", () => {
  test("offers 24-hour 7-day and 30-day anonymous rankings", () => {
    expect(appSource).toContain('path={PUBLIC_PATHS.usageRanking} element={<PublicUsageRankingPage />}');
    expect(publicUsageRankingSource).toContain('"24h"');
    expect(publicUsageRankingSource).toContain('"7d"');
    expect(publicUsageRankingSource).toContain('"30d"');
    expect(publicUsageRankingSource).toContain("RankingTokenBreakdown");
    expect(publicUsageRankingSource).not.toContain("username");
    for (const locale of locales) expect(locale.publicSite.nav.usageRanking).toBeString();
  });
});

describe("Public status", () => {
  test("renders real traffic timelines and model details in a modal", () => {
    expect(publicStatusSource).toContain("group.timeline");
    expect(publicStatusSource).toContain("group.models");
    expect(publicStatusSource).toContain("<Dialog");
    expect(publicStatusSource).not.toContain("<details");
    expect(publicStatusSource).toContain("overallInsufficient");
  });

  test("aligns the Dashboard refresh mark with the status title", () => {
    expect(publicStatusSource).toContain('dashboard ? "flex items-start justify-between gap-4"');
    expect(publicStatusSource).toContain('className="min-w-0 max-w-3xl"');
    expect(publicStatusSource).not.toContain('mb-4 flex justify-end');
  });
});

describe("Dashboard navigation", () => {
  test("links the brand home and exposes the new Console pages", () => {
    expect(layoutSource).toContain('to="/"');
    expect(layoutSource).toContain('to: "/dashboard/usage"');
    expect(layoutSource).toContain('to: "/dashboard/marketplace"');
    expect(layoutSource).toContain('to: "/dashboard/api-docs"');
  });

  test("exposes usage ranking to ordinary authenticated users", () => {
    expect(appSource).toContain('path="usage-ranking" element={<AdminUsagePage />}');
    expect(appSource).toContain('path="admin/usage" element={<AdminRoute><AdminUsagePage /></AdminRoute>}');
    const ordinaryNavigation = layoutSource.slice(
      layoutSource.indexOf("const navItems"),
      layoutSource.indexOf("const adminNavItems"),
    );
    expect(ordinaryNavigation).toContain('to: "/dashboard/usage-ranking"');
  });

  test("keeps ranking charge fields and exchange-rate reads administrator-only", () => {
    expect(apiSource).toContain("cost_nano_usd?: string");
    expect(apiSource).toContain("total_cost_nano_usd?: string");
    expect(adminUsageSource).toContain('const isAdmin = user?.role === "super_admin" || user?.role === "admin"');
    expect(adminUsageSource).toContain('useStoreExchangeRate(currency === "CNY" && isAdmin)');
    expect(adminUsageSource).toContain("{isAdmin ? (");
  });

  test("supports mutually consented ranking identity disclosure", () => {
    expect(apiSource).toContain("usage_ranking_anonymous: boolean");
    expect(userSettingsSource).toContain("usage_ranking_anonymous");
    expect(userSettingsSource).toContain("<Switch");
    expect(adminUsageSource).toContain('t("adminUsage.ranking")');
    expect(adminUsageSource).toContain("row.models?.length");
    for (const locale of locales) {
      expect(locale.userSettings.rankingPrivacy).toBeString();
      expect(locale.userSettings.rankingPrivacyDescription).toBeString();
      expect(locale.adminUsage.ranking).toBeString();
    }
    expect(locales[1].adminUsage.ranking).toBe("当前排名");
  });

  test("shows the ordinary viewer's returned-list rank or unranked state", () => {
    expect(apiSource).toContain("current_user_rank?: number | null");
    expect(adminUsageSource).toContain("data.current_user_rank != null");
    expect(adminUsageSource).toContain('t("adminUsage.currentRank")');
    expect(adminUsageSource).toContain('t("adminUsage.unranked")');
    for (const locale of locales) {
      expect(locale.adminUsage.currentRank).toBeString();
      expect(locale.adminUsage.unranked).toBeString();
    }
  });

  test("shows colored input cache-read and output details in both rankings", () => {
    expect(adminUsageSource).toContain("RankingTokenBreakdown");
    expect(adminUsageSource.match(/<RankingTokenBreakdown/g)?.length).toBe(2);
    expect(rankingBreakdownSource).toContain('className="text-primary"');
    expect(rankingBreakdownSource).toContain('className="text-warning-foreground"');
    expect(rankingBreakdownSource).toContain('className="text-success"');
    expect(rankingBreakdownSource).toContain("<AnimatedTokenValue value={integer(input)}");
    expect(rankingBreakdownSource).toContain("<AnimatedTokenValue value={integer(cacheRead)}");
    expect(rankingBreakdownSource).toContain("<AnimatedTokenValue value={integer(output)}");
  });

  test("adds one persisted CNY and USD display control to the account menu", () => {
    expect(userCenterSource).toContain("useStoreCurrency()");
    expect(userCenterSource).toContain('layoutId="currency-toggle-indicator"');
    expect(userCenterSource).toContain("aria-pressed={currency === item}");
    expect(userCenterSource).toContain('["CNY", "USD"]');
    expect(currencySource).toContain('STORE_CURRENCY_STORAGE_KEY = "monoize-display-currency-v1"');
    expect(currencySource).toContain("localStorage.getItem");
    expect(currencySource).toContain("localStorage.setItem");
    expect(userCenterSource).not.toContain('cnyPerUsd ?? "1"');
    expect(dashboardSource).not.toContain('cnyPerUsd ?? "1"');
    for (const locale of locales) {
      expect(locale.userMenu.displayCurrency).toBeString();
      expect(locale.userMenu.cny).toBeString();
      expect(locale.userMenu.usd).toBeString();
    }
  });
});

describe("API key routing controls", () => {
  test("uses all Groups for an empty selection and requires ambiguous Channel bindings", () => {
    expect(apiKeysSource).not.toContain("use_user_group");
    expect(apiKeysSource).toContain("useApiKeyChannelConflicts");
    expect(apiKeysSource).toContain("channel_bindings");
    expect(apiKeysSource).toContain("<Skeleton");
    expect(apiKeysSource).toContain("unresolvedChannelConflicts");
    expect(apiKeysSource).toContain("channelSelectionBlocked");
  });

  test("does not request the admin transform registry for ordinary users", () => {
    expect(apiKeysSource).toContain("isPaused: () => !canManageSystem");
  });
});

describe("Dashboard page boundaries", () => {
  test("replaces the former Model Data and API Information panels with Token Usage", () => {
    expect(dashboardSource).toContain("TokenSummary");
    expect(dashboardSource).toContain("UsageTrendChart");
    expect(dashboardSource).not.toContain('"Model Data"');
    expect(dashboardSource).not.toContain('"API Information"');
  });

  test("uses focused animated usage components with loading and reduced-motion states", () => {
    expect(tokenSummarySource).toContain("Skeleton");
    expect(tokenSummarySource).toContain("useReducedMotion");
    expect(tokenSummarySource).toContain("motion");
    expect(usageTrendSource).toContain("ChartContainer");
    expect(usageTrendSource).toContain("useReducedMotion");
    expect(modelDistributionSource).toContain("rankModelsByTokens");
    expect(modelDistributionSource).toContain("Skeleton");
    expect(tokenSummarySource).toContain("divide-x");
    expect(tokenSummarySource.match(/<Card(?:\s|>)/g)?.length).toBe(2);
  });

  test("animates chart geometry only for explicit selections", () => {
    expect(selectionDatasetSource).toContain("useSelectionDataset");
    expect(selectionDatasetSource).toContain("selectionKey");
    expect(selectionDatasetSource).toContain("pendingSelectionRef");
    expect(usageTrendSource).toContain("selectionKey");
    expect(usageTrendSource).toContain("useSelectionDataset");
    expect(usageTrendSource).toContain("isAnimationActive={animate}");
    expect(usageTrendSource).toContain("animationDuration={1200}");
    expect(usageTrendSource).toContain('animationEasing="ease-in-out"');
    expect(usageTrendSource).not.toContain("AnimatePresence");
    expect(usageTrendSource).not.toContain("opacity: 0.45, y: 6");
    expect(usageTrendSource).toContain("if (loading && !buckets)");
    expect(modelDistributionSource).toContain("selectionKey");
    expect(modelDistributionSource).toContain("useSelectionDataset");
    expect(modelDistributionSource).toContain("isAnimationActive={animate}");
    expect(modelDistributionSource).toContain("animationDuration={1000}");
    expect(dashboardSource).toContain("selectionKey={range}");
    expect(dashboardSource).toContain("usage.data?.buckets.length !== rangeConfig.buckets");
    expect(usageSource).toContain("analytics.data?.buckets.length !== config.buckets");
    expect(usageSource.match(/selectionKey=\{`\$\{range\}:\$\{metric\}`\}/g)?.length).toBe(2);
  });

  test("uses slow token interpolation for live totals and rankings", () => {
    expect(tokenSummarySource).toContain("duration: 7.2");
    expect(tokenSummarySource).toContain('ease: "easeInOut"');
  });

  test("renders transient Token deltas inline with all three segment values", () => {
    expect(tokenSummarySource).toContain("AnimatePresence");
    expect(tokenSummarySource).toContain('className="inline-flex items-baseline');
    expect(tokenSummarySource).not.toContain('className="inline-flex flex-col"');
    expect(tokenSummarySource).not.toContain("min-h-4");
    expect(tokenSummarySource).toContain("setDelta(next - cycleStartRef.current)");
    expect(adminUsageSource).toContain("<AnimatedTokenValue value={segment.value} showDelta />");
    expect(publicUsageRankingSource).toContain("<AnimatedTokenValue value={value} showDelta />");
  });

  test("keeps every model label visible without legend overflow", () => {
    expect(modelDistributionSource).toContain("grid-cols-[auto_minmax(0,1fr)]");
    expect(modelDistributionSource).toContain("[overflow-wrap:anywhere]");
    expect(modelDistributionSource).toContain("flex-wrap");
    expect(modelDistributionSource).toContain('className="grid items-center gap-6"');
    expect(modelDistributionSource).not.toContain("md:grid-cols");
    expect(modelDistributionSource).toContain("animationDuration={1000}");
    expect(modelDistributionSource).toContain("duration-[1000ms]");
    expect(modelDistributionSource).not.toContain("justify-between gap-4");
    expect(modelDistributionSource).not.toContain("truncate font-medium");
  });

  test("supports the approved Dashboard and Usage Analysis ranges", () => {
    expect(dashboardSource).toContain('"24h"');
    expect(dashboardSource).toContain('"week"');
    expect(dashboardSource).toContain('"month"');
    expect(usageSource).toContain('"24h"');
    expect(usageSource).toContain('"7d"');
    expect(usageSource).toContain('"30d"');
    expect(usageSource).toContain("useDashboardAnalytics(");
    expect(usageSource).toContain('"self"');
    expect(usageSource).toContain("refreshInterval: 2000");
  });

  test("provides separate Usage Analysis and Dashboard API Docs pages", () => {
    expect(usageSource).toContain("export function UsageAnalysisPage");
    expect(dashboardApiDocsSource).toContain("export function DashboardApiDocsPage");
  });
});
