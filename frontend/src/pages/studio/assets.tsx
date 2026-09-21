import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Film, Image as ImageIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { studioApi } from "@/lib/studio-api";

export function StudioAssetsPage() {
  const { t } = useTranslation();
  const [kind, setKind] = useState<"image" | "video" | undefined>();
  const assets = useSWR(["studio-assets", kind ?? "all"], () => studioApi.listAssets(kind));

  return (
    <PageWrapper className="min-h-0 flex-1">
      <PageHeader
        title={t("studio.assets.title")} description={t("studio.assets.description")} />
      <div className="flex items-center gap-2">
        <Button size="sm" variant={kind === undefined ? "default" : "outline"} onClick={() => setKind(undefined)}>
          {t("studio.assets.all")}
        </Button>
        <Button size="sm" variant={kind === "image" ? "default" : "outline"} onClick={() => setKind("image")}>
          <ImageIcon className="size-4" />
          {t("studio.assets.images")}
        </Button>
        <Button size="sm" variant={kind === "video" ? "default" : "outline"} onClick={() => setKind("video")}>
          <Film className="size-4" />
          {t("studio.assets.videos")}
        </Button>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {assets.isLoading && Array.from({ length: 4 }).map((_, i) => <Skeleton key={i} className="aspect-video rounded-xl" />)}
        {assets.data?.assets.map((asset) => (
          <Card key={asset.id}>
            <CardContent className="space-y-2 p-3">
              <div className="flex aspect-video items-center justify-center overflow-hidden rounded-md bg-muted">
                {asset.kind === "image" ? (
                  <img src={studioApi.assetUrl(asset.id)} alt={asset.id} className="size-full object-cover" />
                ) : (
                  <video src={studioApi.assetUrl(asset.id)} controls className="size-full object-cover" />
                )}
              </div>
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span>{new Date(asset.created_at).toLocaleDateString()}</span>
                <span>{asset.upstream_kind}</span>
              </div>
            </CardContent>
          </Card>
        ))}
        {assets.data?.assets.length === 0 && !assets.isLoading && (
          <p className="col-span-full py-10 text-center text-sm text-muted-foreground">
            {t("studio.assets.empty")}
          </p>
        )}
      </div>
    </PageWrapper>
  );
}
