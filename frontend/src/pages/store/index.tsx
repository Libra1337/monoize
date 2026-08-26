import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/ui/page-header";

export function StorePage() {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title={t("store.title")}
        description={t("store.description")}
      />
    </div>
  );
}
