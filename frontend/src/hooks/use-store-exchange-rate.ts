import useSWR from "swr";
import { storeApi, type StoreExchangeRate } from "@/lib/store-api";

export const STORE_EXCHANGE_RATE_KEY = "/api/dashboard/store/exchange-rate";

export function useStoreExchangeRate(enabled = true) {
  return useSWR<StoreExchangeRate>(
    enabled ? STORE_EXCHANGE_RATE_KEY : null,
    storeApi.getExchangeRate,
    {
      keepPreviousData: true,
      refreshInterval: 60_000,
    },
  );
}
