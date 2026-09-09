import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Monitor, Moon, Sun } from "lucide-react";
import { motion } from "framer-motion";
import { useTheme } from "@/hooks/use-theme";
import { springs } from "@/components/ui/motion";

/**
 * Light, dark, and system theme selector.
 *
 * The sliding indicator uses a `layoutId` generated per instance, so two mounted selectors
 * animate independently instead of the indicator flying between them.
 */
export function ThemeToggle({ label }: { label?: string } = {}) {
  const { theme, setTheme } = useTheme();
  const { t } = useTranslation();
  const indicatorId = useId();

  const themes = [
    { value: "light", icon: Sun, label: t("theme.light") },
    { value: "dark", icon: Moon, label: t("theme.dark") },
    { value: "system", icon: Monitor, label: t("theme.system") },
  ] as const;

  return (
    <div className="flex items-center justify-between gap-2 px-2 py-1.5">
      <span className="text-sm text-muted-foreground">{label ?? t("theme.toggle")}</span>
      <div className="relative flex h-8 items-center rounded-full bg-muted p-1">
        {themes.map((item) => {
          const Icon = item.icon;
          const isActive = theme === item.value;
          return (
            <button
              key={item.value}
              type="button"
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                setTheme(item.value);
              }}
              className={`relative z-10 flex h-6 w-8 items-center justify-center rounded-full transition-colors ${
                isActive ? "text-foreground" : "text-muted-foreground hover:text-foreground"
              }`}
              title={item.label}
              aria-label={item.label}
              aria-pressed={isActive}
            >
              {isActive && (
                <motion.div
                  layoutId={`theme-toggle-indicator-${indicatorId}`}
                  className="absolute inset-0 rounded-full bg-background shadow-sm"
                  transition={springs.snappy}
                />
              )}
              <Icon className="relative z-10 h-3.5 w-3.5" />
            </button>
          );
        })}
      </div>
    </div>
  );
}
