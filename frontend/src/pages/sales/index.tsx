import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { AlertCircle, LogOut, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ThemeToggle } from "@/components/theme-toggle";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { useAuth } from "@/hooks/use-auth";
import { salesApi } from "@/lib/sales-api";
import { SalesDiscountCard, SalesPasswordCard } from "./sales-account";
import { SalesBalanceCard, SalesCodeCard, SalesWindowCard } from "./sales-cards";
import {
  SalesClaimPanel,
  SalesEntryList,
  SalesWithdrawalLog,
  SalesWithdrawalPanel,
} from "./sales-panels";

type SalesTab = "overview" | "records" | "withdrawals" | "account";

const OVERVIEW_KEY = "sales:overview";
const ENTRIES_KEY = "sales:entries";
const WITHDRAWALS_KEY = "sales:withdrawals";

/**
 * The Sales surface (SC-UI-1).
 *
 * A dedicated page outside the dashboard shell: an agent exists only to sell, so the API-key,
 * usage, and Store surfaces are deliberately absent. Routing keeps an agent here and keeps a
 * non-agent out.
 */
export function SalesPage() {
  const { t } = useTranslation();
  const { logout } = useAuth();
  // Two sub-pages rather than one long column: at 100% zoom the single page ran past the
  // viewport, and an agent checking today's numbers should not have to scroll to see them.
  const [tab, setTab] = useState<SalesTab>("overview");
  const overview = useSWR(OVERVIEW_KEY, () => salesApi.getOverview());
  const entries = useSWR(ENTRIES_KEY, () => salesApi.listEntries());
  const withdrawals = useSWR(WITHDRAWALS_KEY, () => salesApi.listWithdrawals());

  const refreshAll = async () => {
    await Promise.all([overview.mutate(), entries.mutate(), withdrawals.mutate()]);
  };

  if (overview.isLoading && !overview.data) {
    return <SalesSkeleton />;
  }

  if (overview.error || !overview.data) {
    return (
      <PageWrapper className="mx-auto flex w-full max-w-5xl flex-col gap-6 p-4 sm:p-6">
        <Card className="rounded-2xl border-destructive/40">
          <CardContent className="flex flex-col items-start gap-4 p-6 sm:flex-row sm:items-center sm:justify-between">
            <p className="flex items-center gap-2 text-sm text-destructive">
              <AlertCircle className="size-4" />
              {t("sales.loadFailed")}
            </p>
            <Button
              type="button"
              variant="outline"
              className="h-11 rounded-xl"
              onClick={() => void overview.mutate()}
            >
              <RefreshCw className="size-4" />
              {t("sales.retry")}
            </Button>
          </CardContent>
        </Card>
      </PageWrapper>
    );
  }

  const data = overview.data;

  return (
    <PageWrapper className="mx-auto flex w-full max-w-5xl flex-col gap-6 p-4 sm:p-6">
      <motion.header
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
        className="flex flex-wrap items-center justify-between gap-4"
      >
        <div className="min-w-0">
          <h1 className="truncate font-display text-2xl font-semibold">{t("sales.title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("sales.subtitle", { username: data.agent.username })}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {/* SC-6.8: an agent never reaches the dashboard, where the theme control lives. */}
          <ThemeToggle label="" />
          <Button
            type="button"
            variant="outline"
            className="h-11 rounded-xl"
            onClick={() => void logout()}
          >
            <LogOut className="size-4" />
            {t("sales.signOut")}
          </Button>
        </div>
      </motion.header>

      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.06, ...transitions.normal }}
        className="flex flex-col gap-6"
      >
        <div
          className="grid w-full grid-cols-4 gap-1 rounded-xl bg-muted p-1"
          role="tablist"
          aria-label={t("sales.tabs.label")}
        >
          {(["overview", "records", "withdrawals", "account"] as SalesTab[]).map((item) => (
            <button
              key={item}
              type="button"
              role="tab"
              aria-selected={tab === item}
              onClick={() => setTab(item)}
              className={
                tab === item
                  ? "flex min-h-11 items-center justify-center rounded-lg bg-background px-4 text-sm font-medium shadow-sm"
                  : "flex min-h-11 items-center justify-center rounded-lg px-4 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground"
              }
            >
              {t(`sales.tabs.${item}`)}
            </button>
          ))}
        </div>

        {tab === "overview" ? (
          <>
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)]">
              <SalesCodeCard agent={data.agent} />
              <SalesBalanceCard balanceMinor={data.agent.commission_balance_minor} />
            </div>
            <section aria-labelledby="sales-windows" className="flex flex-col gap-3">
              <h2 id="sales-windows" className="text-sm font-semibold">
                {t("sales.window.title")}
              </h2>
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                <SalesWindowCard title={t("sales.window.today")} window={data.today} />
                <SalesWindowCard title={t("sales.window.last7d")} window={data.last_7d} />
                <SalesWindowCard title={t("sales.window.last30d")} window={data.last_30d} />
              </div>
            </section>
          </>
        ) : tab === "withdrawals" ? (
          <>
            <SalesWithdrawalPanel
              balanceMinor={data.agent.commission_balance_minor}
              pending={data.pending_withdrawal}
              withdrawals={withdrawals.data ?? []}
              onRequested={refreshAll}
            />
            {withdrawals.isLoading && !withdrawals.data ? (
              <Skeleton className="h-48 rounded-2xl" />
            ) : (
              <SalesWithdrawalLog withdrawals={withdrawals.data ?? []} />
            )}
          </>
        ) : tab === "account" ? (
          <div className="grid gap-4 lg:grid-cols-2">
            <SalesDiscountCard
              agent={data.agent}
              maxDiscountBp={data.commission_rate_bp}
              onUpdated={refreshAll}
            />
            <SalesPasswordCard />
          </div>
        ) : (
          <>
            <SalesClaimPanel onClaimed={refreshAll} />
            <section aria-labelledby="sales-entries" className="flex flex-col gap-3">
              <h2 id="sales-entries" className="text-sm font-semibold">
                {t("sales.entries.title")}
              </h2>
              {entries.isLoading && !entries.data ? (
                <Skeleton className="h-48 rounded-2xl" />
              ) : (
                <SalesEntryList entries={entries.data ?? []} />
              )}
            </section>
          </>
        )}
      </motion.div>
    </PageWrapper>
  );
}

function SalesSkeleton() {
  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 p-4 sm:p-6" aria-hidden="true">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex flex-col gap-2">
          <Skeleton className="h-8 w-40" />
          <Skeleton className="h-4 w-56" />
        </div>
        <Skeleton className="h-11 w-28 rounded-xl" />
      </div>
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)]">
        <Skeleton className="h-32 rounded-2xl" />
        <Skeleton className="h-32 rounded-2xl" />
      </div>
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {[0, 1, 2].map((card) => (
          <Skeleton key={card} className="h-40 rounded-2xl" />
        ))}
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        <Skeleton className="h-64 rounded-2xl" />
        <Skeleton className="h-64 rounded-2xl" />
      </div>
      <Skeleton className="h-48 rounded-2xl" />
    </div>
  );
}
