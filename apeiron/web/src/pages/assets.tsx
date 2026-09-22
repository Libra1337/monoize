import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { FolderOpen, Trash2 } from "lucide-react";
import { api, assetContentUrl, type Asset } from "@/lib/api";
import { PageWrapper } from "@/components/ui/motion";
import { EmptyState, PageHeader } from "@/components/ui/page";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { formatBytes, formatTime } from "@/lib/format";

const FILTERS = [
  { key: "", labelKey: "assets.all" },
  { key: "image", labelKey: "assets.images" },
  { key: "video", labelKey: "assets.videos" },
  { key: "audio", labelKey: "assets.audio" },
] as const;

function AssetPreview({ asset }: { asset: Asset }) {
  if (asset.kind === "image") {
    return (
      <img
        src={assetContentUrl(asset.id)}
        alt=""
        loading="lazy"
        className="aspect-video w-full rounded-md border bg-muted object-cover"
      />
    );
  }
  if (asset.kind === "video") {
    return (
      <video
        src={assetContentUrl(asset.id)}
        controls
        preload="metadata"
        className="aspect-video w-full rounded-md border bg-muted"
      />
    );
  }
  if (asset.kind === "audio") {
    return <audio src={assetContentUrl(asset.id)} controls className="w-full" />;
  }
  return (
    <a
      href={assetContentUrl(asset.id)}
      target="_blank"
      rel="noreferrer"
      className="flex aspect-video w-full items-center justify-center rounded-md border bg-muted font-mono text-xs text-muted-foreground"
    >
      .srt
    </a>
  );
}

export function AssetsPage() {
  const { t } = useTranslation();
  const [kind, setKind] = useState<string>("");
  const { data, mutate, isLoading } = useSWR(["assets", kind] as const, ([, filter]) =>
    api.get<{ assets: Asset[] }>(`/assets${filter ? `?kind=${filter}` : ""}`),
  );
  const assets = data?.assets ?? [];

  return (
    <PageWrapper className="gap-6">
      <PageHeader title={t("assets.title")} description={t("assets.description")} />
      <div className="flex flex-wrap gap-1">
        {FILTERS.map((filter) => (
          <button
            key={filter.key}
            onClick={() => setKind(filter.key)}
            className={cn(
              "rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
              kind === filter.key
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
            )}
          >
            {t(filter.labelKey)}
          </button>
        ))}
      </div>
      {isLoading ? (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {[0, 1, 2, 3].map((index) => (
            <Skeleton key={index} className="h-48" />
          ))}
        </div>
      ) : assets.length === 0 ? (
        <EmptyState
          icon={FolderOpen}
          title={t("assets.empty")}
          description={t("assets.emptyDescription")}
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {assets.map((asset) => (
            <div key={asset.id} className="group space-y-2">
              <AssetPreview asset={asset} />
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span className="font-mono">
                  {asset.kind} · {formatBytes(asset.bytes)}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6 opacity-0 transition-opacity group-hover:opacity-100"
                  aria-label={t("common.delete")}
                  onClick={async () => {
                    try {
                      await api.delete(`/assets/${asset.id}`);
                      void mutate();
                    } catch (error) {
                      toast.error(String(error));
                    }
                  }}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
              <div className="text-xs text-muted-foreground/70">{formatTime(asset.created_at)}</div>
            </div>
          ))}
        </div>
      )}
    </PageWrapper>
  );
}
