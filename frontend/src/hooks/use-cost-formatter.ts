import { useCallback } from "react";
import { formatNanoUsd, isSignedIntegerString } from "@/lib/exact-decimal";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";

/**
 * Formats a nano-USD charge string in the Console display currency (DL3h): CNY through the
 * loaded CNY/USD snapshot while it is available, USD with 6 fractional digits otherwise.
 */
export function useCostFormatter() {
  const { currency } = useStoreCurrency();
  const exchangeRate = useStoreExchangeRate(true);
  const cnyPerUsd = exchangeRate.data?.cny_per_usd;
  return useCallback(
    (nanoUsd: string | null | undefined): string => {
      if (nanoUsd == null || !isSignedIntegerString(nanoUsd)) return "-";
      if (currency === "CNY" && cnyPerUsd) {
        return formatCoinFromNanoUsdForCurrency(nanoUsd, "CNY", cnyPerUsd);
      }
      return formatNanoUsd(nanoUsd, 6);
    },
    [currency, cnyPerUsd],
  );
}
