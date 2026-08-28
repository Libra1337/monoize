import type { StorePaymentChannel } from "@/lib/store-api";

export function disabledOptimisticChannel(
  channel: StorePaymentChannel,
): StorePaymentChannel {
  return {
    ...channel,
    enabled: false,
    revision: channel.revision + 1,
    effective_available: false,
    unavailable_reasons: ["channel_disabled"],
    supported_currencies: [],
    amount_limits: {},
    checkout_action_kinds: [],
  };
}
