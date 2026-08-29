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

describe("Dashboard navigation", () => {
  test("links the brand home and exposes the new Console pages", () => {
    expect(layoutSource).toContain('to="/"');
    expect(layoutSource).toContain('to: "/dashboard/usage"');
    expect(layoutSource).toContain('to: "/dashboard/marketplace"');
    expect(layoutSource).toContain('to: "/dashboard/api-docs"');
  });
});

describe("Dashboard page boundaries", () => {
  test("replaces the former Model Data and API Information panels with Token Usage", () => {
    expect(dashboardSource).toContain("TokenSummary");
    expect(dashboardSource).toContain("UsageTrendChart");
    expect(dashboardSource).not.toContain('"Model Data"');
    expect(dashboardSource).not.toContain('"API Information"');
  });

  test("provides separate Usage Analysis and Dashboard API Docs pages", () => {
    expect(usageSource).toContain("export function UsageAnalysisPage");
    expect(dashboardApiDocsSource).toContain("export function DashboardApiDocsPage");
  });
});
