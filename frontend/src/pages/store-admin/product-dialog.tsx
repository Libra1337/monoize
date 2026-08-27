import { useEffect, useMemo, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { Group } from "@/lib/api";
import {
  type PlanQuotaInput,
  type PlanWindowKind,
  type StoreProduct,
  type StoreProductInput,
} from "@/lib/store-api";
import { addMinor, decimalToMinor, formatMinor, minorToDecimal } from "@/lib/store-money";

interface ProductDialogProps {
  open: boolean;
  product: StoreProduct | null;
  groups: Group[];
  saving: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (input: StoreProductInput) => Promise<void>;
}

interface QuotaDraft {
  window_kind: PlanWindowKind | "custom_hours";
  window_hours: string;
  quota_minor_cny: string;
}

const fixedWindowHours: Record<Exclude<PlanWindowKind, "custom">, number> = {
  "5h": 5,
  "12h": 12,
  day: 24,
  week: 168,
  month: 720,
};

const emptyQuota = (): QuotaDraft => ({
  window_kind: "day",
  window_hours: "24",
  quota_minor_cny: "",
});

function quotaFromProduct(quota: StoreProduct["quotas"][number]): QuotaDraft {
  return {
    window_kind: quota.window_kind === "custom" ? "custom_hours" : quota.window_kind,
    window_hours: String(quota.window_seconds / 3600),
    quota_minor_cny: minorToDecimal(quota.quota_fen_cny),
  };
}

export function ProductDialog({
  open,
  product,
  groups,
  saving,
  onOpenChange,
  onSave,
}: ProductDialogProps) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<"balance" | "plan">("balance");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [currency, setCurrency] = useState<"CNY" | "USD">("CNY");
  const [recharge, setRecharge] = useState("");
  const [bonus, setBonus] = useState("0");
  const [price, setPrice] = useState("");
  const [durationDays, setDurationDays] = useState("30");
  const [groupIds, setGroupIds] = useState<string[]>([]);
  const [quotas, setQuotas] = useState<QuotaDraft[]>([emptyQuota()]);
  const [enabled, setEnabled] = useState(true);
  const [sortOrder, setSortOrder] = useState("0");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setKind(product?.kind ?? "balance");
    setName(product?.name ?? "");
    setDescription(product?.description ?? "");
    setCurrency(product?.price_currency ?? "CNY");
    setRecharge(product?.balance ? minorToDecimal(product.balance.recharge_minor) : "");
    setBonus(product?.balance ? minorToDecimal(product.balance.bonus_minor) : "0");
    setPrice(product && product.kind === "plan" ? minorToDecimal(product.price_minor) : "");
    setDurationDays(product?.duration_seconds ? String(product.duration_seconds / 86400) : "30");
    setGroupIds(product?.group_ids ?? []);
    setQuotas(product?.quotas.length ? product.quotas.map(quotaFromProduct) : [emptyQuota()]);
    setEnabled(product?.enabled ?? true);
    setSortOrder(String(product?.sort_order ?? 0));
    setError(null);
  }, [open, product]);

  const actualReceived = useMemo(() => {
    const recharge_minor = decimalToMinor(recharge);
    const bonus_minor = decimalToMinor(bonus);
    if (recharge_minor === null || bonus_minor === null) return null;
    return formatMinor(addMinor(recharge_minor, bonus_minor), currency);
  }, [bonus, currency, recharge]);

  const updateQuota = (index: number, patch: Partial<QuotaDraft>) => {
    setQuotas((current) => current.map((quota, itemIndex) => {
      if (itemIndex !== index) return quota;
      const next = { ...quota, ...patch };
      if (patch.window_kind && patch.window_kind !== "custom_hours") {
        next.window_hours = String(fixedWindowHours[patch.window_kind]);
      }
      return next;
    }));
  };

  const submit = async () => {
    setError(null);
    const price_minor = decimalToMinor(kind === "balance" ? recharge : price);
    const parsedSortOrder = Number(sortOrder);
    if (!name.trim() || !price_minor || BigInt(price_minor) === 0n || !Number.isInteger(parsedSortOrder)) {
      setError(t("store.admin.products.invalidProduct"));
      return;
    }

    let balance: StoreProductInput["balance"] = null;
    let quotaInputs: PlanQuotaInput[] = [];
    let duration_seconds: number | null = null;
    if (kind === "balance") {
      const recharge_minor = decimalToMinor(recharge);
      const bonus_minor = decimalToMinor(bonus);
      if (!recharge_minor || bonus_minor === null) {
        setError(t("store.admin.products.invalidAmounts"));
        return;
      }
      balance = { recharge_minor, bonus_minor };
    } else {
      const days = Number(durationDays);
      if (!Number.isInteger(days) || days < 1 || days > 365 || quotas.length === 0) {
        setError(t("store.admin.products.invalidPlan"));
        return;
      }
      duration_seconds = days * 86400;
      const parsedQuotas = quotas.map((quota, index): PlanQuotaInput | null => {
        const hours = Number(quota.window_hours);
        const quota_fen_cny = decimalToMinor(quota.quota_minor_cny);
        if (!Number.isInteger(hours) || hours < 1 || hours > 8760 || !quota_fen_cny || BigInt(quota_fen_cny) === 0n) {
          return null;
        }
        return {
          window_kind: quota.window_kind === "custom_hours" ? "custom" : quota.window_kind,
          window_seconds: hours * 3600,
          quota_fen_cny,
          sort_order: index,
        };
      });
      if (parsedQuotas.some((quota) => quota === null)) {
        setError(t("store.admin.products.invalidQuota"));
        return;
      }
      quotaInputs = parsedQuotas as PlanQuotaInput[];
      if (new Set(quotaInputs.map((quota) => quota.window_seconds)).size !== quotaInputs.length) {
        setError(t("store.admin.products.duplicateQuota"));
        return;
      }
    }

    await onSave({
      kind,
      name: name.trim(),
      description: description.trim(),
      price_currency: currency,
      price_minor,
      duration_seconds,
      group_ids: kind === "plan" ? groupIds : [],
      sort_order: parsedSortOrder,
      enabled,
      balance,
      quotas: quotaInputs,
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto rounded-2xl sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t(product ? "store.admin.products.edit" : "store.admin.products.create")}</DialogTitle>
          <DialogDescription>{t("store.admin.products.dialogDescription")}</DialogDescription>
        </DialogHeader>

        <div className="grid gap-5 py-2">
          <div className="grid grid-cols-2 gap-2 rounded-xl bg-muted p-1">
            {(["balance", "plan"] as const).map((value) => (
              <Button key={value} type="button" variant={kind === value ? "secondary" : "ghost"} className="min-h-11 rounded-lg" disabled={Boolean(product)} onClick={() => setKind(value)}>
                {t(`store.admin.products.kinds.${value}`)}
              </Button>
            ))}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2">
              <Label htmlFor="store-product-name">{t("store.admin.products.name")}</Label>
              <Input id="store-product-name" className="min-h-11 rounded-xl" value={name} maxLength={100} onChange={(event) => setName(event.target.value)} />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="store-product-currency">{t("store.admin.products.baseCurrency")}</Label>
              <Select value={currency} onValueChange={(value) => setCurrency(value as "CNY" | "USD")}>
                <SelectTrigger id="store-product-currency" className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger>
                <SelectContent><SelectItem value="CNY">CNY</SelectItem><SelectItem value="USD">USD</SelectItem></SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid gap-2">
            <Label htmlFor="store-product-description">{t("store.admin.products.description")}</Label>
            <Textarea id="store-product-description" className="rounded-xl" rows={3} maxLength={500} value={description} onChange={(event) => setDescription(event.target.value)} />
          </div>

          {kind === "balance" ? (
            <div className="grid gap-4 sm:grid-cols-3">
              <div className="grid gap-2"><Label htmlFor="store-recharge">{t("store.admin.products.recharge")}</Label><Input id="store-recharge" className="min-h-11 rounded-xl" inputMode="decimal" value={recharge} onChange={(event) => setRecharge(event.target.value)} /></div>
              <div className="grid gap-2"><Label htmlFor="store-bonus">{t("store.admin.products.bonus")}</Label><Input id="store-bonus" className="min-h-11 rounded-xl" inputMode="decimal" value={bonus} onChange={(event) => setBonus(event.target.value)} /></div>
              <div className="grid gap-2"><Label>{t("store.admin.products.actualReceived")}</Label><div className="flex min-h-11 items-center rounded-xl border bg-muted/40 px-3 font-medium tabular-nums">{actualReceived ?? "--"}</div></div>
            </div>
          ) : (
            <div className="grid gap-5">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="grid gap-2"><Label htmlFor="store-plan-price">{t("store.admin.products.price")}</Label><Input id="store-plan-price" className="min-h-11 rounded-xl" inputMode="decimal" value={price} onChange={(event) => setPrice(event.target.value)} /></div>
                <div className="grid gap-2"><Label htmlFor="store-plan-duration">{t("store.admin.products.durationDays")}</Label><Input id="store-plan-duration" className="min-h-11 rounded-xl" type="number" min={1} max={365} value={durationDays} onChange={(event) => setDurationDays(event.target.value)} /></div>
              </div>

              <fieldset className="grid gap-3">
                <legend className="text-sm font-medium">{t("store.admin.products.groups")}</legend>
                <div className="grid gap-2 sm:grid-cols-2">
                  {groups.map((group) => (
                    <label key={group.id} className="flex min-h-11 cursor-pointer items-center gap-3 rounded-xl border px-3 text-sm">
                      <Checkbox checked={groupIds.includes(group.id)} onCheckedChange={(checked) => setGroupIds((current) => checked ? [...current, group.id] : current.filter((id) => id !== group.id))} />
                      {group.name}
                    </label>
                  ))}
                </div>
              </fieldset>

              <fieldset className="grid gap-3">
                <div className="flex items-center justify-between gap-3">
                  <legend className="text-sm font-medium">{t("store.admin.products.quotas")}</legend>
                  <Button type="button" variant="outline" size="sm" className="rounded-xl" onClick={() => setQuotas((current) => [...current, emptyQuota()])}><Plus className="h-4 w-4" />{t("store.admin.products.addQuota")}</Button>
                </div>
                {quotas.map((quota, index) => (
                  <div key={index} className="grid gap-3 rounded-xl border p-3 sm:grid-cols-[1fr_1fr_auto]">
                    <div className="grid gap-2"><Label>{t("store.admin.products.window")}</Label><Select value={quota.window_kind} onValueChange={(value) => updateQuota(index, { window_kind: value as QuotaDraft["window_kind"] })}><SelectTrigger className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger><SelectContent>{["5h", "12h", "day", "week", "month", "custom_hours"].map((value) => <SelectItem key={value} value={value}>{t(`store.admin.products.windows.${value}`)}</SelectItem>)}</SelectContent></Select></div>
                    {quota.window_kind === "custom_hours" ? <div className="grid gap-2"><Label>{t("store.admin.products.customHours")}</Label><Input className="min-h-11 rounded-xl" type="number" min={1} max={8760} value={quota.window_hours} onChange={(event) => updateQuota(index, { window_hours: event.target.value })} /></div> : <div className="grid gap-2"><Label>{t("store.admin.products.quotaCny")}</Label><Input className="min-h-11 rounded-xl" inputMode="decimal" value={quota.quota_minor_cny} onChange={(event) => updateQuota(index, { quota_minor_cny: event.target.value })} /></div>}
                    {quota.window_kind === "custom_hours" && <div className="grid gap-2 sm:col-start-2"><Label>{t("store.admin.products.quotaCny")}</Label><Input className="min-h-11 rounded-xl" inputMode="decimal" value={quota.quota_minor_cny} onChange={(event) => updateQuota(index, { quota_minor_cny: event.target.value })} /></div>}
                    <Button type="button" variant="ghost" size="icon" className="min-h-11 min-w-11 self-end rounded-xl" aria-label={t("store.admin.products.removeQuota")} disabled={quotas.length === 1} onClick={() => setQuotas((current) => current.filter((_, itemIndex) => itemIndex !== index))}><Trash2 className="h-4 w-4 text-destructive" /></Button>
                  </div>
                ))}
              </fieldset>
            </div>
          )}

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2"><Label htmlFor="store-sort-order">{t("store.admin.products.sortOrder")}</Label><Input id="store-sort-order" className="min-h-11 rounded-xl" type="number" value={sortOrder} onChange={(event) => setSortOrder(event.target.value)} /></div>
            <label className="flex min-h-11 cursor-pointer items-center justify-between gap-3 self-end rounded-xl border px-3"><span className="text-sm font-medium">{t("store.admin.enabled")}</span><Switch checked={enabled} onCheckedChange={setEnabled} /></label>
          </div>
          {error && <p className="text-sm text-destructive" role="alert">{error}</p>}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" className="rounded-xl" onClick={() => onOpenChange(false)}>{t("common.cancel")}</Button>
          <Button type="button" className="rounded-xl" disabled={saving} onClick={() => void submit()}>{saving ? t("common.loading") : t("common.save")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
