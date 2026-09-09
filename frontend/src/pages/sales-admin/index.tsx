import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { SalesAdminPanel } from "./sales-admin-panel";

/**
 * Sales administration (SC-UI-6).
 *
 * Its own dashboard page rather than a Store Management tab: agents, commission records, and
 * withdrawals are a distinct workflow from products and payment channels, and nesting it as a
 * fifth tab pushed the page past the viewport.
 */
export function SalesAdminPage() {
  const { t } = useTranslation();
  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6">
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
      >
        <PageHeader
          title={t("salesAdmin.title")}
          description={t("salesAdmin.description")}
        />
      </motion.div>
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.06, ...transitions.normal }}
      >
        <SalesAdminPanel />
      </motion.div>
    </PageWrapper>
  );
}
