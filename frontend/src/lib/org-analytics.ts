import useSWR, { type SWRConfiguration } from "swr";
import { api, type DashboardAnalytics } from "@/lib/api";
import { useAuth } from "@/hooks/use-auth";

/** Analytics row visibility outside org spaces: UA-25 of dashboard-usage-analysis.spec.md. */
export type WorkspaceAnalyticsScope = "self" | "group" | "all";

/** Resolves the workspace scope from the authenticated role when a page does not pin one. */
export function useWorkspaceAnalyticsScope(): WorkspaceAnalyticsScope {
  const { user } = useAuth();
  if (user?.role === "super_admin") return "all";
  if (user?.role === "admin") return "group";
  return "self";
}

/**
 * One data source for the usage pages: the org's shared keys when `orgId` is
 * set, the caller's own keys otherwise. The response shape is identical, so the
 * same chart components serve the workspace and the org space. The workspace
 * `scope` defaults to the caller's role per UA-25: `all` (omitted parameter)
 * for a super admin, `group` for an admin, `self` for a member. Org mode
 * ignores the scope.
 */
export function useUsageAnalytics(
  orgId: string | undefined,
  buckets: number,
  rangeHours: number,
  config?: SWRConfiguration,
  scope?: WorkspaceAnalyticsScope,
) {
  const roleScope = useWorkspaceAnalyticsScope();
  const resolvedScope = scope ?? roleScope;
  const workspaceScope = resolvedScope === "all" ? undefined : resolvedScope;
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
