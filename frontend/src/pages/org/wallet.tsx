import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Coins } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type OrgDetail, type OrgLedgerEntry } from "@/lib/api";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";

const KIND_LABELS: Record<string, string> = {
  org_deposit_receive: "org.kindDeposit",
  org_grant: "org.kindGrant",
  org_deposit: "org.kindOut",
  store_recharge: "org.kindRecharge",
};

export function OrgWallet() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  const money = (nano: string) =>
    currency === "CNY" && rate?.cny_per_usd
      ? formatCoinFromNanoUsdForCurrency(nano, "CNY", rate.cny_per_usd)
      : formatCoinFromNanoUsdForCurrency(nano, "USD", "1");
  const detail = useSWR<OrgDetail>(orgId ? `/api/dashboard/orgs/${orgId}` : null, () =>
    api.getOrgDetail(orgId!),
  );
  const ledger = useSWR<OrgLedgerEntry[]>(orgId ? `/api/dashboard/orgs/${orgId}/ledger` : null, () =>
    api.getOrgLedger(orgId!),
  );

  if (!orgId) return null;

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <h1 className="text-xl font-semibold">{t("org.navWallet")}</h1>

      <Card className="rounded-2xl">
        <CardContent className="flex items-center gap-3 p-5">
          <Coins className="size-8 text-muted-foreground" />
          <div>
            <p className="text-sm text-muted-foreground">{t("org.walletBalance")}</p>
            <p className="text-2xl font-semibold tabular-nums">
              {detail.isLoading || !detail.data ? "…" : money(detail.data.balance_nano_usd)}
            </p>
          </div>
        </CardContent>
      </Card>

      <Card className="rounded-2xl">
        <CardContent className="p-0">
          {ledger.isLoading ? (
            <div className="space-y-2 p-5">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </div>
          ) : !ledger.data || ledger.data.length === 0 ? (
            <p className="p-8 text-center text-sm text-muted-foreground">{t("org.noLedger")}</p>
          ) : (
            <table className="w-full text-sm">
              <thead className="border-b text-left text-xs text-muted-foreground">
                <tr>
                  <th className="px-5 py-3 font-medium">{t("org.ledgerKind")}</th>
                  <th className="px-5 py-3 font-medium">{t("org.ledgerDelta")}</th>
                  <th className="px-5 py-3 font-medium">{t("org.ledgerBalance")}</th>
                  <th className="px-5 py-3 font-medium">{t("org.ledgerTime")}</th>
                </tr>
              </thead>
              <tbody>
                {ledger.data.map((entry) => (
                  <tr key={entry.id} className="border-b last:border-b-0">
                    <td className="px-5 py-2.5">
                      {t(KIND_LABELS[entry.kind] ?? "org.kindOther")}
                    </td>
                    <td
                      className={`px-5 py-2.5 tabular-nums ${
                        entry.delta_nano_usd.startsWith("-") ? "text-red-500" : "text-emerald-600"
                      }`}
                    >
                      {money(entry.delta_nano_usd)}
                    </td>
                    <td className="px-5 py-2.5 tabular-nums">
                      {money(entry.balance_after_nano_usd)}
                    </td>
                    <td className="px-5 py-2.5 text-muted-foreground">
                      {new Date(entry.created_at).toLocaleString()}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
