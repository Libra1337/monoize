import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import useSWR from "swr";
import { toast } from "sonner";
import { Layers, Upload } from "lucide-react";
import { api, type Template } from "@/lib/api";
import { PageWrapper } from "@/components/ui/motion";
import { PageHeader } from "@/components/ui/page";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Textarea } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function TemplatesPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { data, isLoading, mutate } = useSWR("templates", () =>
    api.get<{ templates: Template[] }>("/templates"),
  );
  const [importOpen, setImportOpen] = useState(false);
  const [importName, setImportName] = useState("");
  const [importJson, setImportJson] = useState("");

  async function use(template: Template) {
    try {
      const created = await api.post<{ id: string }>("/projects", {
        title: template.name,
        template_id: template.id,
      });
      navigate(`/canvas/${created.id}`);
    } catch (error) {
      toast.error(String(error));
    }
  }

  async function importGraph() {
    try {
      const parsed = JSON.parse(importJson);
      await api.post("/admin/templates", {
        name: importName || "Imported workflow",
        description: "Imported from graph JSON",
        graph: parsed,
        enabled: true,
      });
      toast.success(t("templates.import"));
      setImportOpen(false);
      setImportJson("");
      setImportName("");
      void mutate();
    } catch {
      toast.error(t("templates.invalid"));
    }
  }

  return (
    <PageWrapper className="gap-6">
      <PageHeader
        title={t("templates.title")}
        description={t("templates.description")}
        actions={
          <Button variant="outline" onClick={() => setImportOpen(true)}>
            <Upload />
            {t("templates.import")}
          </Button>
        }
      />
      {isLoading ? (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-32" />
          ))}
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {(data?.templates ?? []).map((template) => (
            <Card key={template.id}>
              <CardHeader>
                <div className="flex items-start justify-between gap-2">
                  <span className="flex size-8 items-center justify-center rounded-md bg-muted">
                    <Layers className="h-4 w-4 text-muted-foreground" />
                  </span>
                  <span className="rounded-md border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                    {template.source}
                  </span>
                </div>
                <CardTitle className="mt-2">{template.name}</CardTitle>
                <CardDescription className="line-clamp-3">{template.description}</CardDescription>
                <div className="pt-2 font-mono text-xs text-muted-foreground">
                  {template.graph?.nodes?.length ?? 0} nodes
                </div>
                <Button variant="primary" size="sm" className="mt-3 w-full" onClick={() => use(template)}>
                  {t("templates.use")}
                </Button>
              </CardHeader>
            </Card>
          ))}
        </div>
      )}

      <Dialog open={importOpen} onOpenChange={setImportOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("templates.importTitle")}</DialogTitle>
            <DialogDescription>{t("templates.description")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Textarea
              rows={8}
              className="font-mono text-xs"
              placeholder={t("templates.importPlaceholder")}
              value={importJson}
              onChange={(event) => setImportJson(event.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setImportOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" onClick={importGraph} disabled={!importJson.trim()}>
              {t("templates.import")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}
