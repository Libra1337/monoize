import { useState } from "react";
import { SiAlipay, SiWechat } from "@icons-pack/react-simple-icons";
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
import { Switch } from "@/components/ui/switch";
import {
  storeApi,
  type EpayMethodConfig,
  type EpayMethodKind,
  type StorePaymentChannel,
} from "@/lib/store-api";

interface EpayMethodsDialogProps {
  channel: StorePaymentChannel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => Promise<unknown>;
}

function MethodMark({ method }: { method: EpayMethodKind }) {
  return method === "alipay"
    ? <SiAlipay className="size-5 text-[#1677ff]" />
    : <SiWechat className="size-5 text-[#07c160]" />;
}

function MethodRow({
  method,
  saving,
  onSave,
}: {
  method: EpayMethodConfig;
  saving: boolean;
  onSave: (method: EpayMethodKind, label: string, sortOrder: number, enabled: boolean) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [label, setLabel] = useState(method.label);
  const [sortOrder, setSortOrder] = useState(String(method.sort_order));
  const [enabled, setEnabled] = useState(method.enabled);
  const parsedSortOrder = Number.parseInt(sortOrder, 10);
  const invalid = label.trim() === "" || !Number.isInteger(parsedSortOrder);

  return (
    <section
      className="grid gap-3 rounded-xl border p-3"
      aria-labelledby={`epay-method-${method.method}`}
    >
      <div className="flex items-center gap-2">
        <MethodMark method={method.method} />
        <h3 id={`epay-method-${method.method}`} className="text-sm font-semibold">
          {t(`store.admin.epayMethods.methods.${method.method}`)}
        </h3>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="grid gap-2">
          <Label htmlFor={`epay-method-label-${method.method}`}>
            {t("store.admin.epayMethods.label")}
          </Label>
          <Input
            id={`epay-method-label-${method.method}`}
            className="min-h-11 rounded-xl"
            value={label}
            onChange={(event) => setLabel(event.target.value)}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor={`epay-method-sort-${method.method}`}>
            {t("store.admin.epayMethods.sortOrder")}
          </Label>
          <Input
            id={`epay-method-sort-${method.method}`}
            className="min-h-11 rounded-xl"
            inputMode="numeric"
            value={sortOrder}
            onChange={(event) => setSortOrder(event.target.value)}
          />
        </div>
      </div>
      <label className="flex min-h-11 cursor-pointer items-center justify-between gap-3 rounded-xl border px-3">
        <span className="text-sm font-medium">{t("store.admin.epayMethods.enabled")}</span>
        <Switch checked={enabled} onCheckedChange={setEnabled} />
      </label>
      <Button
        type="button"
        variant="secondary"
        className="min-h-11 w-fit rounded-xl"
        disabled={saving || invalid}
        onClick={() => void onSave(method.method, label.trim(), parsedSortOrder, enabled)}
      >
        {saving ? t("common.loading") : t("common.save")}
      </Button>
    </section>
  );
}

export function EpayMethodsDialog({ channel, open, onOpenChange, onSaved }: EpayMethodsDialogProps) {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);

  const save = async (
    method: EpayMethodKind,
    label: string,
    sortOrder: number,
    enabled: boolean,
  ) => {
    if (!channel) return;
    const current = channel.epay_methods.find((option) => option.method === method);
    if (!current) return;
    setSaving(true);
    try {
      await storeApi.admin.putEpayMethod(channel.id, method, {
        label,
        icon_kind: current.icon_kind,
        icon_value: current.icon_value,
        sort_order: sortOrder,
        enabled,
      });
      await onSaved();
      toast.success(t("store.admin.epayMethods.saved"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("store.admin.epayMethods.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto rounded-2xl sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("store.admin.epayMethods.title")}</DialogTitle>
          <DialogDescription>{t("store.admin.epayMethods.description")}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          {(channel?.epay_methods ?? []).map((method) => (
            <MethodRow
              key={method.method}
              method={method}
              saving={saving}
              onSave={save}
            />
          ))}
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            className="rounded-xl"
            onClick={() => onOpenChange(false)}
          >
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
