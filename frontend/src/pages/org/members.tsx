import { useState } from "react";
import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { api, type OrgDetail, type OrgMember } from "@/lib/api";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { useMyOrgs } from "./shell";

function amountToNanoUsd(raw: string, currency: "CNY" | "USD", cnyPerUsd?: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || !/^\d+(\.\d{1,2})?$/.test(trimmed)) return null;
  const minor = Math.round(parseFloat(trimmed) * 100);
  if (!Number.isSafeInteger(minor) || minor <= 0) return null;
  if (currency === "USD") return (BigInt(minor) * 10_000_000n / 100n).toString();
  if (!cnyPerUsd) return null;
  const [whole, frac = ""] = cnyPerUsd.split(".");
  const numerator = BigInt(whole + frac.padEnd(6, "0").slice(0, 6));
  if (numerator <= 0n) return null;
  return ((BigInt(minor) * 1_000_000n * 10_000_000n + numerator / 2n) / numerator).toString();
}

export function OrgMembers() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const { mutate: reloadOrgs } = useMyOrgs();
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  const detail = useSWR<OrgDetail>(orgId ? `/api/dashboard/orgs/${orgId}` : null, () =>
    api.getOrgDetail(orgId!),
  );
  const [distributeTarget, setDistributeTarget] = useState<OrgMember | null>(null);
  const [amount, setAmount] = useState("");
  const [busy, setBusy] = useState(false);

  if (!orgId || !detail.data) {
    return <div className="p-6 text-sm text-muted-foreground">…</div>;
  }
  const isOwner = detail.data.my_role === "owner";

  return (
    <div className="mx-auto max-w-4xl space-y-4 p-6">
      <h1 className="text-xl font-semibold">{t("org.navMembers")}</h1>
      <Card className="rounded-2xl">
        <CardContent className="p-0">
          <table className="w-full text-sm">
            <thead className="border-b text-left text-xs text-muted-foreground">
              <tr>
                <th className="px-5 py-3 font-medium">{t("org.memberName")}</th>
                <th className="px-5 py-3 font-medium">{t("org.memberRole")}</th>
                <th className="px-5 py-3 font-medium">{t("org.memberJoined")}</th>
                <th className="px-5 py-3 text-right font-medium">{t("common.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {detail.data.members.map((member) => (
                <tr key={member.user_id} className="border-b last:border-b-0">
                  <td className="px-5 py-3 font-medium">{member.username}</td>
                  <td className="px-5 py-3">
                    <Badge variant={member.role === "owner" ? "default" : "secondary"}>
                      {member.role === "owner" ? t("org.owner") : t("org.member")}
                    </Badge>
                  </td>
                  <td className="px-5 py-3 text-muted-foreground">
                    {new Date(member.joined_at).toLocaleDateString()}
                  </td>
                  <td className="px-5 py-3 text-right">
                    {isOwner && member.role !== "owner" && (
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => {
                            setDistributeTarget(member);
                            setAmount("");
                          }}
                        >
                          {t("org.distribute")}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-destructive"
                          onClick={async () => {
                            try {
                              await api.removeOrgMember(orgId, member.user_id);
                              toast.success(t("org.memberRemoved"));
                              await Promise.all([detail.mutate(), reloadOrgs()]);
                            } catch (error) {
                              toast.error(error instanceof Error ? error.message : t("common.error"));
                            }
                          }}
                        >
                          {t("org.removeMember")}
                        </Button>
                      </div>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </CardContent>
      </Card>

      <Dialog open={!!distributeTarget} onOpenChange={(open) => !open && setDistributeTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("org.distributeTitle", { name: distributeTarget?.username ?? "" })}</DialogTitle>
            <DialogDescription>{t("org.transferDescription", { currency })}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="member-distribute">{t("org.amountLabel", { currency })}</Label>
            <Input
              id="member-distribute"
              inputMode="decimal"
              value={amount}
              onChange={(event) => setAmount(event.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDistributeTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy || !amount.trim()}
              onClick={async () => {
                const nano = amountToNanoUsd(amount, currency, rate?.cny_per_usd);
                if (!nano || !distributeTarget) {
                  toast.error(t("org.transferInvalidAmount"));
                  return;
                }
                setBusy(true);
                try {
                  await api.distributeFromOrg(orgId, distributeTarget.user_id, nano);
                  toast.success(t("org.distributeDone"));
                  setDistributeTarget(null);
                  await Promise.all([detail.mutate(), reloadOrgs()]);
                } catch (error) {
                  toast.error(error instanceof Error ? error.message : t("common.error"));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("org.distribute")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
