import { useState } from "react";
import { SiAlipay, SiStripe, SiWechat } from "@icons-pack/react-simple-icons";
import { CreditCard } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { StorePaymentChannel } from "@/lib/store-api";
import { cn } from "@/lib/utils";

interface PaymentMethodsProps {
  channels: StorePaymentChannel[];
  selectedId: string | null;
  onSelect: (channel: StorePaymentChannel) => void;
}

function ChannelIcon({ channel }: { channel: StorePaymentChannel }) {
  const [imageFailed, setImageFailed] = useState(false);

  if (channel.icon_kind !== "builtin" && channel.icon_value && !imageFailed) {
    return (
      <img
        src={channel.icon_value}
        alt=""
        className="size-5 object-contain"
        aria-hidden="true"
        onError={() => setImageFailed(true)}
      />
    );
  }
  if (channel.adapter_kind === "alipay") return <SiAlipay className="size-5 text-[#1677ff]" />;
  if (channel.adapter_kind === "wechat") return <SiWechat className="size-5 text-[#07c160]" />;
  if (channel.adapter_kind === "stripe") return <SiStripe className="size-5 text-[#635bff]" />;
  return <CreditCard className="size-5" />;
}

export function PaymentMethods({ channels, selectedId, onSelect }: PaymentMethodsProps) {
  const { t } = useTranslation();

  return (
    <section className="w-full border-t pt-6" aria-labelledby="store-payment-title">
      <h2 id="store-payment-title" className="mb-3 text-sm font-semibold">
        {t("store.payment.title")}
      </h2>
      {channels.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("store.payment.empty")}</p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {channels.map((channel) => (
            <button
              key={channel.id}
              type="button"
              aria-pressed={selectedId === channel.id}
              aria-label={t("store.payment.select", { name: channel.name })}
              onClick={() => onSelect(channel)}
              className={cn(
                "flex min-h-11 items-center gap-3 rounded-xl border bg-card px-4 py-3 text-left text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                selectedId === channel.id
                  ? "border-foreground bg-accent"
                  : "hover:border-foreground/30 hover:bg-accent/50",
              )}
            >
              <ChannelIcon key={channel.icon_value ?? channel.id} channel={channel} />
              <span className="min-w-0 truncate">{channel.name}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
