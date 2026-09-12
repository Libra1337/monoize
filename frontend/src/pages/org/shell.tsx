import { useMemo } from "react";
import { Link, Navigate, NavLink, Outlet, useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Building2, Coins, KeyRound, LayoutDashboard, UsersRound } from "lucide-react";
import { api, type OrgSummary } from "@/lib/api";
import { useAuth } from "@/hooks/use-auth";
import { DashboardGuard } from "@/pages/sales/sales-route";
import { EmptyState } from "@/components/ui/empty-state";
import { Button } from "@/components/ui/button";
import { UserCenterMenu } from "@/components/user-center-menu";
import { TooltipProvider } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { motion, springs } from "@/components/ui/motion";

export const ORGS_KEY = "/api/dashboard/orgs";

export function useMyOrgs() {
  return useSWR<OrgSummary[]>(ORGS_KEY, () => api.listMyOrgs(), { fallbackData: [] });
}

export function OrgAvatar({
  emoji,
  color,
  image,
  size = "size-10",
  text = "text-xl",
}: {
  emoji: string;
  color: string;
  image?: string | null;
  size?: string;
  text?: string;
}) {
  if (image) {
    return <img src={image} alt="" className={`${size} shrink-0 rounded-xl object-cover`} />;
  }
  return (
    <div
      className={`${size} ${text} flex shrink-0 items-center justify-center rounded-xl`}
      style={{ backgroundColor: `${color}22`, color }}
    >
      {emoji}
    </div>
  );
}

/** The org-space shell: its own sidebar (identity, switcher, nav) plus the mode toggle. */
export function OrgShell() {
  const { t } = useTranslation();
  const { orgId } = useParams();
  const navigate = useNavigate();
  const { user, loading } = useAuth();
  const { data: orgs, isLoading } = useMyOrgs();
  const active = useMemo(
    () => orgs?.find((org) => org.id === orgId) ?? orgs?.[0],
    [orgs, orgId],
  );

  const navItems = [
    { to: "home", icon: LayoutDashboard, label: t("org.navHome"), end: true },
    { to: "members", icon: UsersRound, label: t("org.navMembers") },
    { to: "keys", icon: KeyRound, label: t("org.navKeys") },
    { to: "wallet", icon: Coins, label: t("org.navWallet") },
  ];

  if (loading || isLoading) {
    return <div className="flex min-h-dvh items-center justify-center text-sm text-muted-foreground">…</div>;
  }
  if (!user) {
    return <Navigate to="/login" replace />;
  }

  if (!active) {
    return (
      <div className="flex min-h-dvh flex-col items-center justify-center gap-4 p-6">
        <EmptyState
          variant="card"
          icon={<Building2 className="h-12 w-12" />}
          title={t("org.empty")}
          description={t("org.emptyJoinHint")}
          action={
            <Button asChild variant="outline">
              <Link to="/dashboard">{t("org.backToDashboard")}</Link>
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <TooltipProvider delayDuration={0}>
      <div className="flex h-dvh overflow-hidden">
        <aside className="flex w-60 shrink-0 flex-col border-r bg-background">
          <div className="flex items-center gap-2 p-3">
            <Link to="/org" className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg p-1 transition-colors hover:bg-accent/50">
              <OrgAvatar emoji={active.avatar_emoji} color={active.avatar_color} image={active.avatar_image} />
              <div className="min-w-0">
                <p className="truncate text-sm font-semibold">{active.display_name}</p>
                <p className="text-xs text-muted-foreground">
                  {active.role === "owner" ? t("org.owner") : t("org.member")}
                  {" · "}
                  {t("org.members", { count: active.member_count })}
                </p>
              </div>
            </Link>
          </div>

          {(orgs?.length ?? 0) > 1 && (
            <div className="flex flex-wrap gap-1 px-3 pb-2">
              {orgs
                ?.filter((org) => org.id !== active.id)
                .map((org) => (
                  <button
                    key={org.id}
                    type="button"
                    onClick={() => navigate(`/org/${org.id}/home`)}
                    className="flex items-center gap-1.5 rounded-full border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent/50"
                  >
                    <OrgAvatar emoji={org.avatar_emoji} color={org.avatar_color} image={org.avatar_image} size="size-5" text="text-xs" />
                    <span className="max-w-24 truncate">{org.display_name}</span>
                  </button>
                ))}
            </div>
          )}

          <nav className="flex flex-1 flex-col gap-0.5 px-3">
            {navItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  cn(
                    "relative flex items-center gap-3 rounded-md px-2.5 py-2 text-sm font-medium transition-colors",
                    isActive ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                  )
                }
              >
                <item.icon className="h-4 w-4 shrink-0" />
                {item.label}
              </NavLink>
            ))}
          </nav>

          <div className="space-y-2 p-3">
            <div className="flex items-center gap-1 rounded-lg bg-muted p-1" role="group" aria-label={t("nav.modeSwitch")}>
              <Link
                to="/dashboard"
                className="relative flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
              >
                <LayoutDashboard className="size-3.5" />
                {t("nav.workspace")}
              </Link>
              <Link
                to="/org"
                className="relative flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-xs font-medium text-foreground"
              >
                <motion.span
                  layoutId="org-mode-toggle-indicator"
                  className="absolute inset-0 rounded-md bg-background shadow-sm"
                  transition={springs.snappy}
                />
                <Building2 className="relative z-10 size-3.5" />
                <span className="relative z-10">{t("nav.orgSpace")}</span>
              </Link>
            </div>
            {user && (
              <div className="flex items-center justify-between rounded-lg px-1">
                <span className="truncate text-xs text-muted-foreground">{user.username}</span>
                <UserCenterMenu />
              </div>
            )}
          </div>
        </aside>

        <main className="min-w-0 flex-1 overflow-y-auto">
          <Outlet key={active.id} />
        </main>
      </div>
    </TooltipProvider>
  );
}

export { DashboardGuard, springs };
