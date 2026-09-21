import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Settings2, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { studioApi } from "@/lib/studio-api";

export function StudioAdminPage() {
  const { t } = useTranslation();
  return (
    <PageWrapper className="min-h-0 flex-1">
      <PageHeader
        title={t("studio.admin.title")} description={t("studio.admin.description")} />
      <Tabs defaultValue="runs">
        <TabsList>
          <TabsTrigger value="runs">{t("studio.admin.runs")}</TabsTrigger>
          <TabsTrigger value="templates">{t("studio.admin.templates")}</TabsTrigger>
          <TabsTrigger value="settings">{t("studio.admin.settings")}</TabsTrigger>
        </TabsList>
        <TabsContent value="runs" className="mt-4">
          <RunsPanel />
        </TabsContent>
        <TabsContent value="templates" className="mt-4">
          <TemplatesPanel />
        </TabsContent>
        <TabsContent value="settings" className="mt-4">
          <SettingsPanel />
        </TabsContent>
      </Tabs>
    </PageWrapper>
  );
}

function RunsPanel() {
  const { t } = useTranslation();
  const runs = useSWR("studio-admin-runs", () => studioApi.adminListRuns(100));
  return (
    <Card>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("studio.admin.runId")}</TableHead>
              <TableHead>{t("studio.admin.kind")}</TableHead>
              <TableHead>{t("studio.admin.status")}</TableHead>
              <TableHead>{t("studio.admin.createdAt")}</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {runs.isLoading &&
              Array.from({ length: 3 }).map((_, i) => (
                <TableRow key={i}>
                  <TableCell colSpan={5}>
                    <Skeleton className="h-5 w-full" />
                  </TableCell>
                </TableRow>
              ))}
            {runs.data?.runs.map((run) => (
              <TableRow key={run.id}>
                <TableCell className="font-mono text-xs">{run.id.slice(0, 8)}</TableCell>
                <TableCell>{run.kind}</TableCell>
                <TableCell>
                  <Badge variant="secondary">{run.status}</Badge>
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {new Date(run.created_at).toLocaleString()}
                </TableCell>
                <TableCell>
                  {["queued", "running"].includes(run.status) && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={async () => {
                        try {
                          await studioApi.adminCancelRun(run.id);
                          runs.mutate();
                        } catch (error) {
                          toast.error(String(error));
                        }
                      }}
                    >
                      {t("studio.admin.cancel")}
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

function TemplatesPanel() {
  const { t } = useTranslation();
  const templates = useSWR("studio-admin-templates", studioApi.adminListTemplates);
  const [newId, setNewId] = useState("");
  const [newName, setNewName] = useState("");
  return (
    <div className="space-y-4">
      <Card>
        <CardContent className="flex flex-wrap items-end gap-2 p-4">
          <div className="grid flex-1 gap-2 sm:grid-cols-2">
            <Input value={newId} onChange={(e) => setNewId(e.target.value)} placeholder={t("studio.admin.templateId")} />
            <Input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder={t("studio.admin.templateName")} />
          </div>
          <Button
            size="sm"
            disabled={!newId.trim() || !newName.trim()}
            onClick={async () => {
              try {
                await studioApi.adminCreateTemplate({
                  id: newId.trim(),
                  name: newName.trim(),
                  graph: { nodes: [], edges: [] },
                });
                templates.mutate();
                setNewId("");
                setNewName("");
                toast.success(t("studio.admin.templateCreated"));
              } catch (error) {
                toast.error(String(error));
              }
            }}
          >
            {t("studio.admin.createTemplate")}
          </Button>
        </CardContent>
      </Card>
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("studio.admin.templateId")}</TableHead>
                <TableHead>{t("studio.admin.templateName")}</TableHead>
                <TableHead>{t("studio.admin.source")}</TableHead>
                <TableHead>{t("studio.admin.enabled")}</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {templates.data?.templates.map((template) => (
                <TableRow key={template.id}>
                  <TableCell className="font-mono text-xs">{template.id}</TableCell>
                  <TableCell>{template.name}</TableCell>
                  <TableCell>
                    <Badge variant="outline">{template.source}</Badge>
                  </TableCell>
                  <TableCell>
                    <Switch
                      checked={template.enabled}
                      onCheckedChange={async (checked) => {
                        try {
                          await studioApi.adminUpdateTemplate(template.id, { enabled: checked });
                          templates.mutate();
                        } catch (error) {
                          toast.error(String(error));
                        }
                      }}
                    />
                  </TableCell>
                  <TableCell>
                    {template.source === "custom" && (
                      <Button
                        size="icon"
                        variant="ghost"
                        onClick={async () => {
                          try {
                            await studioApi.adminDeleteTemplate(template.id);
                            templates.mutate();
                          } catch (error) {
                            toast.error(String(error));
                          }
                        }}
                      >
                        <Trash2 className="size-4" />
                        <span className="sr-only">{t("common.delete")}</span>
                      </Button>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}

function SettingsPanel() {
  const { t } = useTranslation();
  const settings = useSWR("studio-admin-settings", studioApi.adminGetSettings);
  const [saving] = useState(false);
  if (settings.isLoading || !settings.data) {
    return <Skeleton className="h-48 rounded-xl" />;
  }
  const current = settings.data.settings;
  return (
    <Card>
      <CardContent className="max-w-xl space-y-4 p-4">
        <div className="flex items-center gap-2 text-sm font-medium">
          <Settings2 className="size-4 text-primary" />
          {t("studio.admin.settings")}
        </div>
        <label className="block space-y-1">
          <span className="text-xs text-muted-foreground">{t("studio.admin.agentModel")}</span>
          <Input
            defaultValue={current.agent_model}
            onBlur={(event) =>
              studioApi
                .adminUpdateSettings({ agent_model: event.target.value })
                .then(() => settings.mutate())
                .catch((error) => toast.error(String(error)))
            }
          />
        </label>
        <label className="block space-y-1">
          <span className="text-xs text-muted-foreground">{t("studio.admin.videoUpstream")}</span>
          <Input
            defaultValue={current.default_video_upstream}
            onBlur={(event) =>
              studioApi
                .adminUpdateSettings({ default_video_upstream: event.target.value })
                .then(() => settings.mutate())
                .catch((error) => toast.error(String(error)))
            }
          />
        </label>
        <label className="block space-y-1">
          <span className="text-xs text-muted-foreground">{t("studio.admin.imageModel")}</span>
          <Input
            defaultValue={current.image_model}
            onBlur={(event) =>
              studioApi
                .adminUpdateSettings({ image_model: event.target.value })
                .then(() => settings.mutate())
                .catch((error) => toast.error(String(error)))
            }
          />
        </label>
        <p className="text-xs text-muted-foreground">{t("studio.admin.settingsHint")}</p>
        {saving && <p className="text-xs text-muted-foreground">{t("common.loading")}</p>}
      </CardContent>
    </Card>
  );
}
