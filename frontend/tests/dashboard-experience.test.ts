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
    expect(usageTrendSource).toContain("isAnimationActive={!reduceMotion}");
    expect(modelDistributionSource).toContain("rankModelsByTokens");
    expect(modelDistributionSource).toContain("Skeleton");
    expect(tokenSummarySource).toContain("divide-x");
    expect(tokenSummarySource.match(/<Card(?:\s|>)/g)?.length).toBe(2);
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
