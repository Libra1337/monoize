import { Skeleton } from "@/components/ui/skeleton";

export function StoreSkeleton() {
  return (
    <div className="flex flex-col gap-6" aria-hidden="true">
      <div className="grid gap-3 sm:grid-cols-3">
        {[0, 1, 2].map((item) => (
          <div key={item} className="rounded-2xl border p-5">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="mt-3 h-7 w-32" />
          </div>
        ))}
      </div>
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_360px]">
        <div className="grid gap-3 sm:grid-cols-2">
          {[0, 1, 2, 3].map((item) => (
            <div key={item} className="rounded-2xl border p-5">
              <Skeleton className="h-5 w-32" />
              <Skeleton className="mt-3 h-4 w-full" />
              <Skeleton className="mt-6 h-7 w-24" />
            </div>
          ))}
        </div>
        <div className="min-h-[260px] rounded-2xl border p-5">
          <Skeleton className="h-5 w-28" />
          <Skeleton className="mt-6 h-4 w-full" />
          <Skeleton className="mt-3 h-4 w-4/5" />
          <Skeleton className="mt-20 h-11 w-full rounded-xl" />
        </div>
      </div>
      <Skeleton className="h-20 w-full rounded-2xl" />
    </div>
  );
}
