import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { KeyRound, Percent } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAuth } from "@/hooks/use-auth";
import { type SalesAgent, formatBasisPoints, salesApi } from "@/lib/sales-api";

const MIN_PASSWORD_LENGTH = 8;

/**
 * Password change for the calling agent (SC-6.6).
 *
 * An agent account is created with a generated one-time password and is routed away from the
 * dashboard, so the ordinary account settings page is unreachable. Without this control the
 * generated password could never be replaced.
 */
export function SalesPasswordCard() {
  const { t } = useTranslation();
  const { changePassword } = useAuth();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [saving, setSaving] = useState(false);

  const tooShort = newPassword.length > 0 && newPassword.length < MIN_PASSWORD_LENGTH;
  const mismatch = confirmPassword.length > 0 && newPassword !== confirmPassword;
  const submittable =
    currentPassword.length > 0
    && newPassword.length >= MIN_PASSWORD_LENGTH
    && newPassword === confirmPassword
    && !saving;

  const submit = async () => {
    if (!submittable) return;
    setSaving(true);
    try {
      await changePassword(currentPassword, newPassword);
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      toast.success(t("sales.account.passwordChanged"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card className="rounded-2xl">
      <CardContent className="flex flex-col gap-4 p-5">
        <div className="flex items-center gap-2">
          <KeyRound className="size-4 text-muted-foreground" aria-hidden />
          <h2 className="text-sm font-semibold">{t("sales.account.passwordTitle")}</h2>
        </div>
        <p className="text-sm text-muted-foreground">
          {t("sales.account.passwordDescription")}
        </p>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="sales-current-password">
              {t("sales.account.currentPassword")}
            </Label>
            <Input
              id="sales-current-password"
              type="password"
              autoComplete="current-password"
              className="h-11 rounded-xl"
              value={currentPassword}
              onChange={(event) => setCurrentPassword(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="sales-new-password">{t("sales.account.newPassword")}</Label>
            <Input
              id="sales-new-password"
              type="password"
              autoComplete="new-password"
              className="h-11 rounded-xl"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
              aria-invalid={tooShort}
            />
            {tooShort && (
              <p className="text-xs text-destructive">
                {t("sales.account.passwordTooShort", { min: MIN_PASSWORD_LENGTH })}
              </p>
            )}
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="sales-confirm-password">
              {t("sales.account.confirmPassword")}
            </Label>
            <Input
              id="sales-confirm-password"
              type="password"
              autoComplete="new-password"
              className="h-11 rounded-xl"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
              aria-invalid={mismatch}
            />
            {mismatch && (
              <p className="text-xs text-destructive">{t("sales.account.passwordMismatch")}</p>
            )}
          </div>
        </div>
        <Button
          type="button"
          className="h-11 rounded-xl"
          disabled={!submittable}
          onClick={() => void submit()}
        >
          {saving ? t("sales.account.saving") : t("sales.account.changePassword")}
        </Button>
      </CardContent>
    </Card>
  );
}

export interface SalesDiscountCardProps {
  agent: SalesAgent;
  maxDiscountBp: number;
  onUpdated: () => Promise<unknown>;
}

/**
 * Discount control for the calling agent (SC-1.2a).
 *
 * The discount is funded from this agent's own commission, so the agent chooses it and Admin
 * cannot change it on their behalf. The stated commission-after-discount makes the trade
 * explicit before it is saved.
 */
export function SalesDiscountCard({ agent, maxDiscountBp, onUpdated }: SalesDiscountCardProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(String(agent.discount_bp));
  const [saving, setSaving] = useState(false);

  const parsed = Number.parseInt(draft, 10);
  const valid = Number.isInteger(parsed) && parsed >= 0 && parsed <= maxDiscountBp;
  const changed = valid && parsed !== agent.discount_bp;

  const submit = async () => {
    if (!changed || saving) return;
    setSaving(true);
    try {
      await salesApi.updateOwnDiscount(parsed);
      await onUpdated();
      toast.success(t("sales.account.discountSaved"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
      setDraft(String(agent.discount_bp));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card className="rounded-2xl">
      <CardContent className="flex flex-col gap-4 p-5">
        <div className="flex items-center gap-2">
          <Percent className="size-4 text-muted-foreground" aria-hidden />
          <h2 className="text-sm font-semibold">{t("sales.account.discountTitle")}</h2>
        </div>
        <p className="text-sm text-muted-foreground">
          {t("sales.account.discountDescription", {
            max: formatBasisPoints(maxDiscountBp),
          })}
        </p>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="sales-discount-bp">
            {t("sales.account.discountLabel", { max: maxDiscountBp })}
          </Label>
          <Input
            id="sales-discount-bp"
            type="number"
            min={0}
            max={maxDiscountBp}
            step={1}
            inputMode="numeric"
            className="h-11 rounded-xl"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            aria-invalid={draft.length > 0 && !valid}
          />
          {valid && (
            <p className="text-xs text-muted-foreground">
              {t("sales.account.discountEffect", {
                discount: formatBasisPoints(parsed),
                commission: formatBasisPoints(maxDiscountBp - parsed),
              })}
            </p>
          )}
        </div>
        <Button
          type="button"
          className="h-11 rounded-xl"
          disabled={!changed || saving}
          onClick={() => void submit()}
        >
          {saving ? t("sales.account.saving") : t("sales.account.saveDiscount")}
        </Button>
      </CardContent>
    </Card>
  );
}
