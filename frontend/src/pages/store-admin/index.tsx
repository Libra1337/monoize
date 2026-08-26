import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/ui/page-header";

export function StoreAdminPage() {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title={t("store.admin.title")}
        description={t("store.admin.description")}
      />
    </div>
  );
}
