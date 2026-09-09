import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Loader2 } from "lucide-react";

export type PaymentWatchStatus = "watching" | "succeeded";

export interface PaymentWatchProps {
  /** Epoch milliseconds at which polling for this order began. */
  startedAt: number;
  /** Completed status checks, including the one in flight. */
  attempts: number;
  status: PaymentWatchStatus;
}

/**
 * Progress readout for the checkout QR dialog (SB-UI-10C).
 *
 * The dialog polls order status regardless of this component; without a visible readout the
 * buyer cannot tell whether the payment was noticed, so the elapsed time and attempt count
 * are surfaced to make the wait legible. On success the dialog holds this component in its
 * succeeded state briefly before closing, so confirmation is seen rather than inferred from
 * the dialog vanishing.
 */
export function PaymentWatch({ startedAt, attempts, status }: PaymentWatchProps) {
  const { t } = useTranslation();
  const [elapsedSeconds, setElapsedSeconds] = useState(() =>
    Math.max(0, Math.floor((Date.now() - startedAt) / 1000)),
  );

  useEffect(() => {
    if (status !== "watching") return;
    const id = window.setInterval(() => {
      setElapsedSeconds(Math.max(0, Math.floor((Date.now() - startedAt) / 1000)));
    }, 1_000);
    return () => window.clearInterval(id);
  }, [startedAt, status]);

  if (status === "succeeded") {
    return (
      <div
        className="flex items-center justify-center gap-2 text-sm font-medium text-emerald-600 dark:text-emerald-400"
        role="status"
        aria-live="polite"
      >
        <CheckCircle2 className="size-4" aria-hidden />
        {t("store.payment.watchSucceeded")}
      </div>
    );
  }

  return (
    <div
      className="flex items-center justify-center gap-2 text-sm text-muted-foreground"
      role="status"
      aria-live="polite"
    >
      <Loader2 className="size-4 animate-spin" aria-hidden />
      {t("store.payment.watching", { seconds: elapsedSeconds, attempts })}
    </div>
  );
}
