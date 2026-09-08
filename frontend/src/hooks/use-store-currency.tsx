import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import type { StoreCurrency } from "@/lib/store-money";

export const STORE_CURRENCY_STORAGE_KEY = "monoize-display-currency-v1";

interface StoreCurrencyContextValue {
  currency: StoreCurrency;
  setCurrency: (currency: StoreCurrency) => void;
}

const StoreCurrencyContext = createContext<StoreCurrencyContextValue | null>(null);

function initialCurrency(): StoreCurrency {
  if (typeof window === "undefined") return "CNY";
  try {
    const stored = window.localStorage.getItem(STORE_CURRENCY_STORAGE_KEY);
    return stored === "CNY" || stored === "USD" ? stored : "CNY";
  } catch {
    return "CNY";
  }
}

export function StoreCurrencyProvider({ children }: { children: ReactNode }) {
  const [currency, setCurrencyState] = useState<StoreCurrency>(initialCurrency);
  const setCurrency = useCallback((next: StoreCurrency) => {
    setCurrencyState(next);
    try {
      window.localStorage.setItem(STORE_CURRENCY_STORAGE_KEY, next);
    } catch {
      // Browser storage can be unavailable without invalidating the in-memory preference.
    }
  }, []);
  const value = useMemo(() => ({ currency, setCurrency }), [currency, setCurrency]);
  return (
    <StoreCurrencyContext.Provider value={value}>
      {children}
    </StoreCurrencyContext.Provider>
  );
}

// The hook and provider share one private Context and must stay in this module.
// eslint-disable-next-line react-refresh/only-export-components
export function useStoreCurrency(): StoreCurrencyContextValue {
  const value = useContext(StoreCurrencyContext);
  if (!value) {
    throw new Error("useStoreCurrency must be used within StoreCurrencyProvider");
  }
  return value;
}
