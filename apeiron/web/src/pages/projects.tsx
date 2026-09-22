import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import useSWR from "swr";
import { toast } from "sonner";
import { Film, Plus, Trash2 } from "lucide-react";
import { api, type ProjectSummary, type Template } from "@/lib/api";
import { EmptyState } from "@/components/ui/page";
import { BalanceChip, TopUpButton, WorkHeader } from "@/components/app-shell";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input, Select } from "@/components/ui/input";
import { Label } from "@/components/ui/controls";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/badge";
import { formatTime } from "@/lib/format";

export function ProjectsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { data, mutate, isLoading } = useSWR("projects", () =>
    api.get<{ projects: ProjectSummary[] }>("/projects"),
  );
  const { data: templates } = useSWR("templates", () =>
    api.get<{ templates: Template[] }>("/templates"),
  );
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [templateId, setTemplateId] = useState("blank-canvas");
  const [creating, setCreating] = useState(false);

  async function create() {
    setCreating(true);
    try {
      const created = await api.post<{ id: string }>("/projects", {
        title,
        template_id: templateId === "blank" ? undefined : templateId,
      });
      setOpen(false);
      setTitle("");
      navigate(`/canvas/${created.id}`);
    } catch (error) {
      toast.error(String(error));
    } finally {
      setCreating(false);
    }
  }

  const projects = data?.projects ?? [];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkHeader
        title={t("projects.title")}
        description={t("projects.description")}
        actions={
          <>
            <BalanceChip />
            <TopUpButton />
            <Button variant="primary" size="sm" onClick={() => setOpen(true)}>
              <Plus />
              {t("projects.new")}
            </Button>
          </>
        }
      />
      <div className="flex-1 overflow-y-auto p-6 pb-24 md:pb-6">
      {isLoading ? (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-28" />
          ))}
        </div>
      ) : projects.length === 0 ? (
        <EmptyState
          icon={Film}
          title={t("projects.empty")}
          description={t("projects.emptyDescription")}
          action={
            <Button variant="primary" size="sm" onClick={() => setOpen(true)}>
              <Plus />
              {t("projects.new")}
            </Button>
          }
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {projects.map((project) => (
            <Card
              key={project.id}
              className="group cursor-pointer p-4 transition-colors hover:bg-muted/30"
              onClick={() => navigate(`/canvas/${project.id}`)}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium">{project.title}</div>
                  <div className="mt-1 font-mono text-xs text-muted-foreground">
                    v{project.version} · {formatTime(project.updated_at)}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  className="opacity-0 transition-opacity group-hover:opacity-100"
                  aria-label={t("common.delete")}
                  onClick={async (event) => {
                    event.stopPropagation();
                    try {
                      await api.delete(`/projects/${project.id}`);
                      void mutate();
                    } catch (error) {
                      toast.error(String(error));
                    }
                  }}
                >
                  <Trash2 />
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}

      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("projects.new")}</DialogTitle>
            <DialogDescription>{t("projects.description")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="project-title">{t("projects.namePlaceholder")}</Label>
              <Input
                id="project-title"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                placeholder={t("projects.namePlaceholder")}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="project-template">{t("projects.fromTemplate")}</Label>
              <Select
                id="project-template"
                value={templateId}
                onChange={(event) => setTemplateId(event.target.value)}
              >
                <option value="blank">{t("projects.blank")}</option>
                {(templates?.templates ?? []).map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </Select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" onClick={create} disabled={creating || !title.trim()}>
              {t("common.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
