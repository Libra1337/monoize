import { useDeferredValue, useMemo, useRef, useState } from "react";
import useSWR from "swr";
import { useTranslation } from "react-i18next";
import { Boxes, CircleDollarSign, RefreshCw, Search, Store } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import {
  marketplaceRequest,
  type MarketplaceItem,
  type MarketplaceOffersResponse,
  type MarketplaceResponse,
} from "@/lib/marketplace-api";
import {
  formatMarketplaceRate,
  formatMarketplaceRateRange,
} from "@/lib/marketplace-pricing";
import { resolveMarketplacePages } from "@/lib/marketplace-pages";
import { storeApi, type StoreExchangeRate } from "@/lib/store-api";
import { cn } from "@/lib/utils";

const ALL = "__all";
const LIST_LIMIT = "50";
const EXCHANGE_RATE_KEY = "/api/dashboard/store/exchange-rate";

function MarketplaceSkeleton() {
  return (
    <div className="flex flex-col gap-6" aria-hidden="true">
      {[0, 1].map((group) => (
        <div key={group} className="flex flex-col gap-3">
          <Skeleton className="h-7 w-36" />
          <div className="overflow-hidden rounded-lg border">
            {[0, 1, 2].map((row) => (
              <div key={row} className="grid gap-4 border-b p-4 last:border-b-0 md:grid-cols-[minmax(0,1fr)_11rem_11rem_4rem]">
                <div className="flex flex-col gap-2"><Skeleton className="h-5 w-48" /><Skeleton className="h-5 w-28" /></div>
                <Skeleton className="h-9" />
                <Skeleton className="h-9" />
                <Skeleton className="h-7" />
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function CurrencyControl({ unavailable }: { unavailable: boolean }) {
  const { t } = useTranslation();
  const { currency, setCurrency } = useStoreCurrency();
  return (
    <div className="flex min-h-11 rounded-lg border bg-muted/45 p-1" role="radiogroup" aria-label={t("modelMarketplace.currency")}>
      {(["CNY", "USD"] as const).map((item) => (
        <button
          key={item}
          type="button"
          role="radio"
          aria-checked={currency === item}
          disabled={item === "CNY" && unavailable}
          onClick={() => setCurrency(item)}
          className={cn(
            "relative min-h-9 min-w-14 cursor-pointer rounded-md px-3 text-sm font-medium transition-colors duration-200 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45",
            currency === item ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {item}
        </button>
      ))}
    </div>
  );
}

function ModelRow({
  item,
  currencyRate,
  onOpen,
}: {
  item: MarketplaceItem;
  currencyRate: string | null;
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const { currency } = useStoreCurrency();
  const canPrice = currency === "USD" || currencyRate !== null;
  const price = (range: MarketplaceItem["input_rate_range"]) => {
    if (!range || !canPrice) return t("modelMarketplace.unavailable");
    return formatMarketplaceRateRange(range, currency, currencyRate ?? "1");
  };

  return (
    <button
      type="button"
      onClick={onOpen}
      className="grid min-h-20 w-full cursor-pointer gap-4 border-b p-4 text-left transition-colors duration-200 last:border-b-0 hover:bg-muted/45 focus-visible:relative focus-visible:z-10 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-inset focus-visible:ring-ring md:grid-cols-[minmax(0,1fr)_minmax(10rem,0.62fr)_minmax(10rem,0.62fr)_5rem] md:items-center"
    >
      <div className="min-w-0">
        <div className="truncate font-mono text-sm font-semibold">{item.model}</div>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {item.capabilities.map((capability) => <Badge key={capability} variant="outline">{capability}</Badge>)}
        </div>
      </div>
      <div>
        <div className="text-xs text-muted-foreground">{t("modelMarketplace.inputPrice")}</div>
        <div className="mt-1 font-mono text-xs font-medium tabular-nums">{price(item.input_rate_range)}</div>
      </div>
      <div>
        <div className="text-xs text-muted-foreground">{t("modelMarketplace.outputPrice")}</div>
        <div className="mt-1 font-mono text-xs font-medium tabular-nums">{price(item.output_rate_range)}</div>
      </div>
      <Badge className="w-fit justify-self-start md:justify-self-end" variant="secondary">
        {t("modelMarketplace.offerCount", { count: item.offer_count })}
      </Badge>
    </button>
  );
}

export function ModelMarketplacePage() {
  const { t } = useTranslation();
  const { currency } = useStoreCurrency();
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search.trim());
  const [group, setGroup] = useState(ALL);
  const [capability, setCapability] = useState(ALL);
  const [knownGroups, setKnownGroups] = useState<string[]>([]);
  const [selected, setSelected] = useState<MarketplaceItem | null>(null);
  const [pages, setPages] = useState<MarketplaceResponse[]>([]);
  const [loadCursor, setLoadCursor] = useState<string | null>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);

  const query = new URLSearchParams({ limit: LIST_LIMIT });
  if (deferredSearch) query.set("q", deferredSearch);
  if (group !== ALL) query.set("group", group);
  const listKey = `/api/public/marketplace?${query}`;
  const list = useSWR<MarketplaceResponse>(listKey, marketplaceRequest, {
    keepPreviousData: true,
    onSuccess: (page) => {
      setPages([page]);
      setKnownGroups((current) => [...new Set([
        ...current,
        ...page.items.map((item) => item.public_group_name),
      ])]);
      setLoadCursor(null);
    },
  });
  const loadQuery = new URLSearchParams({ limit: LIST_LIMIT });
  if (deferredSearch) loadQuery.set("q", deferredSearch);
  if (group !== ALL) loadQuery.set("group", group);
  if (loadCursor) loadQuery.set("cursor", loadCursor);
  const loadMore = useSWR<MarketplaceResponse>(
    loadCursor ? `/api/public/marketplace?${loadQuery}` : null,
    marketplaceRequest,
    {
      onSuccess: (page) => {
        setPages((current) => [...current, page].slice(-3));
        setLoadCursor(null);
      },
    },
  );
  const exchangeRate = useSWR<StoreExchangeRate>(EXCHANGE_RATE_KEY, storeApi.getExchangeRate, {
    keepPreviousData: true,
  });
  const offersKey = selected
    ? `/api/public/marketplace/offers?group=${encodeURIComponent(selected.public_group_name)}&model=${encodeURIComponent(selected.model)}&limit=50`
    : null;
  const offers = useSWR<MarketplaceOffersResponse>(offersKey, marketplaceRequest, {
    keepPreviousData: false,
  });

  const resolvedPages = resolveMarketplacePages(pages, list.data);
  const items = resolvedPages.flatMap((page) => page.items);
  const capabilities = useMemo(() => [...new Set(items.flatMap((item) => item.capabilities))], [items]);
  const filtered = items.filter((item) => (
    (group === ALL || item.public_group_name === group)
    && (capability === ALL || item.capabilities.includes(capability))
  ));
  const grouped = Map.groupBy(filtered, (item) => item.public_group_name);
  const nextCursor = resolvedPages.at(-1)?.next_cursor ?? null;
  const cnyRate = exchangeRate.data?.cny_per_usd ?? null;
  const cnyUnavailable = !exchangeRate.isLoading && (!cnyRate || Boolean(exchangeRate.error));

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6">
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }} transition={transitions.normal}>
        <PageHeader title={t("modelMarketplace.title")} description={t("modelMarketplace.description")} />
      </motion.div>

      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.06, ...transitions.normal }}
        className="flex flex-col gap-6"
      >
        <div className="grid gap-3 rounded-lg border bg-card p-3 sm:grid-cols-2 lg:grid-cols-[minmax(14rem,1fr)_12rem_12rem_auto]">
          <label className="relative block">
            <span className="sr-only">{t("modelMarketplace.searchPlaceholder")}</span>
            <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input value={search} onChange={(event) => setSearch(event.target.value)} className="h-11 pl-9" placeholder={t("modelMarketplace.searchPlaceholder")} />
          </label>
          <Select value={group} onValueChange={setGroup}>
            <SelectTrigger className="h-11" aria-label={t("modelMarketplace.groupFilter")}><SelectValue /></SelectTrigger>
            <SelectContent><SelectItem value={ALL}>{t("modelMarketplace.allGroups")}</SelectItem>{knownGroups.map((item) => <SelectItem key={item} value={item}>{item}</SelectItem>)}</SelectContent>
          </Select>
          <Select value={capability} onValueChange={setCapability}>
            <SelectTrigger className="h-11" aria-label={t("modelMarketplace.capabilityFilter")}><SelectValue /></SelectTrigger>
            <SelectContent><SelectItem value={ALL}>{t("modelMarketplace.allCapabilities")}</SelectItem>{capabilities.map((item) => <SelectItem key={item} value={item}>{item}</SelectItem>)}</SelectContent>
          </Select>
          <CurrencyControl unavailable={cnyUnavailable} />
        </div>

        {currency === "CNY" && cnyUnavailable ? (
          <div className="flex flex-wrap items-center gap-3 rounded-lg border border-warning/45 bg-warning/10 px-4 py-3 text-sm text-foreground">
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <CircleDollarSign className="size-4 shrink-0 text-warning" />
              <span>{t("modelMarketplace.currencyUnavailable")}</span>
            </div>
            <Button size="sm" variant="outline" onClick={() => exchangeRate.mutate()}>
              <RefreshCw className="size-4" />{t("modelMarketplace.retry")}
            </Button>
          </div>
        ) : null}

        {list.isLoading && !list.data ? <MarketplaceSkeleton /> : list.error ? (
          <div className="flex flex-col items-start gap-3 rounded-lg border border-destructive/40 bg-destructive/5 p-5">
            <p className="text-sm text-destructive">{t("modelMarketplace.loadError")}</p>
            <Button variant="outline" onClick={() => list.mutate()}><RefreshCw className="size-4" />{t("modelMarketplace.retry")}</Button>
          </div>
        ) : filtered.length === 0 ? (
          <EmptyState icon={<Store className="size-11" />} title={t("modelMarketplace.noModels")} description={t("modelMarketplace.noModelsDesc")} />
        ) : (
          <div className="flex flex-col gap-7">
            {[...grouped.entries()].map(([groupName, groupItems], groupIndex) => (
              <motion.section
                key={groupName}
                aria-labelledby={`marketplace-group-${groupIndex}`}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: Math.min(groupIndex * 0.04, 0.16), ...transitions.normal }}
                className="flex flex-col gap-3"
              >
                <div className="flex items-center gap-2">
                  <Boxes className="size-5 text-primary" />
                  <h2 id={`marketplace-group-${groupIndex}`} className="text-lg font-semibold">{groupName}</h2>
                  <Badge variant="outline">{groupItems.length}</Badge>
                </div>
                <div className="overflow-hidden rounded-lg border bg-card">
                  {groupItems.map((item) => (
                    <ModelRow key={`${item.public_group_name}:${item.model}`} item={item} currencyRate={cnyRate} onOpen={() => setSelected(item)} />
                  ))}
                </div>
              </motion.section>
            ))}
          </div>
        )}

        {(nextCursor || loadMore.error || loadMore.isLoading) ? (
          <div className="flex flex-col items-center gap-2">
            <Button variant="outline" disabled={!nextCursor || loadMore.isLoading} onClick={() => nextCursor && setLoadCursor(nextCursor)}>
              {loadMore.isLoading ? t("modelMarketplace.loadingMore") : t("modelMarketplace.loadMore")}
            </Button>
            {loadMore.error ? <button type="button" className="min-h-11 text-sm text-destructive underline underline-offset-4" onClick={() => loadMore.mutate()}>{t("modelMarketplace.retry")}</button> : null}
          </div>
        ) : null}
      </motion.div>

      <Dialog open={selected !== null} onOpenChange={(open) => { if (!open) setSelected(null); }}>
        <DialogContent
          className="max-h-[85dvh] max-w-3xl rounded-xl"
          closeLabel={t("common.close")}
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            headingRef.current?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle ref={headingRef} tabIndex={-1} className="pr-10 font-mono text-xl outline-none">{selected?.model}</DialogTitle>
            <DialogDescription>{selected?.public_group_name} · {t("modelMarketplace.detailsDescription")}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-wrap gap-1.5" aria-label={t("modelMarketplace.capabilities")}>
            {selected?.capabilities.map((item) => <Badge key={item} variant="outline">{item}</Badge>)}
          </div>
          {offers.isLoading ? (
            <div className="flex flex-col gap-3"><Skeleton className="h-28" /><Skeleton className="h-28" /></div>
          ) : offers.error ? (
            <div className="flex flex-col items-start gap-3 rounded-lg border border-destructive/40 p-4">
              <p className="text-sm text-destructive">{t("modelMarketplace.offersError")}</p>
              <Button variant="outline" onClick={() => offers.mutate()}><RefreshCw className="size-4" />{t("modelMarketplace.retry")}</Button>
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              {offers.data?.offers.map((offer) => (
                <section key={`${offer.public_provider_name}:${offer.public_channel_name}`} className="overflow-hidden rounded-lg border">
                  <div className="flex flex-wrap items-start justify-between gap-3 border-b bg-muted/35 px-4 py-3">
                    <div>
                      <div className="text-sm font-semibold">{offer.public_provider_name}</div>
                      <div className="mt-1 text-xs text-muted-foreground">{offer.public_channel_name}</div>
                    </div>
                    <Badge variant="outline">{offer.api_type}</Badge>
                  </div>
                  <dl className="divide-y">
                    {offer.rates.map((rate, index) => (
                      <div key={`${rate.usage_class}:${index}`} className="grid gap-2 px-4 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
                        <dt className="flex flex-wrap gap-1.5">
                          <span>{rate.usage_class}</span>
                          {[rate.context_tier, rate.service_tier, rate.modality, rate.cache_ttl].filter(Boolean).map((value) => <Badge key={value} variant="secondary">{value}</Badge>)}
                        </dt>
                        <dd className="font-mono text-xs font-medium tabular-nums">
                          {currency === "CNY" && !cnyRate ? t("modelMarketplace.unavailable") : formatMarketplaceRate(rate.display_rate_nano_usd, rate.unit, currency, cnyRate ?? "1")}
                        </dd>
                      </div>
                    ))}
                  </dl>
                </section>
              ))}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}
