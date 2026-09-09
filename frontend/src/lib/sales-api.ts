import { StoreApiError } from "./store-api";

/**
 * Client for the sales commission endpoints (`spec/sales-commission.spec.md`).
 *
 * Amounts are Coin minor units as canonical integer strings. A commission balance may carry
 * a leading `-`: a refund reverses an accrual the agent may already have withdrawn, and that
 * debt is carried until later commission repays it (SC-3.4a).
 */

const BASE = "/api/dashboard";

export interface SalesAgent {
  user_id: string;
  username: string;
  code: string;
  discount_bp: number;
  commission_balance_minor: string;
  enabled: boolean;
  created_at: string;
}

export interface SalesWindow {
  sales_minor: string;
  commission_minor: string;
  order_count: number;
}

export interface SalesCommissionEntry {
  id: string;
  order_number: string;
  base_minor: string;
  commission_minor: string;
  discount_bp: number;
  commission_rate_bp: number;
  origin: "code" | "claim";
  reversed_at: string | null;
  created_at: string;
}

/** SC-7.6: the Admin view adds the identities the agent view withholds. */
export interface AdminSalesEntry extends SalesCommissionEntry {
  agent_user_id: string;
  agent_username: string;
  buyer_user_id: string;
}

export interface SalesWithdrawal {
  id: string;
  agent_user_id: string;
  agent_username: string;
  amount_minor: string;
  state: "requested" | "paid" | "rejected" | "cancelled";
  requested_at: string;
  decided_at: string | null;
  decision_note: string | null;
}

export interface SalesOverview {
  agent: SalesAgent;
  commission_rate_bp: number;
  today: SalesWindow;
  last_7d: SalesWindow;
  last_30d: SalesWindow;
  pending_withdrawal: SalesWithdrawal | null;
}

export interface CreatedSalesAgent {
  agent: SalesAgent;
  /** SC-7.2a: returned once and never retrievable again. */
  password: string;
}

async function salesRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${BASE}${path}`, {
    credentials: "include",
    headers: {
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
    ...init,
  });
  const text = await response.text();
  const data = text ? JSON.parse(text) : {};
  if (!response.ok) {
    throw new StoreApiError(
      data.error?.message || data.error?.code || "Request failed",
      data.error?.code || "request_failed",
      response.status,
    );
  }
  return data as T;
}

export const salesApi = {
  getOverview: () => salesRequest<SalesOverview>("/sales/overview"),
  listEntries: () => salesRequest<SalesCommissionEntry[]>("/sales/entries"),
  listWithdrawals: () => salesRequest<SalesWithdrawal[]>("/sales/withdrawals"),
  claim: (orderNumber: string, userId: string) =>
    salesRequest<SalesCommissionEntry>("/sales/claims", {
      method: "POST",
      body: JSON.stringify({ order_number: orderNumber, user_id: userId }),
    }),
  cancelWithdrawal: (id: string) =>
    salesRequest<SalesWithdrawal>(`/sales/withdrawals/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  requestWithdrawal: (amountMinor: string) =>
    salesRequest<SalesWithdrawal>("/sales/withdrawals", {
      method: "POST",
      body: JSON.stringify({ amount_fen: amountMinor }),
    }),
  admin: {
    listAgents: () => salesRequest<SalesAgent[]>("/store/admin/sales/agents"),
    createAgent: (discountBp: number) =>
      salesRequest<CreatedSalesAgent>("/store/admin/sales/agents", {
        method: "POST",
        body: JSON.stringify({ discount_bp: discountBp }),
      }),
    updateAgent: (userId: string, discountBp: number, enabled: boolean) =>
      salesRequest<SalesAgent>(`/store/admin/sales/agents/${encodeURIComponent(userId)}`, {
        method: "PUT",
        body: JSON.stringify({ discount_bp: discountBp, enabled }),
      }),
    listEntries: (agentUserId?: string) =>
      salesRequest<AdminSalesEntry[]>(
        agentUserId
          ? `/store/admin/sales/entries?agent_user_id=${encodeURIComponent(agentUserId)}`
          : "/store/admin/sales/entries",
      ),
    claimForAgent: (agentUserId: string, orderNumber: string, userId: string) =>
      salesRequest<SalesCommissionEntry>("/store/admin/sales/claims", {
        method: "POST",
        body: JSON.stringify({
          agent_user_id: agentUserId,
          order_number: orderNumber,
          user_id: userId,
        }),
      }),
    listWithdrawals: () => salesRequest<SalesWithdrawal[]>("/store/admin/sales/withdrawals"),
    decideWithdrawal: (id: string, decision: "paid" | "rejected", note: string) =>
      salesRequest<SalesWithdrawal>(
        `/store/admin/sales/withdrawals/${encodeURIComponent(id)}/decide`,
        { method: "POST", body: JSON.stringify({ decision, decision_note: note }) },
      ),
    getSettings: () =>
      salesRequest<{ commission_rate_bp: number }>("/store/admin/sales/settings"),
    updateSettings: (commissionRateBp: number) =>
      salesRequest<{ commission_rate_bp: number }>("/store/admin/sales/settings", {
        method: "PUT",
        body: JSON.stringify({ commission_rate_bp: commissionRateBp }),
      }),
  },
};

/** Formats basis points as a percentage without floating-point arithmetic. */
export function formatBasisPoints(bp: number): string {
  const whole = Math.trunc(bp / 100);
  const fraction = Math.abs(bp % 100);
  return fraction === 0
    ? `${whole}%`
    : `${whole}.${String(fraction).padStart(2, "0").replace(/0$/, "")}%`;
}
