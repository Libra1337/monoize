import type { StoreOrder, StorePaymentAttempt } from "@/lib/store-api";

function canQueryAttempt(order: StoreOrder, attempt: StorePaymentAttempt): boolean {
  if (order.contract_version === 1 && order.payment_state === "closed") return false;
  if (attempt.adapter_kind === "http") return false;
  return !(attempt.adapter_kind === "stripe" && attempt.state === "created" && !attempt.provider_object_id);
}

export function canCloseAttempt(
  order: StoreOrder,
  attempts: StorePaymentAttempt[],
  attempt: StorePaymentAttempt,
): boolean {
  return (
    order.contract_version === 2
    && order.payment_state === "unpaid"
    && ["created", "presented", "failed"].includes(attempt.state)
    && !attempts.some((candidate) => (
      candidate.id !== attempt.id && ["created", "presented"].includes(candidate.state)
    ))
    && canQueryAttempt(order, attempt)
  );
}
