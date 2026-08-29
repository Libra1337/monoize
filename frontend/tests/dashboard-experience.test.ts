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
const publicStatusSource = source("../src/pages/public-status.tsx");
const apiKeysSource = source("../src/pages/api-keys.tsx");
const adminUsageSource = source("../src/pages/admin-usage.tsx");
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

describe("Public status", () => {
  test("renders real traffic timelines and model details in a modal", () => {
    expect(publicStatusSource).toContain("group.timeline");
    expect(publicStatusSource).toContain("group.models");
    expect(publicStatusSource).toContain("<Dialog");
    expect(publicStatusSource).not.toContain("<details");
    expect(publicStatusSource).toContain("overallInsufficient");
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

  test("animates explicit trend selections without interpolating polled paths", () => {
    expect(usageTrendSource).toContain("transitionKey");
    expect(usageTrendSource).toContain("<motion.div");
    expect(usageTrendSource).toContain("<AnimatePresence");
    expect(usageTrendSource).toContain("mode=\"wait\"");
    expect(usageTrendSource).toContain("opacity: 0.45, y: 6");
    expect(usageTrendSource).toContain("duration: 0.65");
    expect(usageTrendSource).toContain("isAnimationActive={false}");
    expect(usageTrendSource).toContain("if (loading)");
    expect(usageTrendSource).not.toContain("isAnimationActive={!reduceMotion}");
    expect(dashboardSource).toContain("transitionKey={range}");
    expect(usageSource).toContain('transitionKey={`${range}:${metric}`}');
  });

  test("uses slow token interpolation for live totals and rankings", () => {
    expect(tokenSummarySource).toContain("duration: 7.2");
    expect(tokenSummarySource).toContain('ease: "easeInOut"');
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
