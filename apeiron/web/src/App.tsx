import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { SWRConfig } from "swr";
import { Toaster } from "sonner";
import { MotionConfig } from "framer-motion";
import { AppShell } from "@/components/app-shell";
import { AuthProvider, useAuth } from "@/auth";
import { LandingPage } from "@/pages/landing";
import { HandoffPage } from "@/pages/handoff";
import { CreatePage } from "@/pages/create";
import { ProjectsPage } from "@/pages/projects";
import { CanvasPage } from "@/pages/canvas";
import { TemplatesPage } from "@/pages/templates";
import { AssetsPage } from "@/pages/assets";
import { RunsPage } from "@/pages/runs";
import { ManagerPage } from "@/pages/manager";
import { AdminPage } from "@/pages/admin";

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { me } = useAuth();
  // `undefined` means the /me probe is still in flight; `null` is signed out.
  if (me === null) return <Navigate to="/handoff" replace />;
  return <>{children}</>;
}

function RequireAdmin({ children }: { children: React.ReactNode }) {
  const { me, isAdmin } = useAuth();
  if (me === null) return <Navigate to="/handoff" replace />;
  if (me && !isAdmin) return <Navigate to="/" replace />;
  return <>{children}</>;
}

export default function App() {
  return (
    <BrowserRouter>
    <MotionConfig reducedMotion="user">
      <SWRConfig
        value={{
          revalidateOnFocus: true,
          revalidateOnReconnect: true,
          dedupingInterval: 2000,
          shouldRetryOnError: false,
        }}
      >
        <AuthProvider>
          <Routes>
            <Route path="/" element={<LandingPage />} />
            <Route path="/handoff" element={<HandoffPage />} />
            <Route
              element={
                <RequireAuth>
                  <AppShell />
                </RequireAuth>
              }
            >
              <Route path="/create" element={<CreatePage />} />
              <Route path="/projects" element={<ProjectsPage />} />
              <Route path="/canvas/:projectId" element={<CanvasPage />} />
              <Route path="/templates" element={<TemplatesPage />} />
              <Route path="/assets" element={<AssetsPage />} />
              <Route path="/runs" element={<RunsPage />} />
              <Route path="/manager" element={<ManagerPage />} />
              <Route
                path="/admin"
                element={
                  <RequireAdmin>
                    <AdminPage />
                  </RequireAdmin>
                }
              />
            </Route>
          </Routes>
          <Toaster position="top-center" richColors closeButton />
        </AuthProvider>
      </SWRConfig>
    </MotionConfig>
    </BrowserRouter>
  );
}
