import { useState } from "react";
import { LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface RedemptionPanelProps {
  onRedeem: (code: string) => Promise<void>;
  redeeming: boolean;
}

export function RedemptionPanel({ onRedeem, redeeming }: RedemptionPanelProps) {
  const { t } = useTranslation();
  const [code, setCode] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const normalized = code.trim();
    if (!normalized) return;
    setSubmitting(true);
    try {
      await onRedeem(normalized);
      setCode("");
      toast.success(t("store.redeem.success"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("store.redeem.failed"));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="w-full">
      <p className="mb-3 text-sm text-muted-foreground">{t("store.redeem.description")}</p>
      <form className="flex flex-col gap-3 sm:flex-row" onSubmit={submit}>
          <label className="sr-only" htmlFor="redemption-code">
            {t("store.redeem.code")}
          </label>
          <Input
            id="redemption-code"
            value={code}
            autoComplete="off"
            className="h-11 rounded-xl font-mono uppercase"
            placeholder={t("store.redeem.placeholder")}
            onChange={(event) => setCode(event.target.value.toUpperCase())}
          />
          <Button
            type="submit"
            className="h-11 rounded-xl px-5"
            disabled={!code.trim() || submitting || redeeming}
          >
            {submitting && <LoaderCircle className="animate-spin motion-reduce:animate-none" />}
            {t("store.redeem.submit")}
          </Button>
      </form>
      {redeeming && (
        <p className="mt-3 flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" />
          {t("store.ui.redeeming")}
        </p>
      )}
    </div>
  );
}
