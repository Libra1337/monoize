import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";

export type PaymentWatchStatus = "watching" | "succeeded";

export interface PaymentWatchProps {
  /** Epoch milliseconds at which polling for this order began. */
  startedAt: number;
  /** Completed status checks, including the one in flight. */
  attempts: number;
  status: PaymentWatchStatus;
}

/**
 * Success mark for a settled payment (SB-UI-10E).
 *
 * The ring and the tick are drawn by animating `stroke-dashoffset` from the full path length
 * to zero, so the mark draws itself in one stroke rather than appearing at once. Both paths
 * carry `pathLength="1"`, which normalizes the dash units and keeps the timing independent of
 * the actual geometry.
 */
function SuccessMark() {
  return (
    <svg
      viewBox="0 0 52 52"
      className="size-16"
      fill="none"
      strokeWidth={3}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle
        cx="26"
        cy="26"
        r="23"
        pathLength="1"
        className="stroke-emerald-500 [stroke-dasharray:1] [animation:payment-success-draw_0.5s_cubic-bezier(0.65,0,0.35,1)_forwards]"
      />
      <path
        d="M15 27.5 L22.5 35 L37 19"
        pathLength="1"
        className="stroke-emerald-500 [stroke-dasharray:1] [animation:payment-success-draw_0.35s_cubic-bezier(0.65,0,0.35,1)_0.4s_forwards]"
      />
    </svg>
  );
}

/**
 * Progress readout for the checkout QR dialog (SB-UI-10C).
 *
 * The dialog polls payment status regardless of this component; without a visible readout the
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
        className="flex flex-col items-center justify-center gap-3 py-2"
        role="status"
        aria-live="polite"
      >
        <SuccessMark />
        <p className="text-base font-medium text-emerald-600 [animation:payment-success-rise_0.3s_ease-out_0.6s_backwards] dark:text-emerald-400">
          {t("store.payment.watchSucceeded")}
        </p>
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
