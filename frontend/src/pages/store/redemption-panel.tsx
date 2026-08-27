import { useState } from "react";
import { Gift, LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
    <Card className="mx-auto w-full max-w-2xl rounded-2xl">
      <CardHeader className="p-6 pb-3">
        <div className="mb-2 flex size-11 items-center justify-center rounded-xl bg-muted">
          <Gift className="size-5" />
        </div>
        <CardTitle>{t("store.redeem.title")}</CardTitle>
        <p className="text-sm text-muted-foreground">{t("store.redeem.description")}</p>
      </CardHeader>
      <CardContent className="p-6 pt-3">
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
      </CardContent>
    </Card>
  );
}
