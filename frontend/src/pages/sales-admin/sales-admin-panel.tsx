import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Skeleton } from "@/components/ui/skeleton";
import { salesApi } from "@/lib/sales-api";
import { SalesAdminAgents } from "./sales-admin-agents";
import { SalesAdminCommissions } from "./sales-admin-commissions";
import { SalesAdminWithdrawals } from "./sales-admin-withdrawals";

type AdminSalesTab = "agents" | "withdrawals" | "commissions";

const TABS: AdminSalesTab[] = ["agents", "withdrawals", "commissions"];

const AGENTS_KEY = "admin:sales:agents";
const WITHDRAWALS_KEY = "admin:sales:withdrawals";
const ENTRIES_KEY = "admin:sales:entries";
const SETTINGS_KEY = "admin:sales:settings";

/**
 * Admin sales management (SC-7).
 *
 * Three sub-pages rather than one column of six cards: roster management, the withdrawal
 * queue, and the commission ledger are separate tasks, and stacking them made the page
 * unreadable. Data is fetched once here so switching tabs does not refetch.
 */
export function SalesAdminPanel() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<AdminSalesTab>("agents");
  const agents = useSWR(AGENTS_KEY, () => salesApi.admin.listAgents());
  const withdrawals = useSWR(WITHDRAWALS_KEY, () => salesApi.admin.listWithdrawals());
  const entries = useSWR(ENTRIES_KEY, () => salesApi.admin.listEntries());
  const settings = useSWR(SETTINGS_KEY, () => salesApi.admin.getSettings());

  const pendingCount = (withdrawals.data ?? []).filter(
    (item) => item.state === "requested",
  ).length;

  if (agents.isLoading && !agents.data) {
    return (
      <div className="flex flex-col gap-4" aria-hidden="true">
        <Skeleton className="h-11 rounded-xl" />
        <Skeleton className="h-24 rounded-2xl" />
        <Skeleton className="h-64 rounded-2xl" />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div
        className="grid w-full grid-cols-3 gap-1 rounded-xl bg-muted p-1"
        role="tablist"
        aria-label={t("store.admin.sales.tabs.label")}
      >
        {TABS.map((item) => (
          <button
            key={item}
            type="button"
            role="tab"
            aria-selected={tab === item}
            onClick={() => setTab(item)}
            className={
              tab === item
                ? "flex min-h-11 items-center justify-center gap-2 rounded-lg bg-background px-4 text-sm font-medium shadow-sm"
                : "flex min-h-11 items-center justify-center gap-2 rounded-lg px-4 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground"
            }
          >
            {t(`store.admin.sales.tabs.${item}`)}
            {item === "withdrawals" && pendingCount > 0 && (
              <span className="rounded-md bg-warning-soft px-1.5 py-0.5 text-xs text-warning-foreground">
                {pendingCount}
              </span>
            )}
          </button>
        ))}
      </div>

      {tab === "agents" && (
        <SalesAdminAgents
          agents={agents.data ?? []}
          commissionRateBp={settings.data?.commission_rate_bp ?? null}
          onAgentsChanged={() => agents.mutate()}
          onRateChanged={() => settings.mutate()}
        />
      )}

      {tab === "withdrawals" &&
        (withdrawals.isLoading && !withdrawals.data ? (
          <Skeleton className="h-64 rounded-2xl" />
        ) : (
          <SalesAdminWithdrawals
            withdrawals={withdrawals.data ?? []}
            onDecided={() => Promise.all([withdrawals.mutate(), agents.mutate()])}
          />
        ))}

      {tab === "commissions" &&
        (entries.isLoading && !entries.data ? (
          <Skeleton className="h-64 rounded-2xl" />
        ) : (
          <SalesAdminCommissions
            agents={agents.data ?? []}
            entries={entries.data ?? []}
            onClaimed={() => Promise.all([entries.mutate(), agents.mutate()])}
          />
        ))}
    </div>
  );
}
