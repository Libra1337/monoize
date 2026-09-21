import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { studioApi } from "@/lib/studio-api";
import { StudioCanvasPage } from "./canvas";
import { StudioAssetsPage } from "./assets";

export function StudioProjectPage() {
  const { id = "", "*": splat } = useParams();
  const { t } = useTranslation();
  const project = useSWR(id ? ["studio-project", id] : null, () => studioApi.getProject(id));

  if (splat === "assets") {
    return <StudioAssetsPage />;
  }

  if (project.isLoading) {
    return (
      <PageWrapper className="min-h-0 flex-1">
        <PageHeader
        title={t("studio.title")} description="" />
        <Skeleton className="h-[60vh] rounded-xl" />
      </PageWrapper>
    );
  }

  if (project.error || !project.data?.project) {
    return (
      <PageWrapper className="min-h-0 flex-1">
        <PageHeader
        title={t("studio.title")} description="" />
        <p className="text-sm text-muted-foreground">{t("studio.projectMissing")}</p>
      </PageWrapper>
    );
  }

  return (
    <StudioCanvasPage
      project={project.data.project}
      onProjectChanged={() => project.mutate()}
    />
  );
}
