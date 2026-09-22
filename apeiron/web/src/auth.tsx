import * as React from "react";
import useSWR from "swr";
import { api, ApiError, type Me } from "@/lib/api";

interface AuthValue {
  /** undefined = probe in flight; null = signed out. */
  me: Me | null | undefined;
  isAdmin: boolean;
  refresh: () => void;
  logout: () => Promise<void>;
}

const AuthContext = React.createContext<AuthValue | null>(null);

/** 401 resolves to `null` (signed out); other failures stay `undefined`. */
async function fetchMe(): Promise<Me | null | undefined> {
  try {
    return await api.get<Me>("/me");
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) return null;
    return undefined;
  }
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const { data, mutate } = useSWR<Me | null | undefined>("me", fetchMe, {
    revalidateOnFocus: true,
    shouldRetryOnError: false,
  });

  React.useEffect(() => {
    const handler = () => {
      void mutate(undefined as unknown as Me | null, { revalidate: true });
    };
    window.addEventListener("apeiron:unauthorized", handler);
    return () => window.removeEventListener("apeiron:unauthorized", handler);
  }, [mutate]);

  const logout = React.useCallback(async () => {
    await api.post("/auth/logout").catch(() => undefined);
    await mutate(undefined as unknown as Me | null, { revalidate: true });
  }, [mutate]);

  const value = React.useMemo(
    () => ({
      me: data,
      isAdmin: data?.user.role === "admin" || data?.user.role === "super_admin",
      refresh: () => {
        void api
          .get<Me>("/me?refresh=1")
          .then(
            (fresh) => mutate(fresh, { revalidate: false }),
            () => undefined,
          );
      },
      logout,
    }),
    [data, logout, mutate],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthValue {
  const value = React.useContext(AuthContext);
  if (!value) throw new Error("useAuth must be used inside AuthProvider");
  return value;
}
