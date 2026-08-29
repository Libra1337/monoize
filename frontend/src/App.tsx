import { Routes, Route, Navigate } from "react-router-dom";
import { SWRConfig } from "swr";
import { MotionConfig } from "framer-motion";
import { AuthProvider } from "@/hooks/use-auth";
import { ThemeProvider } from "@/hooks/use-theme";
import { Toaster } from "@/components/ui/sonner";
import { LoginPage } from "@/pages/login";
import { DashboardLayout } from "@/pages/layout";
import { DashboardPage } from "@/pages/dashboard";
import { AdminDashboardPage } from "@/pages/admin-dashboard";
import { AdminUsagePage } from "@/pages/admin-usage";
import { AdminRuntimePage } from "@/pages/admin-runtime";
import { ProvidersPage } from "@/pages/providers";
import { ApiKeysPage } from "@/pages/api-keys";
import { UsersPage } from "@/pages/users";
import { GroupsPage } from "@/pages/groups";
import { BillingPlansPage } from "@/pages/billing-plans";
import { SettingsPage } from "@/pages/settings";
import { UserSettingsPage } from "@/pages/user-settings";
import { PlaygroundPage } from "@/pages/playground";
import { RequestLogsPage } from "@/pages/request-logs";
import { ModelMetadataPage } from "@/pages/model-metadata";
import { PublicLayout } from "@/pages/public-layout";
import { WelcomePage } from "@/pages/welcome";
import { ApiDocsPage } from "@/pages/api-docs";
import { PublicMarketplacePage } from "@/pages/public-marketplace";
import { PublicStatusPage } from "@/pages/public-status";
import { PUBLIC_PATHS } from "@/public-routes";
import { StorePage } from "@/pages/store";
import { OrdersPage } from "@/pages/orders";
import { StoreAdminPage } from "@/pages/store-admin";
import { UsageAnalysisPage } from "@/pages/usage-analysis";
import { ModelMarketplacePage } from "@/pages/model-marketplace";
import { DashboardApiDocsPage } from "@/pages/dashboard-api-docs";
import { useAuth } from "@/hooks/use-auth";
import { StoreCurrencyProvider } from "@/hooks/use-store-currency";
import { EmptyState } from "@/components/ui/empty-state";
import { PageWrapper } from "@/components/ui/motion";
import { useTranslation } from "react-i18next";
import "@/i18n";

function AdminRoute({ children }: { children: React.ReactNode }) {
  const { user } = useAuth();
  const { t } = useTranslation();
  const isAdmin = user?.role === "super_admin" || user?.role === "admin";
  return isAdmin ? children : (
    <PageWrapper className="h-full min-h-0">
      <EmptyState
        title={t("auth.accessDenied")}
        description={t("auth.needAdminPrivileges")}
        className="h-full py-0"
      />
    </PageWrapper>
  );
}

function StoreAdminRoute() {
  const { user } = useAuth();
  const isAdmin = user?.role === "super_admin" || user?.role === "admin";
  return isAdmin ? <StoreAdminPage /> : <Navigate to="/dashboard/store" replace />;
}

function App() {
  return (
    <ThemeProvider>
      <MotionConfig reducedMotion="user">
      <StoreCurrencyProvider>
      <SWRConfig
        value={{
          revalidateOnFocus: true,
          revalidateOnReconnect: true,
          dedupingInterval: 2000,
        }}
      >
        <AuthProvider>
        <Routes>
          <Route path={PUBLIC_PATHS.login} element={<LoginPage />} />
          <Route element={<PublicLayout />}>
            <Route path={PUBLIC_PATHS.home} element={<WelcomePage />} />
            <Route path={PUBLIC_PATHS.apiDocs} element={<ApiDocsPage />} />
            <Route path={PUBLIC_PATHS.status} element={<PublicStatusPage />} />
            <Route path={PUBLIC_PATHS.marketplace} element={<PublicMarketplacePage />} />
          </Route>
          {/* Dashboard routes - admin panel */}
          <Route path="/dashboard" element={<DashboardLayout />}>
            <Route index element={<DashboardPage />} />
            <Route path="usage" element={<UsageAnalysisPage />} />
            <Route path="status" element={<PublicStatusPage refreshInterval={2000} dashboard />} />
            <Route path="admin" element={<AdminRoute><AdminDashboardPage /></AdminRoute>} />
            <Route path="admin/usage" element={<AdminRoute><AdminUsagePage /></AdminRoute>} />
            <Route path="admin/runtime" element={<AdminRoute><AdminRuntimePage /></AdminRoute>} />
            <Route path="providers" element={<ProvidersPage />} />
            <Route path="tokens" element={<ApiKeysPage />} />
            <Route path="logs" element={<RequestLogsPage />} />
            <Route path="playground" element={<PlaygroundPage />} />
            <Route path="marketplace" element={<ModelMarketplacePage />} />
            <Route path="api-docs" element={<DashboardApiDocsPage />} />
            <Route path="models" element={<ModelMetadataPage />} />
            <Route path="users" element={<UsersPage />} />
            <Route path="groups" element={<GroupsPage />} />
            <Route path="plans" element={<BillingPlansPage />} />
            <Route path="store" element={<StorePage />} />
            <Route path="orders" element={<OrdersPage />} />
            <Route path="store-admin" element={<StoreAdminRoute />} />
            <Route path="admin-settings" element={<SettingsPage />} />
          </Route>
          {/* User settings routes */}
          <Route path="/settings" element={<DashboardLayout />}>
            <Route index element={<UserSettingsPage />} />
          </Route>
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
        </AuthProvider>
      </SWRConfig>
      </StoreCurrencyProvider>
      <Toaster />
      </MotionConfig>
    </ThemeProvider>
  );
}

export default App;
