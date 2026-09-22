import * as React from "react";
import useSWR from "swr";
import { api, type Me } from "@/lib/api";

interface AuthValue {
  me: Me | null;
  isAdmin: boolean;
  refresh: () => void;
  logout: () => Promise<void>;
}

const AuthContext = React.createContext<AuthValue>({
  me: null,
  isAdmin: false,
  refresh: () => {},
  logout: async () => {},
});

async function fetchMe(): Promise<Me> {
  return api.get<Me>("/me");
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const { data, mutate } = useSWR<Me>("me", fetchMe, {
    revalidateOnFocus: true,
    shouldRetryOnError: false,
  });

  React.useEffect(() => {
    const handler = () => {
      void mutate(undefined, { revalidate: true });
    };
    window.addEventListener("apeiron:unauthorized", handler);
    return () => window.removeEventListener("apeiron:unauthorized", handler);
  }, [mutate]);

  const logout = React.useCallback(async () => {
    await api.post("/auth/logout").catch(() => undefined);
    await mutate(undefined, { revalidate: true });
  }, [mutate]);

  const value = React.useMemo(
    () => ({
      me: data ?? null,
      isAdmin: data?.user.role === "admin" || data?.user.role === "super_admin",
      refresh: () => {
        void api.get<Me>("/me?refresh=1").then(
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
  return React.useContext(AuthContext);
}
