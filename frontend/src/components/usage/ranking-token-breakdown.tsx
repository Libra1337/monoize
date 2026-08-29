import { useTranslation } from "react-i18next";

import { AnimatedTokenValue } from "@/components/usage/token-summary";

function integer(value: string): bigint {
  return /^(?:0|[1-9]\d*)$/.test(value) ? BigInt(value) : 0n;
}

export function RankingTokenBreakdown({
  input,
  cacheRead,
  output,
}: {
  input: string;
  cacheRead: string;
  output: string;
}) {
  const { t } = useTranslation();
  return (
    <div className="mt-1 flex flex-wrap justify-end gap-x-2 gap-y-0.5 font-mono text-[10px] tabular-nums">
      <span className="text-primary">
        {t("adminUsage.inputTokens")} <AnimatedTokenValue value={integer(input)} />
      </span>
      <span className="text-warning-foreground">
        {t("adminUsage.cacheTokens")} <AnimatedTokenValue value={integer(cacheRead)} />
      </span>
      <span className="text-success">
        {t("adminUsage.outputTokens")} <AnimatedTokenValue value={integer(output)} />
      </span>
    </div>
  );
}
