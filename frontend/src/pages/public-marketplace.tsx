import { useDeferredValue, useRef, useState } from "react";
import useSWR from "swr";
import { useTranslation } from "react-i18next";
import { Search, Store } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  marketplaceRequest,
  type MarketplaceItem,
  type MarketplaceOffersResponse,
  type MarketplaceRateRange,
  type MarketplaceResponse,
} from "@/lib/marketplace-api";
import { resolveMarketplacePages } from "@/lib/marketplace-pages";
import { formatCoinDecimalUsd, formatCoinPerMillionUsd } from "@/lib/store-money";

function rangeLabel(range: MarketplaceRateRange | null): string {
  if (!range) return "—";
  const format = (value: string) => range.unit.toLowerCase() === "token"
    ? formatCoinPerMillionUsd(value)
    : formatCoinDecimalUsd(value, range.unit);
  const value = format(range.min);
  if (range.min === range.max) return value;
  return `${value.replace(/ \/ .*$/, "")}–${format(range.max)}`;
}

export function PublicMarketplacePage() {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search.trim());
  const [selected, setSelected] = useState<MarketplaceItem | null>(null);
  const [pages, setPages] = useState<MarketplaceResponse[]>([]);
  const [loadCursor, setLoadCursor] = useState<string | null>(null);
  const dialogHeadingRef = useRef<HTMLHeadingElement>(null);
  const query = new URLSearchParams({ limit: "50" });
  if (deferredSearch) query.set("q", deferredSearch);
  const listKey = `/api/public/marketplace?${query}`;
  const { data, error, isLoading } = useSWR<MarketplaceResponse>(listKey, marketplaceRequest, { keepPreviousData: true, onSuccess: (page) => { setPages([page]); setLoadCursor(null); } });
  const loadKey = loadCursor ? `/api/public/marketplace?${new URLSearchParams({ limit: "50", q: deferredSearch, cursor: loadCursor })}` : null;
  const { error: loadError, isLoading: isLoadingMore } = useSWR<MarketplaceResponse>(loadKey, marketplaceRequest, { onSuccess: (page) => { setPages((current) => current.some((candidate) => candidate.revision === page.revision && candidate.items[0]?.model === page.items[0]?.model) ? current : [...current, page].slice(-3)); setLoadCursor(null); } });
  const offersKey = selected ? `/api/public/marketplace/offers?group=${encodeURIComponent(selected.public_group_name)}&model=${encodeURIComponent(selected.model)}&limit=50` : null;
  const { data: offers, error: offersError, isLoading: offersLoading } = useSWR<MarketplaceOffersResponse>(offersKey, marketplaceRequest);

  const resolvedPages = resolveMarketplacePages(pages, data);
  const items = resolvedPages.flatMap((page) => page.items);
  const nextCursor = resolvedPages.at(-1)?.next_cursor ?? null;

  return (
    <div className="mx-auto max-w-7xl px-4 py-12 sm:px-6 lg:px-8">
      <header className="max-w-3xl"><p className="font-mono text-sm text-primary">MODEL MARKETPLACE</p><h1 className="mt-3 font-display text-4xl font-semibold sm:text-5xl">{t("publicSite.marketplace.title")}</h1><p className="mt-4 text-lg leading-8 text-muted-foreground">{t("publicSite.marketplace.description")}</p></header>
      <div className="relative mt-8 max-w-xl"><Search className="pointer-events-none absolute left-3 top-1/2 size-5 -translate-y-1/2 text-muted-foreground" /><Input value={search} onChange={(event) => setSearch(event.target.value)} className="h-12 pl-10 text-base" placeholder={t("publicSite.marketplace.search")} aria-label={t("publicSite.marketplace.search")} /></div>
      {isLoading && !data ? <div className="mt-10 space-y-5"><Skeleton className="h-8 w-40" /><div className="grid gap-4 md:grid-cols-2"><Skeleton className="h-48" /><Skeleton className="h-48" /></div></div> : error ? <Card className="mt-10 p-6 text-destructive">{t("publicSite.marketplace.loadError")}</Card> : items.length === 0 ? <Card className="mt-10 flex flex-col items-center p-10 text-center"><Store className="size-10 text-muted-foreground" /><h2 className="mt-4 text-lg font-semibold">{t("publicSite.marketplace.empty")}</h2></Card> : (
        <div className="mt-10 space-y-5">{items.map((item, index) => { const showGroup = index === 0 || item.public_group_name !== items[index - 1]?.public_group_name; return <div key={`${item.public_group_name}:${item.model}`}>{showGroup && <h2 className="mb-4 mt-9 font-display text-2xl font-semibold first:mt-0">{item.public_group_name}</h2>}<button type="button" onClick={() => setSelected(item)} className="block min-h-11 w-full rounded-lg text-left focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring"><Card className="grid gap-5 p-5 transition-colors duration-200 hover:border-primary/45 md:grid-cols-[minmax(0,1fr)_auto] md:items-center"><div className="min-w-0"><h3 className="truncate font-mono text-base font-semibold">{item.model}</h3><div className="mt-3 flex flex-wrap gap-2">{item.capabilities.map((capability) => <Badge key={capability} variant="outline">{capability}</Badge>)}<Badge>{t("publicSite.marketplace.offerCount", { count: item.offer_count })}</Badge></div></div><dl className="grid gap-3 text-sm sm:grid-cols-2"><div><dt className="text-muted-foreground">{t("publicSite.marketplace.input")}</dt><dd className="mt-1 font-mono text-xs">{rangeLabel(item.input_rate_range)}</dd></div><div><dt className="text-muted-foreground">{t("publicSite.marketplace.output")}</dt><dd className="mt-1 font-mono text-xs">{rangeLabel(item.output_rate_range)}</dd></div></dl></Card></button></div>})}</div>
      )}

      {(nextCursor || isLoadingMore || loadError) && <div className="mt-8 flex flex-col items-center gap-3"><button type="button" disabled={isLoadingMore} onClick={() => nextCursor && setLoadCursor(nextCursor)} className="min-h-11 rounded-md border px-5 text-sm font-medium transition-colors hover:bg-muted disabled:cursor-wait disabled:opacity-60">{isLoadingMore ? t("publicSite.marketplace.loadingMore") : t("publicSite.marketplace.loadMore")}</button>{loadError && <p className="text-sm text-destructive">{t("publicSite.marketplace.loadMoreError")}</p>}</div>}

      <Dialog open={selected !== null} onOpenChange={(open) => { if (!open) setSelected(null); }}>
        <DialogContent
          className="max-h-[85dvh] max-w-3xl"
          closeLabel={t("common.close")}
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            dialogHeadingRef.current?.focus();
          }}
        >
          <DialogHeader><DialogTitle ref={dialogHeadingRef} tabIndex={-1} className="pr-8 font-mono text-xl outline-none">{selected?.model}</DialogTitle><DialogDescription>{selected?.public_group_name} · {t("publicSite.marketplace.detailDescription")}</DialogDescription></DialogHeader>
          {offersLoading ? <div className="space-y-3"><Skeleton className="h-28" /><Skeleton className="h-28" /></div> : offersError ? <Card className="p-4 text-destructive">{t("publicSite.marketplace.offersError")}</Card> : <div className="space-y-4">{offers?.offers.map((offer) => <Card key={`${offer.public_provider_name}:${offer.public_channel_name}`} className="p-5"><div className="flex flex-wrap items-start justify-between gap-3"><div><h3 className="font-semibold">{offer.public_provider_name}</h3><p className="mt-1 text-sm text-muted-foreground">{offer.public_channel_name}</p></div><Badge variant="outline">{offer.api_type}</Badge></div><dl className="mt-4 divide-y border-y">{offer.rates.map((rate, index) => <div key={`${rate.usage_class}:${index}`} className="grid gap-1 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto]"><dt>{rate.usage_class}{rate.context_tier ? ` · ${rate.context_tier}` : ""}{rate.service_tier ? ` · ${rate.service_tier}` : ""}{rate.modality ? ` · ${rate.modality}` : ""}{rate.cache_ttl ? ` · ${rate.cache_ttl}` : ""}</dt><dd className="font-mono text-xs">{rate.display_rate_nano_usd} nano-USD / {rate.unit}</dd></div>)}</dl></Card>)}</div>}
          <p className="text-xs leading-5 text-muted-foreground">{t("publicSite.marketplace.billingNote")}</p>
        </DialogContent>
      </Dialog>
    </div>
  );
}
