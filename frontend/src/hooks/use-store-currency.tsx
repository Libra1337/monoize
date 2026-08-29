import { createContext, useContext, useMemo, useState, type ReactNode } from "react";
import type { StoreCurrency } from "@/lib/store-money";

interface StoreCurrencyContextValue {
  currency: StoreCurrency;
  setCurrency: (currency: StoreCurrency) => void;
}

const StoreCurrencyContext = createContext<StoreCurrencyContextValue | null>(null);

export function StoreCurrencyProvider({ children }: { children: ReactNode }) {
  const [currency, setCurrency] = useState<StoreCurrency>("CNY");
  const value = useMemo(() => ({ currency, setCurrency }), [currency]);
  return (
    <StoreCurrencyContext.Provider value={value}>
      {children}
    </StoreCurrencyContext.Provider>
  );
}

export function useStoreCurrency(): StoreCurrencyContextValue {
  const value = useContext(StoreCurrencyContext);
  if (!value) {
    throw new Error("useStoreCurrency must be used within StoreCurrencyProvider");
  }
  return value;
}
