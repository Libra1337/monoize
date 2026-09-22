import * as React from "react";
import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  Activity,
  Boxes,
  Clapperboard,
  Coins,
  Languages,
  LayoutTemplate,
  LogOut,
  Moon,
  ShieldCheck,
  Sparkles,
  Sun,
  Images,
} from "lucide-react";
import { useAuth } from "@/auth";
import { ApeironLogo } from "@/components/logo";
import { TooltipProvider, Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/controls";
import { cycleLanguage } from "@/i18n";
import { nanoToUsd } from "@/lib/format";
import { cn } from "@/lib/utils";

/** Rail item: icon over a 10px label (Jimeng / libtv rail pattern). */
function RailItem({
  to,
  icon: Icon,
  label,
  active,
}: {
  to?: string;
  icon: typeof Sparkles;
  label: string;
  active?: boolean;
}) {
  const body = (
    <>
      <Icon className="h-[18px] w-[18px]" />
      <span className="text-[10px] leading-none">{label}</span>
    </>
  );
  const className = cn(
    "flex w-14 flex-col items-center gap-1 rounded-md py-2 transition-colors",
    active
      ? "bg-accent text-foreground [&_svg]:text-primary"
      : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
  );
  if (to) {
    return (
      <NavLink to={to} className={({ isActive }) => cn(className, isActive && "bg-accent text-foreground [&_svg]:text-primary")}>
        {body}
      </NavLink>
    );
  }
  return <button className={className}>{body}</button>;
}

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

/**
 * Pro-tool shell: a narrow icon rail (Jimeng smart-canvas / libtv pattern)
 * plus a full-bleed work area. Page chrome lives in compact WorkHeaders,
 * not marketing headers.
 */
export function AppShell() {
  const { t } = useTranslation();
  const { me, logout } = useAuth();
  const navigate = useNavigate();
  const { dark, toggle } = useTheme();

  const nav = [
    { to: "/create", icon: Sparkles, label: t("nav.create") },
    { to: "/projects", icon: Clapperboard, label: t("nav.projects") },
    { to: "/templates", icon: LayoutTemplate, label: t("nav.templates") },
    { to: "/assets", icon: Images, label: t("nav.assets") },
    { to: "/runs", icon: Activity, label: t("nav.runs") },
    { to: "/manager", icon: Boxes, label: t("nav.manager") },
  ];
  if (me?.user.role === "admin" || me?.user.role === "super_admin") {
    nav.push({ to: "/admin", icon: ShieldCheck, label: t("nav.admin") });
  }

  const operatorNav = nav.filter((item) => item.to === "/admin");
  const mainNav = nav.filter((item) => item.to !== "/admin" && item.to !== "/manager");
  const workNav = nav.filter((item) => item.to === "/manager");

  return (
    <TooltipProvider delayDuration={0}>
      <div className="flex h-dvh overflow-hidden bg-background">
        {/* Desktop icon rail with labels (Jimeng smart-canvas / libtv pattern) */}
        <aside className="hidden w-[72px] shrink-0 flex-col items-center gap-2 border-r py-3 md:flex">
          <NavLink
            to="/"
            className="mb-1 flex size-10 items-center justify-center rounded-lg bg-foreground text-background"
            aria-label="Apeiron"
          >
            <ApeironLogo className="h-5 w-5" />
          </NavLink>
          {mainNav.map((item) => (
            <RailItem key={item.to} to={item.to} icon={item.icon} label={item.label} />
          ))}
          {workNav.length > 0 ? (
            <div className="mt-2 w-14 border-t pt-2">
              {workNav.map((item) => (
                <div key={item.to} className="mb-2">
                  <RailItem to={item.to} icon={item.icon} label={item.label} />
                </div>
              ))}
            </div>
          ) : null}
          {operatorNav.length > 0 ? (
            <div className="mt-2 w-14 border-t pt-2">
              {operatorNav.map((item) => (
                <div key={item.to} className="mb-2">
                  <RailItem to={item.to} icon={item.icon} label={item.label} />
                </div>
              ))}
            </div>
          ) : null}
          <div className="mt-auto flex flex-col items-center gap-1">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  className="flex size-9 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                  onClick={cycleLanguage}
                  aria-label={t("common.language")}
                >
                  <Languages className="h-[18px] w-[18px]" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{t("common.language")}</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  className="flex size-9 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                  onClick={toggle}
                  aria-label={t("common.theme")}
                >
                  {dark ? <Sun className="h-[18px] w-[18px]" /> : <Moon className="h-[18px] w-[18px]" />}
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{t("common.theme")}</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  className="flex size-9 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                  aria-label={t("nav.logout")}
                  onClick={async () => {
                    await logout();
                    navigate("/handoff");
                  }}
                >
                  <LogOut className="h-[18px] w-[18px]" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{t("nav.logout")}</TooltipContent>
            </Tooltip>
          </div>
        </aside>

        {/* Work area */}
        <div className="flex min-w-0 flex-1 flex-col">
          <Outlet />
        </div>

        {/* Mobile bottom tab bar */}
        <nav className="fixed inset-x-0 bottom-0 z-40 flex items-center justify-around border-t bg-background/95 py-1.5 backdrop-blur md:hidden">
          {nav.slice(0, 5).map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              className={({ isActive }) =>
                cn(
                  "flex flex-col items-center gap-0.5 rounded-md px-3 py-1 text-[10px]",
                  isActive ? "text-primary" : "text-muted-foreground",
                )
              }
            >
              <item.icon className="h-5 w-5" />
              {item.label}
            </NavLink>
          ))}
        </nav>
      </div>
    </TooltipProvider>
  );
}

/** Compact tool header replacing marketing page headers inside the app. */
export function WorkHeader({
  title,
  description,
  actions,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b px-4">
      <h1 className="truncate text-sm font-medium">{title}</h1>
      {description ? (
        <p className="hidden truncate text-xs text-muted-foreground lg:block">{description}</p>
      ) : null}
      <div className="ml-auto flex shrink-0 items-center gap-1.5">{actions}</div>
    </header>
  );
}

export function BalanceChip() {
  const { me, refresh } = useAuth();
  const { t } = useTranslation();
  if (!me) return null;
  return (
    <button
      onClick={refresh}
      className="inline-flex h-7 items-center gap-1.5 rounded-md border px-2 font-mono text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      title={t("common.balance")}
    >
      <Coins className="h-3 w-3" />
      {me.balance_unlimited ? "∞" : nanoToUsd(me.balance_nano_usd)}
    </button>
  );
}

export function TopUpButton() {
  const { me } = useAuth();
  const { t } = useTranslation();
  if (!me) return null;
  return (
    <button
      onClick={() => window.open(`${me.platform_url}/dashboard/wallet`, "_blank", "noreferrer")}
      className="inline-flex h-7 items-center rounded-md bg-primary px-2.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90"
    >
      {t("nav.topup")}
    </button>
  );
}

