import { useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { motion } from "@/components/ui/motion";

export type StoreAdminTab = "products" | "channels" | "orders" | "redemptions";

interface StoreAdminTabsProps {
  activeTab: StoreAdminTab;
  onTabChange: (tab: StoreAdminTab) => void;
}

const tabs: StoreAdminTab[] = ["products", "channels", "orders", "redemptions"];

export function StoreAdminTabs({ activeTab, onTabChange }: StoreAdminTabsProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();

  return (
    <div
      className="grid w-full grid-cols-2 gap-1 rounded-xl bg-muted p-1 sm:grid-cols-4"
      role="tablist"
      aria-label={t("store.admin.tabsLabel")}
    >
      {tabs.map((tab) => (
        <button
          key={tab}
          type="button"
          role="tab"
          aria-selected={activeTab === tab}
          className="relative flex min-h-11 cursor-pointer items-center justify-center rounded-lg px-4 text-sm font-medium text-muted-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 data-[active=true]:text-foreground"
          data-active={activeTab === tab}
          onClick={() => onTabChange(tab)}
        >
          {activeTab === tab && (
            <motion.span
              layoutId="store-admin-tab-indicator"
              className="absolute inset-0 rounded-lg bg-background shadow-sm"
              transition={
                reduceMotion
                  ? { duration: 0 }
                  : { type: "spring", bounce: 0.15, duration: 0.3 }
              }
            />
          )}
          <span className="relative z-10">{t(`store.admin.tabs.${tab}`)}</span>
        </button>
      ))}
    </div>
  );
}
