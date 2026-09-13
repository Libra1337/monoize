import useSWR, { type SWRConfiguration } from "swr";
import { api, type DashboardAnalytics } from "@/lib/api";

/** Analytics row visibility outside org spaces: UA-25 of dashboard-usage-analysis.spec.md. */
export type WorkspaceAnalyticsScope = "self" | "group" | "all";

/**
 * One data source for the usage pages: the org's shared keys when `orgId` is
 * set, the caller's own keys otherwise. The response shape is identical, so the
 * same chart components serve the workspace and the org space. The workspace
 * `scope` follows the caller's role where the page asks for it: `self` for a
 * member, `group` for an admin, and `all` (omitted parameter) for a super
 * admin. Org mode ignores the scope.
 */
export function useUsageAnalytics(
  orgId: string | undefined,
  buckets: number,
  rangeHours: number,
  config?: SWRConfiguration,
  scope: WorkspaceAnalyticsScope = "self",
) {
  const workspaceScope = scope === "all" ? undefined : scope;
  return useSWR<DashboardAnalytics>(
    orgId
      ? `/api/dashboard/orgs/${orgId}/analytics?buckets=${buckets}&range_hours=${rangeHours}`
      : `/dashboard/analytics?buckets=${buckets}&range_hours=${rangeHours}${workspaceScope ? `&scope=${workspaceScope}` : ""}`,
    () =>
      orgId
        ? api.getOrgAnalytics(orgId, buckets, rangeHours)
        : api.getDashboardAnalytics(buckets, rangeHours, workspaceScope),
    { keepPreviousData: true, refreshInterval: 2000, ...config },
  );
}
