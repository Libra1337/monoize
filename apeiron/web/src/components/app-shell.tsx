import * as React from "react";
import { Link, NavLink, Outlet, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Clapperboard, Coins, Languages, LogOut, Moon, Sun, Sparkles, User } from "lucide-react";
import { useAuth } from "@/auth";
import { ApeironLogo } from "@/components/logo";
import { Button } from "@/components/ui/button";
import { TooltipProvider, Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/controls";
import { cycleLanguage } from "@/i18n";
import { nanoToUsd } from "@/lib/format";
import { cn } from "@/lib/utils";

export const GRID_TEXTURE =
  "bg-[radial-gradient(circle_at_72%_22%,hsl(var(--primary)/0.15),transparent_34%),linear-gradient(to_right,hsl(var(--border)/0.35)_1px,transparent_1px),linear-gradient(to_bottom,hsl(var(--border)/0.35)_1px,transparent_1px)] bg-[size:auto,32px_32px,32px_32px] [mask-image:linear-gradient(to_bottom,black,transparent)]";

function useTheme() {
  const [dark, setDark] = React.useState(() =>
    document.documentElement.classList.contains("dark"),
  );
  const toggle = React.useCallback(() => {
    setDark((current) => {
      const next = !current;
      document.documentElement.classList.toggle("dark", next);
      localStorage.setItem("apeiron-theme", next ? "dark" : "light");
      return next;
    });
  }, []);
  return { dark, toggle };
}

function BalanceChip() {
  const { me, refresh } = useAuth();
  const { t } = useTranslation();
  if (!me) return null;
  return (
    <button
      onClick={refresh}
      className="inline-flex h-8 items-center gap-1.5 rounded-md border px-2.5 font-mono text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      title={t("common.balance")}
    >
      <Coins className="h-3.5 w-3.5" />
      {me.balance_unlimited ? "∞" : nanoToUsd(me.balance_nano_usd)}
    </button>
  );
}

export function AppShell() {
  const { t } = useTranslation();
  const { me, isAdmin, logout } = useAuth();
  const navigate = useNavigate();
  const { dark, toggle } = useTheme();

  const nav = [
    { to: "/create", icon: Sparkles, label: t("nav.create") },
    { to: "/projects", icon: Clapperboard, label: t("nav.projects") },
    { to: "/templates", icon: LayersIcon, label: t("nav.templates") },
    { to: "/assets", icon: ImageIconSafe, label: t("nav.assets") },
    { to: "/runs", icon: ActivityIcon, label: t("nav.runs") },
    { to: "/manager", icon: WrenchIcon, label: t("nav.manager") },
  ];

  return (
    <TooltipProvider delayDuration={0}>
      <div className="flex h-dvh flex-col overflow-hidden bg-background">
        <header className="sticky top-0 z-40 border-b bg-background/92 px-4 backdrop-blur-xl sm:px-6">
          <div className="mx-auto flex h-14 max-w-7xl items-center gap-4">
            <Link to="/" className="flex items-center gap-2.5">
              <span className="flex size-8 items-center justify-center rounded-lg border bg-card">
                <ApeironLogo className="h-4.5 w-4.5" />
              </span>
              <span className="font-display text-sm font-semibold tracking-tight">
                {t("brand.name")}
              </span>
            </Link>
            <nav className="ml-2 hidden items-center gap-1 md:flex">
              {nav.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  className={({ isActive }) =>
                    cn(
                      "rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
                      isActive
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                    )
                  }
                >
                  {item.label}
                </NavLink>
              ))}
              {isAdmin ? (
                <NavLink
                  to="/admin"
                  className={({ isActive }) =>
                    cn(
                      "rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
                      isActive
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                    )
                  }
                >
                  {t("nav.admin")}
                </NavLink>
              ) : null}
            </nav>
            <div className="ml-auto flex items-center gap-1.5">
              <BalanceChip />
              {me ? (
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() =>
                    window.open(`${me.platform_url}/dashboard/wallet`, "_blank", "noreferrer")
                  }
                >
                  <Coins />
                  <span className="hidden sm:inline">{t("nav.topup")}</span>
                </Button>
              ) : null}
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" onClick={cycleLanguage} aria-label={t("common.language")}>
                    <Languages />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("common.language")}</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" onClick={toggle} aria-label={t("common.theme")}>
                    {dark ? <Sun /> : <Moon />}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("common.theme")}</TooltipContent>
              </Tooltip>
              {me ? (
                <div className="hidden items-center gap-1.5 pl-1 sm:flex">
                  <span className="flex size-7 items-center justify-center rounded-md bg-muted">
                    <User className="h-3.5 w-3.5 text-muted-foreground" />
                  </span>
                  <span className="max-w-28 truncate text-sm text-muted-foreground">
                    {me.user.display_name || me.user.username}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={t("nav.logout")}
                    onClick={async () => {
                      await logout();
                      navigate("/handoff");
                    }}
                  >
                    <LogOut />
                  </Button>
                </div>
              ) : null}
            </div>
          </div>
        </header>
        <main className="flex min-h-0 flex-1 flex-col overflow-y-auto px-4 py-6 sm:px-6">
          <div className="mx-auto flex min-h-0 w-full max-w-7xl flex-1 flex-col">
            <Outlet />
          </div>
        </main>
      </div>
    </TooltipProvider>
  );
}

// Icon imports kept local to avoid a wide top-level import list.
import { Activity as ActivityIcon, Image as ImageIconSafe, Layers as LayersIcon, Wrench as WrenchIcon } from "lucide-react";
