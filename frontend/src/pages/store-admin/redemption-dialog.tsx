import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
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
import type {
  GeneratedRedemptionCode,
  GenerateRedemptionCodesInput,
  StoreProduct,
} from "@/lib/store-api";
import { decimalToMinor } from "@/lib/store-money";

interface RedemptionDialogProps {
  open: boolean;
  plans: StoreProduct[];
  generating: boolean;
  onOpenChange: (open: boolean) => void;
  onGenerate: (input: GenerateRedemptionCodesInput) => Promise<GeneratedRedemptionCode[]>;
}

export function RedemptionDialog({ open, plans, generating, onOpenChange, onGenerate }: RedemptionDialogProps) {
  const { t } = useTranslation();
  const [rewardKind, setRewardKind] = useState<"balance" | "plan">("balance");
  const [currency, setCurrency] = useState<"CNY" | "USD">("CNY");
  const [amount, setAmount] = useState("");
  const [planId, setPlanId] = useState("");
  const [count, setCount] = useState("1");
  const [validityDays, setValidityDays] = useState("30");
  const [generatedCodes, setGeneratedCodes] = useState<GeneratedRedemptionCode[]>([]);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setRewardKind("balance");
    setCurrency("CNY");
    setAmount("");
    setPlanId(plans[0]?.id ?? "");
    setCount("1");
    setValidityDays("30");
    setGeneratedCodes([]);
    setCopied(false);
    setError(null);
  }, [open, plans]);

  const generate = async () => {
    const parsedCount = Number(count);
    const parsedValidity = Number(validityDays);
    if (!Number.isInteger(parsedCount) || parsedCount < 1 || parsedCount > 20 || !Number.isInteger(parsedValidity) || parsedValidity < 1 || parsedValidity > 365) {
      setError(t("store.admin.redemptions.invalidBounds"));
      return;
    }
    let reward: GenerateRedemptionCodesInput["reward"];
    if (rewardKind === "balance") {
      const amount_minor = decimalToMinor(amount);
      if (!amount_minor || BigInt(amount_minor) === 0n) {
        setError(t("store.admin.redemptions.invalidAmount"));
        return;
      }
      reward = { kind: "balance", currency, amount_minor };
    } else {
      if (!plans.some((plan) => plan.id === planId)) {
        setError(t("store.admin.redemptions.selectPlan"));
        return;
      }
      reward = { kind: "plan", product_id: planId };
    }
    setError(null);
    const result = await onGenerate({ reward, count: parsedCount, validity_days: parsedValidity });
    setGeneratedCodes(result);
  };

  const copyAll = async () => {
    await navigator.clipboard.writeText(generatedCodes.map((item) => item.code).join("\n"));
    setCopied(true);
    toast.success(t("common.copied"));
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto rounded-2xl sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("store.admin.redemptions.generate")}</DialogTitle>
          <DialogDescription>{generatedCodes.length ? t("store.admin.redemptions.onceOnly") : t("store.admin.redemptions.dialogDescription")}</DialogDescription>
        </DialogHeader>

        {generatedCodes.length ? (
          <div className="grid gap-4">
            <div className="max-h-72 overflow-y-auto rounded-xl border bg-muted/30 p-3 font-mono text-sm">
              {generatedCodes.map((item) => <div key={item.record.id} className="border-b py-2 last:border-0">{item.code}</div>)}
            </div>
            <Button type="button" variant="outline" className="min-h-11 rounded-xl" onClick={() => void copyAll()}>{copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}{t("store.admin.redemptions.copyAll")}</Button>
          </div>
        ) : (
          <div className="grid gap-5 py-2">
            <div className="grid grid-cols-2 gap-2 rounded-xl bg-muted p-1">
              {(["balance", "plan"] as const).map((value) => <Button key={value} type="button" variant={rewardKind === value ? "secondary" : "ghost"} className="min-h-11 rounded-lg" onClick={() => setRewardKind(value)}>{t(`store.admin.redemptions.rewardKinds.${value}`)}</Button>)}
            </div>
            {rewardKind === "balance" ? (
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="grid gap-2"><Label>{t("store.admin.redemptions.currency")}</Label><Select value={currency} onValueChange={(value) => setCurrency(value as "CNY" | "USD")}><SelectTrigger className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="CNY">CNY</SelectItem><SelectItem value="USD">USD</SelectItem></SelectContent></Select></div>
                <div className="grid gap-2"><Label htmlFor="redemption-amount">{t("store.admin.redemptions.amount")}</Label><Input id="redemption-amount" className="min-h-11 rounded-xl" inputMode="decimal" value={amount} onChange={(event) => setAmount(event.target.value)} /></div>
              </div>
            ) : (
              <div className="grid gap-2"><Label>{t("store.admin.redemptions.plan")}</Label><Select value={planId} onValueChange={setPlanId}><SelectTrigger className="min-h-11 rounded-xl"><SelectValue placeholder={t("store.admin.redemptions.selectPlan")} /></SelectTrigger><SelectContent>{plans.map((plan) => <SelectItem key={plan.id} value={plan.id}>{plan.name}</SelectItem>)}</SelectContent></Select></div>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="grid gap-2"><Label htmlFor="redemption-count">{t("store.admin.redemptions.count")}</Label><Input id="redemption-count" className="min-h-11 rounded-xl" type="number" min={1} max={20} value={count} onChange={(event) => setCount(event.target.value)} /></div>
              <div className="grid gap-2"><Label htmlFor="redemption-validity">{t("store.admin.redemptions.validityDays")}</Label><Input id="redemption-validity" className="min-h-11 rounded-xl" type="number" min={1} max={365} value={validityDays} onChange={(event) => setValidityDays(event.target.value)} /></div>
            </div>
            {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          </div>
        )}

        <DialogFooter>
          <Button type="button" variant="outline" className="rounded-xl" onClick={() => onOpenChange(false)}>{t(generatedCodes.length ? "store.ui.close" : "common.cancel")}</Button>
          {!generatedCodes.length && <Button type="button" className="rounded-xl" disabled={generating} onClick={() => void generate()}>{generating ? t("common.loading") : t("store.admin.redemptions.generate")}</Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
