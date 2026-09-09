import { Navigate } from "react-router-dom";
import { useAuth } from "@/hooks/use-auth";
import { SalesPage } from "./index";

/**
 * SC-UI-1: keeps the two surfaces separate in both directions.
 *
 * An agent exists only to sell, so the dashboard would offer capabilities the account is not
 * created for — including the Store, where SC-2.3 forbids an agent from using their own code.
 * A non-agent has nothing to see here.
 */
export function SalesRoute() {
  const { user, loading } = useAuth();

  if (loading) return null;
  if (!user) return <Navigate to="/login" replace />;
  if (!user.is_sales_agent) return <Navigate to="/dashboard" replace />;
  return <SalesPage />;
}

/** Wraps a dashboard route so an agent is sent to their own surface instead. */
export function DashboardGuard({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuth();

  if (loading) return null;
  if (user?.is_sales_agent) return <Navigate to="/sales" replace />;
  return <>{children}</>;
}
