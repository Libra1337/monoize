import { useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { motion } from "@/components/ui/motion";

export type StoreTab = "balance" | "plan" | "redeem";

interface StoreTabsProps {
  activeTab: StoreTab;
  onTabChange: (tab: StoreTab) => void;
}

const tabs: StoreTab[] = ["balance", "plan", "redeem"];

export function StoreTabs({ activeTab, onTabChange }: StoreTabsProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();

  return (
    <div
      className="grid w-full grid-cols-3 gap-1 rounded-xl bg-muted p-1 sm:w-fit sm:min-w-[360px]"
      role="tablist"
      aria-label={t("store.ui.tabsLabel")}
    >
      {tabs.map((tab) => (
        <button
          key={tab}
          type="button"
          role="tab"
          aria-selected={activeTab === tab}
          className="relative flex min-h-11 items-center justify-center rounded-lg px-4 text-sm font-medium text-muted-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 data-[active=true]:text-foreground"
          data-active={activeTab === tab}
          onClick={() => onTabChange(tab)}
        >
          {activeTab === tab && (
            <motion.span
              layoutId="store-tab-indicator"
              className="absolute inset-0 rounded-lg bg-background shadow-sm"
              transition={reduceMotion ? { duration: 0 } : { type: "spring", bounce: 0.15, duration: 0.35 }}
            />
          )}
          <span className="relative z-10">{t(`store.tabs.${tab}`)}</span>
        </button>
      ))}
    </div>
  );
}
