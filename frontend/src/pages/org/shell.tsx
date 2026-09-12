import { useMemo, useState } from "react";
import { Link, Navigate, NavLink, Outlet, useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { mutate } from "swr";
import {
  Building2,
  ChartNoAxesCombined,
  Coins,
  DatabaseZap,
  KeyRound,
  LayoutDashboard,
  Plus,
  ScrollText,
  UsersRound,
} from "lucide-react";
import { useAuth } from "@/hooks/use-auth";
import { EmptyState } from "@/components/ui/empty-state";
import { Button } from "@/components/ui/button";
import { UserCenterMenu } from "@/components/user-center-menu";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";
import { motion, springs } from "@/components/ui/motion";
import { CreateOrgDialog } from "./entry";
import { ORGS_KEY, useMyOrgs, OrgAvatar } from "./shared";

/** The org-space shell: the workspace sidebar design with org identity and org navigation. */
export function OrgShell() {
  const { orgId } = useParams();
  const navigate = useNavigate();
  const { t } = useTranslation();
  const { user, loading } = useAuth();
  const { data: overview, isLoading } = useMyOrgs();
  const orgs = overview?.orgs;
  const [createOpen, setCreateOpen] = useState(false);
  const active = useMemo(
    () => orgs?.find((org) => org.id === orgId) ?? orgs?.[0],
    [orgs, orgId],
  );

  const navItems = [
    { to: "home", icon: LayoutDashboard, label: t("org.navHome"), exact: true },
    { to: "usage", icon: ChartNoAxesCombined, label: t("nav.usage") },
    { to: "usage/cache", icon: DatabaseZap, label: t("nav.cacheHitRate") },
    { to: "logs", icon: ScrollText, label: t("nav.logs") },
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
  const canCreate = overview?.can_create ?? false;

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
        <aside className="w-60 shrink-0 border-r">
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.2 }}
            className="flex h-full flex-col p-3"
          >
            {/* Brand slot: the org identity, the same row layout as the workspace logo. */}
            <Link
              to={`/org/${active.id}/home`}
              className="group flex items-center gap-3 rounded-lg px-2.5 py-2.5 transition-colors hover:bg-accent/50"
            >
              <OrgAvatar
                emoji={active.avatar_emoji}
                color={active.avatar_color}
                image={active.avatar_image}
                size="size-8"
                text="text-base"
              />
              <div className="min-w-0">
                <p className="truncate text-sm font-semibold tracking-tight">{active.display_name}</p>
                <p className="truncate text-xs text-muted-foreground">
                  {active.role === "owner" ? t("org.owner") : t("org.member")}
                  {" · "}
                  {t("org.members", { count: active.member_count })}
                </p>
              </div>
            </Link>

            {/* Mode toggle: the same control as the workspace sidebar, org side active. */}
            <div
              className="mt-2 flex items-center gap-1 rounded-lg bg-muted p-1"
              role="group"
              aria-label={t("nav.modeSwitch")}
            >
              <Link
                to="/dashboard"
                className="relative flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
                title={t("nav.workspace")}
              >
                <LayoutDashboard className="size-3.5" />
                {t("nav.workspace")}
              </Link>
              <Link
                to={`/org/${active.id}/home`}
                className="relative flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-xs font-medium text-foreground"
                title={t("nav.orgSpace")}
              >
                {/* The same layoutId as the workspace toggle, so the pill slides
                    across the workspace ⇄ org transition. */}
                <motion.span
                  layoutId="mode-toggle-indicator"
                  className="absolute inset-0 rounded-md bg-background shadow-sm"
                  transition={springs.snappy}
                />
                <Building2 className="relative z-10 size-3.5" />
                <span className="relative z-10">{t("nav.orgSpace")}</span>
              </Link>
            </div>

            <Separator className="my-3" />

            {/* Multi-org switcher chips; the plus opens creation for eligible owners. */}
            <div className="mb-2 flex flex-wrap gap-1">
              {orgs
                ?.filter((org) => org.id !== active.id)
                .map((org) => (
                  <button
                    key={org.id}
                    type="button"
                    onClick={() => navigate(`/org/${org.id}/home`)}
                    className="flex items-center gap-1.5 rounded-full border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent/50"
                  >
                    <OrgAvatar
                      emoji={org.avatar_emoji}
                      color={org.avatar_color}
                      image={org.avatar_image}
                      size="size-4"
                      text="text-[10px]"
                    />
                    <span className="max-w-24 truncate">{org.display_name}</span>
                  </button>
                ))}
              {canCreate && (
                <button
                  type="button"
                  onClick={() => setCreateOpen(true)}
                  className="flex items-center gap-1 rounded-full border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent/50"
                  title={t("org.create")}
                >
                  <Plus className="size-3.5" />
                </button>
              )}
            </div>

            {/* Org navigation: the same NavLink treatment as the workspace sidebar. */}
            <nav className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
              {navItems.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  end={item.exact}
                  className={({ isActive }) =>
                    cn(
                      "relative flex items-center gap-3 rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
                      isActive
                        ? "text-foreground"
                        : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                    )
                  }
                >
                  {({ isActive }) => (
                    <>
                      {isActive && (
                        <motion.span
                          layoutId="org-nav-active"
                          className="absolute inset-0 rounded-md bg-accent"
                          transition={springs.snappy}
                        />
                      )}
                      <item.icon className="relative z-10 h-4 w-4 shrink-0" />
                      <span className="relative z-10">{item.label}</span>
                    </>
                  )}
                </NavLink>
              ))}
            </nav>

            {/* Account menu: identical to the workspace bottom slot. */}
            <div className="mt-auto pt-3">
              <Separator className="mb-3" />
              <UserCenterMenu />
            </div>
          </motion.div>
        </aside>

        <main className="min-w-0 flex-1 overflow-y-auto">
          <Outlet key={active.id} />
        </main>

        <CreateOrgDialog
          open={createOpen}
          onOpenChange={setCreateOpen}
          onCreated={async (createdId) => {
            await mutate(ORGS_KEY);
            navigate(`/org/${createdId}/home`);
          }}
        />
      </div>
    </TooltipProvider>
  );
}
