import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/ui/page-header";

export function OrdersPage() {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title={t("store.orders.title")}
        description={t("store.orders.description")}
      />
    </div>
  );
}
