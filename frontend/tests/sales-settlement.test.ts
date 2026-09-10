import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const source = (path: string) => readFileSync(join(import.meta.dir, path), "utf8");

const cardsSource = source("../src/pages/sales/sales-cards.tsx");
const agentPageSource = source("../src/pages/sales/index.tsx");
const adminAgentsSource = source("../src/pages/sales-admin/sales-admin-agents.tsx");
const locales = ["en", "zh", "zh-TW", "ja"].map((name) =>
  JSON.parse(source(`../src/locales/${name}.json`)),
);

describe("Sales settlement figures", () => {
  // SC-6.9: four figures, not one balance. A balance alone cannot tell an agent whether a
  // payout is awaiting approval or was never requested.
  test("shows all four figures on the agent page", () => {
    for (const field of [
      "accrued_minor",
      "available_minor",
      "pending_withdrawal_minor",
      "withdrawn_minor",
    ]) {
      expect(cardsSource).toContain(`settlement.${field}`);
    }
    expect(agentPageSource).toContain("<SalesSettlementCard");
    expect(agentPageSource).toContain("settlement={data.agent.settlement}");
  });

  // SC-6.10: Admin reads the same four per agent, so the two surfaces cannot disagree.
  test("shows the same four figures per agent in the Admin roster", () => {
    for (const field of [
      "accrued_minor",
      "available_minor",
      "pending_withdrawal_minor",
      "withdrawn_minor",
    ]) {
      expect(adminAgentsSource).toContain(`agent.settlement.${field}`);
    }
  });

  // Money is carried as minor-unit strings precisely so no float touches it; summing the
  // roster with Number would reintroduce that.
  test("sums the platform totals without floating point", () => {
    const start = adminAgentsSource.indexOf("const totals = useMemo");
    expect(start).toBeGreaterThan(-1);
    const block = adminAgentsSource.slice(start, adminAgentsSource.indexOf("}, [agents]);", start));
    expect(block).toContain("BigInt(");
    expect(block).not.toContain("Number(");
    expect(block).not.toContain("parseFloat");
    expect(block).not.toContain("parseInt");
  });

  test("labels every figure in all locales", () => {
    for (const locale of locales) {
      for (const key of ["title", "accrued", "available", "pending", "withdrawn"]) {
        expect(locale.sales.settlement[key]).toBeString();
      }
    }
  });
});
