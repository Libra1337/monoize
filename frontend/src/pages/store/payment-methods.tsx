import { useState } from "react";
import { SiAlipay, SiStripe, SiWechat } from "@icons-pack/react-simple-icons";
import { CreditCard } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { StorePaymentOption } from "./store-selection";

interface PaymentMethodsProps {
  options: StorePaymentOption[];
  selectedId: string | null;
  onSelect: (option: StorePaymentOption) => void;
}

function OptionIcon({ option }: { option: StorePaymentOption }) {
  const [imageFailed, setImageFailed] = useState(false);

  if (option.iconKind !== "builtin" && option.iconValue && !imageFailed) {
    return (
      <img
        src={option.iconValue}
        alt=""
        className="size-5 object-contain"
        aria-hidden="true"
        onError={() => setImageFailed(true)}
      />
    );
  }
  if (option.method === "alipay") return <SiAlipay className="size-5 text-[#1677ff]" />;
  if (option.method === "wxpay") return <SiWechat className="size-5 text-[#07c160]" />;
  if (option.channel.adapter_kind === "stripe") {
    return <SiStripe className="size-5 text-[#635bff]" />;
  }
  return <CreditCard className="size-5" />;
}

export function PaymentMethods({ options, selectedId, onSelect }: PaymentMethodsProps) {
  const { t } = useTranslation();

  return (
    <section className="w-full border-t pt-6" aria-labelledby="store-payment-title">
      <h2 id="store-payment-title" className="mb-3 text-sm font-semibold">
        {t("store.payment.title")}
      </h2>
      {options.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("store.payment.empty")}</p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {options.map((option) => (
            <button
              key={option.id}
              type="button"
              aria-pressed={selectedId === option.id}
              aria-label={t("store.payment.select", { name: option.label })}
              onClick={() => onSelect(option)}
              className={cn(
                "flex min-h-11 items-center gap-3 rounded-xl border bg-card px-4 py-3 text-left text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                selectedId === option.id
                  ? "border-foreground bg-accent"
                  : "hover:border-foreground/30 hover:bg-accent/50",
              )}
            >
              <OptionIcon key={option.iconValue ?? option.id} option={option} />
              <span className="min-w-0 truncate">{option.label}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
