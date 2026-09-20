import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CloudDownload,
  Database,
  ListFilter,
  Pencil,
  Pin,
  Plus,
  RefreshCw,
  Settings2,
  SlidersHorizontal,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import { ModelBadge } from "@/components/ModelBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageWrapper } from "@/components/ui/motion";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { TableVirtuoso } from "react-virtuoso";
import { VirtualTableCell, VirtualTableHeaderCell } from "@/components/ui/data-table-shell";
import {
  useBillingRateProfiles,
  useBillingRatesForProfile,
  usePricingProfilePatterns,
  syncBillingRatesCatalog,
  syncModelMetadata,
} from "@/lib/swr";
import type { BillingRateRecord, ModelMetadataRecord } from "@/lib/api";
import { cn } from "@/lib/utils";
import { MatchPatternsDialog } from "./match-patterns-dialog";
import { ManageProfilesDialog } from "./manage-profiles-dialog";
import { SyncReportDialog, type SyncReportData } from "./sync-report-dialog";
import { PriceEditorPanel, PriceEditorSkeleton } from "./price-editor-panel";
import { summarizeModelPrices } from "./price-utils";

/**
 * UI4a: the single-page master-detail workbench. The master list shows one row
 * per model of the selected pricing profile with a compressed price summary;
 * the sticky right panel is the per-model price editor.
 */
export function ModelWorkbench({
  metadata,
  metadataLoading,
  onEditMetadata,
  onCreateMetadata,
  onDeleteMetadata,
}: {
  metadata: ModelMetadataRecord[];
  metadataLoading: boolean;
  onEditMetadata: (record: ModelMetadataRecord) => void;
  onCreateMetadata: () => void;
  onDeleteMetadata: (modelId: string) => void;
}) {
  const { t } = useTranslation();
  const {
    data: profiles = [],
    isLoading: profilesLoading,
    mutate: revalidateProfiles,
  } = useBillingRateProfiles();
  const {
    data: patterns = [],
  } = usePricingProfilePatterns();

  const [selectedProfile, setSelectedProfile] = useState<string>("");
  const [search, setSearch] = useState("");
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [patternsOpen, setPatternsOpen] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const [syncing, setSyncing] = useState<"models_dev" | "catalog" | null>(null);
  const [syncReport, setSyncReport] = useState<SyncReportData | null>(null);
  const [syncReportOpen, setSyncReportOpen] = useState(false);

  // Default to the first profile (openai preferred) once summaries load.
  useEffect(() => {
    if (profiles.length === 0 || (selectedProfile && profiles.some((p) => p.pricing_profile === selectedProfile))) {
      return;
    }
    const preferred = profiles.find((p) => p.pricing_profile === "openai") ?? profiles[0];
    setSelectedProfile(preferred?.pricing_profile ?? "");
    setSelectedModel(null);
  }, [profiles, selectedProfile]);

  const {
    data: rates = [],
    isLoading: ratesLoading,
    mutate: revalidateRates,
  } = useBillingRatesForProfile(selectedProfile || null);

  const ratesByModel = useMemo(() => {
    const grouped = new Map<string, BillingRateRecord[]>();
    for (const rate of rates) {
      if (!rate.model_pattern) continue;
      const list = grouped.get(rate.model_pattern) ?? [];
      list.push(rate);
      grouped.set(rate.model_pattern, list);
    }
    return grouped;
  }, [rates]);

  const rows = useMemo(() => {
    const models = new Set<string>(ratesByModel.keys());
    // Registry-only models of this provider show with an empty price set.
    for (const record of metadata) {
      if (record.models_dev_provider === selectedProfile) models.add(record.model_id);
    }
    return [...models]
      .filter((model) => model.toLowerCase().includes(search.trim().toLowerCase()))
      .sort((a, b) => a.localeCompare(b))
      .map((model) => ({
        model,
        rates: ratesByModel.get(model) ?? [],
        metadata: metadata.find((m) => m.model_id === model),
      }));
  }, [ratesByModel, metadata, selectedProfile, search]);

  const selectedModelRates = useMemo(
    () => (selectedModel ? (ratesByModel.get(selectedModel) ?? []) : []),
    [ratesByModel, selectedModel]
  );

  const runSync = async (kind: "models_dev" | "catalog") => {
    setSyncing(kind);
    try {
      if (kind === "models_dev") {
        const result = await syncModelMetadata((error) =>
          toast.error(t("modelMetadata.syncFailed"), { description: error.message })
        );
        setSyncReport({
          kind,
          upserted: result.upserted,
          skipped: result.skipped,
          deleted: result.deleted,
          retainedManualModels: result.retained_manual_models ?? [],
        });
      } else {
        const result = await syncBillingRatesCatalog();
        setSyncReport({
          kind,
          upserted: result.upserted,
          skipped: result.skipped,
          deleted: result.deleted,
        });
      }
      setSyncReportOpen(true);
      await revalidateRates();
      await revalidateProfiles();
    } catch {
      return;
    } finally {
      setSyncing(null);
    }
  };

  const handleProfileDeleted = (deleted: string) => {
    if (selectedProfile === deleted) {
      setSelectedProfile("");
      setSelectedModel(null);
    }
  };

  return (
    <PageWrapper className="space-y-4">
      {/* Toolbar: profile selector, search, sync dropdown, dialogs, create */}
      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={selectedProfile || undefined}
          onValueChange={(value) => {
            setSelectedProfile(value);
            setSelectedModel(null);
          }}
        >
          <SelectTrigger className="w-56 font-mono text-xs" aria-label={t("modelMetadata.profile")}>
            <SelectValue
              placeholder={profilesLoading ? t("common.loading") : t("modelMetadata.profile")}
            />
          </SelectTrigger>
          <SelectContent>
            {profiles.map((profile) => (
              <SelectItem key={profile.pricing_profile} value={profile.pricing_profile}>
                <span className="font-mono">{profile.pricing_profile}</span>
                <span className="ml-1 text-muted-foreground">
                  {t("modelMetadata.profiles.modelCount", { count: profile.model_count })}
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={t("modelMetadata.searchPlaceholder")}
          className="max-w-64"
        />

        <div className="ml-auto flex items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" disabled={syncing !== null}>
                <RefreshCw
                  data-icon
                  className={syncing !== null ? "animate-spin" : undefined}
                />
                {syncing !== null
                  ? t("modelMetadata.syncing")
                  : t("modelMetadata.sync.button")}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => void runSync("models_dev")}>
                <CloudDownload data-icon />
                {t("modelMetadata.syncModelsDev")}
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => void runSync("catalog")}>
                <Database data-icon />
                {t("modelMetadata.syncCatalog")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>

          <Button variant="outline" onClick={() => setPatternsOpen(true)}>
            <ListFilter data-icon />
            {t("modelMetadata.patterns.button")}
          </Button>

          <Button variant="outline" onClick={() => setManageOpen(true)}>
            <Settings2 data-icon />
            {t("modelMetadata.profiles.manageButton")}
          </Button>

          <Button onClick={onCreateMetadata}>
            <Plus data-icon />
            {t("modelMetadata.addModel")}
          </Button>
        </div>
      </div>

      {/* Master-detail body */}
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(360px,2fr)_minmax(0,3fr)]">
        {/* Master list */}
        <div className="min-w-0 lg:col-span-2">
          {profilesLoading || (metadataLoading && metadata.length === 0) ? (
            <div className="space-y-2 rounded-xl border bg-card p-4">
              {Array.from({ length: 8 }).map((_, index) => (
                <Skeleton key={index} className="h-10 w-full" />
              ))}
            </div>
          ) : rows.length === 0 ? (
            <EmptyState
              className="rounded-xl border py-16"
              icon={<Database className="h-12 w-12" />}
              title={t("modelMetadata.noModels")}
              description={t("modelMetadata.noModelsDesc")}
            />
          ) : (
            <div className="overflow-hidden rounded-xl border bg-card">
              <TableVirtuoso
                style={{ height: "calc(100dvh - 300px)", minHeight: 400 }}
                data={rows}
                components={{
                  Table: (props) => (
                    <table {...props} className="w-full caption-bottom text-sm" />
                  ),
                  TableHead: (props) => <thead {...props} className="[&_tr]:border-b" />,
                  TableRow: (props) => (
                    <tr
                      {...props}
                      className={cn(
                        "cursor-pointer border-b transition-colors hover:bg-muted/50",
                        selectedModel != null && "data-[selected=true]:bg-accent/40"
                      )}
                    />
                  ),
                  TableBody: (props) => (
                    <tbody {...props} className="[&_tr:last-child]:border-0" />
                  ),
                }}
                fixedHeaderContent={() => (
                  <tr className="border-b bg-background">
                    <VirtualTableHeaderCell className="min-w-[200px]">
                      {t("modelMetadata.modelId")}
                    </VirtualTableHeaderCell>
                    <VirtualTableHeaderCell>
                      {t("modelMetadata.priceSummary")}
                    </VirtualTableHeaderCell>
                    <VirtualTableHeaderCell className="w-[90px]">
                      {t("common.actions")}
                    </VirtualTableHeaderCell>
                  </tr>
                )}
                itemContent={(_index, row) => {
                  const summary = summarizeModelPrices(row.rates);
                  const isSelected = selectedModel === row.model;
                  return (
                    <>
                      <VirtualTableCell
                        onClick={() => setSelectedModel(row.model)}
                        data-selected={isSelected}
                      >
                        <ModelBadge
                          model={row.model}
                          provider={row.metadata?.models_dev_provider}
                          showDetails={false}
                        />
                      </VirtualTableCell>
                      <VirtualTableCell
                        onClick={() => setSelectedModel(row.model)}
                        data-selected={isSelected}
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-1.5">
                            <span className="font-mono text-xs">
                              {summary.primary ?? "—"}
                            </span>
                            {summary.isManual && (
                              <Badge variant="default" className="px-1 py-0 text-[10px]">
                                {t("modelMetadata.manualShort")}
                              </Badge>
                            )}
                            {summary.hasPeak && (
                              <Pin className="size-3 text-warning" aria-label={t("modelMetadata.editor.peak")} />
                            )}
                          </div>
                          {summary.detail && (
                            <p className="truncate text-xs text-muted-foreground">
                              {summary.detail}
                            </p>
                          )}
                        </div>
                      </VirtualTableCell>
                      <VirtualTableCell>
                        <div className="flex items-center gap-1">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-11 touch-manipulation sm:size-9"
                            aria-label={t("common.edit")}
                            onClick={(event) => {
                              event.stopPropagation();
                              setSelectedModel(row.model);
                            }}
                          >
                            <Pencil className="h-4 w-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-11 touch-manipulation text-destructive hover:text-destructive sm:size-9"
                            aria-label={t("common.delete")}
                            onClick={(event) => {
                              event.stopPropagation();
                              onDeleteMetadata(row.model);
                            }}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </VirtualTableCell>
                    </>
                  );
                }}
              />
            </div>
          )}
        </div>

        {/* Sticky price editor */}
        <aside className="lg:sticky lg:top-4 lg:h-[calc(100dvh-9rem)]">
          <div className="h-full overflow-hidden rounded-xl border bg-card p-4">
            {ratesLoading && selectedModel ? (
              <PriceEditorSkeleton />
            ) : (
              <PriceEditorPanel
                profile={selectedProfile}
                model={selectedModel}
                modelRates={selectedModelRates}
                onRatesChanged={() => {
                  void revalidateRates();
                  void revalidateProfiles();
                }}
              />
            )}
          </div>
        </aside>
      </div>

      <MatchPatternsDialog
        open={patternsOpen}
        onOpenChange={setPatternsOpen}
        patterns={patterns}
      />

      <ManageProfilesDialog
        open={manageOpen}
        onOpenChange={setManageOpen}
        profiles={profiles}
        onProfileDeleted={handleProfileDeleted}
      />

      <SyncReportDialog
        open={syncReportOpen}
        onOpenChange={setSyncReportOpen}
        report={syncReport}
      />
    </PageWrapper>
  );
}
