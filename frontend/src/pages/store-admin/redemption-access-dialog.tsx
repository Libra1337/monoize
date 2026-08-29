import { useEffect, useState } from "react";
import { Check, Copy, Eye } from "lucide-react";
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
import type { RedemptionCodeRecord } from "@/lib/store-api";

interface RedemptionAccessDialogProps {
  open: boolean;
  record: RedemptionCodeRecord | null;
  action: "reveal" | "copy";
  onOpenChange: (open: boolean) => void;
  onSubmit: (record: RedemptionCodeRecord, action: "reveal" | "copy", password: string) => Promise<string>;
}

export function RedemptionAccessDialog({ open, record, action, onOpenChange, onSubmit }: RedemptionAccessDialogProps) {
  const { t } = useTranslation();
  const [currentPassword, setCurrentPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const clearSensitiveState = (clearError = true) => {
    setCurrentPassword("");
    setCode("");
    if (clearError) setError(null);
  };

  useEffect(() => {
    if (!open) clearSensitiveState();
  }, [open]);

  const submit = async () => {
    if (!record || !currentPassword) {
      setError(t("store.admin.redemptions.passwordRequired"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const revealed = await onSubmit(record, action, currentPassword);
      setCode(revealed);
      setCurrentPassword("");
      if (action === "copy") {
        await navigator.clipboard.writeText(revealed);
        toast.success(t("common.copied"));
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("common.error"));
      clearSensitiveState(false);
    } finally {
      setBusy(false);
    }
  };

  const close = (nextOpen: boolean) => {
    if (!nextOpen) clearSensitiveState();
    onOpenChange(nextOpen);
  };

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent className="rounded-2xl sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">{action === "copy" ? <Copy className="size-4 text-primary" /> : <Eye className="size-4 text-primary" />}{t(action === "copy" ? "store.admin.redemptions.copyCode" : "store.admin.redemptions.revealCode")}</DialogTitle>
          <DialogDescription>{t("store.admin.redemptions.reauthDescription")}</DialogDescription>
        </DialogHeader>
        {code ? (
          <div className="grid gap-3 rounded-xl border bg-muted/30 p-4">
            <Label htmlFor="revealed-redemption-code">{t("store.admin.redemptions.completeCode")}</Label>
            <code id="revealed-redemption-code" className="break-all rounded-lg bg-background p-3 text-base font-semibold tracking-wide">{code}</code>
            {action === "copy" && <p className="flex items-center gap-2 text-sm text-muted-foreground"><Check className="size-4 text-primary" />{t("store.admin.redemptions.copiedFromAccess")}</p>}
          </div>
        ) : (
          <div className="grid gap-2">
            <Label htmlFor="redemption-access-password">{t("store.admin.orders.currentPassword")}</Label>
            <Input id="redemption-access-password" className="min-h-11 rounded-xl" type="password" autoComplete="current-password" value={currentPassword} onChange={(event) => setCurrentPassword(event.target.value)} />
            {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          </div>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" className="min-h-11 rounded-xl" onClick={() => close(false)}>{t("store.ui.close")}</Button>
          {!code && <Button type="button" className="min-h-11 rounded-xl" disabled={busy} onClick={() => void submit()}>{busy ? t("common.loading") : t("store.admin.redemptions.continue")}</Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
