import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Film, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { studioApi, type StudioProject, type StudioTemplate } from "@/lib/studio-api";

export function StudioPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const projects = useSWR("studio-projects", studioApi.listProjects);
  const templates = useSWR("studio-templates", studioApi.listTemplates);
  const [creating, setCreating] = useState(false);

  const createProject = async (title: string, templateId?: string) => {
    setCreating(true);
    try {
      const { project } = await studioApi.createProject({ title, template_id: templateId });
      navigate(`/dashboard/studio/p/${project.id}`);
    } catch (error) {
      toast.error(String(error));
    } finally {
      setCreating(false);
    }
  };

  return (
    <PageWrapper className="min-h-0 flex-1">
      <PageHeader
        title={t("studio.title")}
        description={t("studio.description")}
      />
      <div className="flex flex-wrap items-center gap-2">
        <NewProjectButton
          creating={creating}
          templates={templates.data?.templates ?? []}
          onCreate={createProject}
        />
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {projects.isLoading &&
          Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-32 rounded-xl" />
          ))}
        {projects.data?.projects.map((project) => (
          <ProjectCard
            key={project.id}
            project={project}
            onDeleted={() => projects.mutate()}
          />
        ))}
        {projects.data?.projects.length === 0 && !projects.isLoading && (
          <Card className="sm:col-span-2 lg:col-span-3">
            <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
              <Film className="size-8 text-muted-foreground" />
              <p className="text-sm text-muted-foreground">{t("studio.empty")}</p>
            </CardContent>
          </Card>
        )}
      </div>
    </PageWrapper>
  );
}

function NewProjectButton({
  creating,
  templates,
  onCreate,
}: {
  creating: boolean;
  templates: StudioTemplate[];
  onCreate: (title: string, templateId?: string) => void;
}) {
  const { t } = useTranslation();
  const [title, setTitle] = useState("");
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button size="sm">
          <Plus className="size-4" />
          {t("studio.newProject")}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("studio.newProject")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <Input
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            placeholder={t("studio.projectTitle")}
          />
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              disabled={creating || !title.trim()}
              onClick={() => onCreate(title.trim())}
            >
              {t("studio.blankCanvas")}
            </Button>
            {templates.map((template) => (
              <Button
                key={template.id}
                size="sm"
                variant="outline"
                disabled={creating || !title.trim()}
                onClick={() => onCreate(title.trim(), template.id)}
              >
                {template.name}
              </Button>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ProjectCard({
  project,
  onDeleted,
}: {
  project: StudioProject;
  onDeleted: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Card className="group relative">
      <CardContent className="space-y-2 p-4">
        <Link
          to={`/dashboard/studio/p/${project.id}`}
          className="block space-y-1 pr-8"
        >
          <p className="truncate font-medium">{project.title}</p>
          <p className="text-xs text-muted-foreground">
            {t("studio.updatedAt", { time: new Date(project.updated_at).toLocaleString() })}
          </p>
          <p className="text-xs text-muted-foreground">
            {t("studio.versionLabel", { version: project.version })}
          </p>
        </Link>
        <Button
          variant="ghost"
          size="icon"
          className="absolute right-2 top-2 opacity-0 transition-opacity group-hover:opacity-100"
          onClick={async () => {
            try {
              await studioApi.deleteProject(project.id);
              onDeleted();
            } catch (error) {
              toast.error(String(error));
            }
          }}
        >
          <Trash2 className="size-4 text-muted-foreground" />
          <span className="sr-only">{t("common.delete")}</span>
        </Button>
      </CardContent>
    </Card>
  );
}
