import useSWR, { type SWRConfiguration } from "swr";
import { api, type DashboardAnalytics } from "@/lib/api";

/**
 * One data source for the usage pages: the org's shared keys when `orgId` is
 * set, the caller's own keys otherwise. The response shape is identical, so the
 * same chart components serve the workspace and the org space.
 */
export function useUsageAnalytics(
  orgId: string | undefined,
  buckets: number,
  rangeHours: number,
  config?: SWRConfiguration,
) {
  return useSWR<DashboardAnalytics>(
    orgId
      ? `/api/dashboard/orgs/${orgId}/analytics?buckets=${buckets}&range_hours=${rangeHours}`
      : `/dashboard/analytics?buckets=${buckets}&range_hours=${rangeHours}&scope=self`,
    () =>
      orgId
        ? api.getOrgAnalytics(orgId, buckets, rangeHours)
        : api.getDashboardAnalytics(buckets, rangeHours, "self"),
    { keepPreviousData: true, refreshInterval: 2000, ...config },
  );
}
