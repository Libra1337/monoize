import { useTranslation } from "react-i18next";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";

export function DashboardApiDocsPage() {
  const { t } = useTranslation();
  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6">
      <PageHeader title={t("publicSite.docs.title")} description={t("publicSite.docs.description")} />
    </PageWrapper>
  );
}
