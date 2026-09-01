import { useMemo, useState } from "react";
import { Infinity as InfinityIcon, WalletCards } from "lucide-react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Card, CardContent } from "@/components/ui/card";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { useAuth } from "@/hooks/use-auth";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { api } from "@/lib/api";
import { storeApi } from "@/lib/store-api";
import { formatCoinFromNanoUsd } from "@/lib/store-money";
import { RedemptionPanel } from "./store/redemption-panel";
import { OrdersPage } from "./orders";

const ENTITLEMENT_KEY = "/api/dashboard/store/entitlement";

export function WalletPage() {
  const { t } = useTranslation();
  const { user, refreshUser } = useAuth();
  const [redeeming, setRedeeming] = useState(false);
  const exchangeRate = useStoreExchangeRate(true);
  const entitlement = useSWR(ENTITLEMENT_KEY, storeApi.getEntitlement);
  const monthStart = useMemo(() => {
    const now = new Date();
    return new Date(now.getFullYear(), now.getMonth(), 1).toISOString();
  }, []);
  const monthlyUsage = useSWR(["wallet-monthly-usage", monthStart], () => api.listRequestLogs(1, 0, { time_from: monthStart }));
  const rate = exchangeRate.data?.cny_per_usd;
  const formatCoin = (value: string | null | undefined) => rate ? formatCoinFromNanoUsd(value ?? "0", rate) : "--";

  const handleRedeem = async (code: string) => {
    setRedeeming(true);
    try {
      await storeApi.redeem(code);
      await Promise.all([refreshUser(), entitlement.mutate()]);
    } finally {
      setRedeeming(false);
    }
  };

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6 pb-6">
      <PageHeader title={t("wallet.title")} description={t("wallet.description")} />
      <section className="grid gap-3 sm:grid-cols-3" aria-label={t("wallet.summaryLabel")}>
        <Card className="rounded-2xl"><CardContent className="p-5"><p className="text-sm text-muted-foreground">{t("wallet.balance")}</p><p className="mt-2 text-xl font-semibold">{user?.balance_unlimited ? <span className="flex items-center gap-2"><InfinityIcon className="size-5" />{t("store.ui.accountUnlimited")}</span> : formatCoin(user?.balance_nano_usd)}</p></CardContent></Card>
        <Card className="rounded-2xl"><CardContent className="p-5"><p className="text-sm text-muted-foreground">{t("wallet.monthlyUsage")}</p><p className="mt-2 text-xl font-semibold">{formatCoin(monthlyUsage.data?.total_charge_nano_usd)}</p></CardContent></Card>
        <Card className="rounded-2xl"><CardContent className="p-5"><p className="text-sm text-muted-foreground">{t("wallet.currentPlan")}</p><p className="mt-2 font-semibold">{entitlement.data?.product_name ?? t("store.account.noPlan")}</p>{entitlement.data && <p className="mt-1 text-xs text-muted-foreground">{t("store.ui.accountEnds", { date: new Date(entitlement.data.ends_at).toLocaleDateString() })}</p>}</CardContent></Card>
      </section>
      <Card className="rounded-2xl"><CardContent className="p-5"><div className="mb-4 flex items-center gap-2"><WalletCards className="size-5 text-primary" /><h2 className="font-display text-base font-semibold">{t("wallet.redemptionTitle")}</h2></div><RedemptionPanel onRedeem={handleRedeem} redeeming={redeeming} /></CardContent></Card>
      <section aria-labelledby="wallet-orders-title"><h2 id="wallet-orders-title" className="mb-4 font-display text-lg font-semibold">{t("wallet.orders")}</h2><OrdersPage embedded /></section>
    </PageWrapper>
  );
}
