import { useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { PageHeader } from "@/components/ui/page-header";
import { api, type RequestLogsFilter } from "@/lib/api";
import { RequestLogsTable } from "../request-logs/request-logs-table";
import { formatCost } from "../request-logs/utils";

const ORG_LOGS_PAGE_SIZE = 100;

/** ORG-24: request logs across every key of this org, for every member. */
export function OrgLogsPage() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const [searchInput, setSearchInput] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [modelInput, setModelInput] = useState("");
  const [committedModel, setCommittedModel] = useState("");
  const [status, setStatus] = useState("all");
  const [pages, setPages] = useState(1);

  useEffect(() => {
    const handle = window.setTimeout(() => setDebouncedSearch(searchInput.trim()), 300);
    return () => window.clearTimeout(handle);
  }, [searchInput]);

  const filters = useMemo<RequestLogsFilter>(() => {
    const next: RequestLogsFilter = {};
    if (debouncedSearch) next.search = debouncedSearch;
    if (committedModel) next.model = committedModel;
    if (status !== "all") next.status = status;
    return next;
  }, [debouncedSearch, committedModel, status]);

  const limit = ORG_LOGS_PAGE_SIZE * pages;
  const query = useMemo(
    () => JSON.stringify(filters),
    [filters],
  );
  const { data, isLoading } = useSWR(
    orgId ? [`/api/dashboard/orgs/${orgId}/request-logs`, limit, query] : null,
    () => api.listOrgRequestLogs(orgId!, limit, 0, filters),
    { keepPreviousData: true },
  );

  const logs = useMemo(() => data?.data ?? [], [data]);
  const total = data?.total ?? 0;
  const emptyAffinityNames = useMemo(() => new Map<string, string>(), []);

  return (
    <PageWrapper className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
      <motion.div
        initial={{ opacity: 0, y: -10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
      >
        <PageHeader title={t("requestLogs.title")} description={t("requestLogs.description")} />
      </motion.div>

      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.05, ...transitions.normal }}
        className="space-y-1.5 rounded-lg border bg-card px-3 py-1.5"
      >
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative w-full sm:w-80">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="h-9 pl-10"
              placeholder={t("requestLogs.searchPlaceholder")}
              value={searchInput}
              onChange={(event) => {
                setSearchInput(event.target.value);
                setPages(1);
              }}
            />
          </div>
          <Input
            className="h-9 w-[200px] focus-visible:border-ring focus-visible:ring-0"
            placeholder={t("requestLogs.filterModelPlaceholder")}
            value={modelInput}
            onChange={(event) => setModelInput(event.target.value)}
            onBlur={() => {
              setCommittedModel(modelInput.trim());
              setPages(1);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                setCommittedModel(modelInput.trim());
                setPages(1);
              }
            }}
          />
          <Select
            value={status}
            onValueChange={(value) => {
              setStatus(value);
              setPages(1);
            }}
          >
            <SelectTrigger className="h-9 w-[130px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("requestLogs.allStatuses")}</SelectItem>
              <SelectItem value="pending">{t("requestLogs.pending")}</SelectItem>
              <SelectItem value="success">{t("requestLogs.success")}</SelectItem>
              <SelectItem value="client_gone">{t("requestLogs.clientGone")}</SelectItem>
              <SelectItem value="error">{t("requestLogs.error")}</SelectItem>
            </SelectContent>
          </Select>
          <div className="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
            {data ? (
              <>
                <span className="font-medium text-foreground">
                  {t("requestLogs.totalCost")}: {formatCost(data.total_charge_nano_usd)}
                </span>
                {t("requestLogs.showing", {
                  from: total === 0 ? 0 : 1,
                  to: Math.min(logs.length, total),
                  total,
                })}
              </>
            ) : (
              <Skeleton className="inline-block h-4 w-24" />
            )}
          </div>
        </div>
      </motion.div>

      <RequestLogsTable
        affinityTargetNames={emptyAffinityNames}
        isAdmin={false}
        isInitialLoading={isLoading && logs.length === 0}
        logs={logs}
        onLoadMore={() => {
          if (logs.length < total && !isLoading) setPages((current) => current + 1);
        }}
        onOpenCapture={() => {}}
        onTooltipOpenChange={() => {}}
        showIp={false}
        t={t}
      />
    </PageWrapper>
  );
}
