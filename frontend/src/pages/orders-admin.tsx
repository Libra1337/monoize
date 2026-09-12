import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import {
  storeApi,
  type AdminOrderOperationResult,
  type AdminStoreOrderDetail,
  type StoreRefundRecord,
} from "@/lib/store-api";
import { AdminLoadState, OrdersPanel } from "./store-admin/admin-panels";
import { OrderDialog } from "./store-admin/order-dialog";

const ORDERS_KEY = "/api/dashboard/store/admin/orders";

type AdminOrderDetailCache = AdminStoreOrderDetail & { pending_action?: string };

function applyOrderOperation(
  current: AdminOrderDetailCache | undefined,
  result: AdminOrderOperationResult,
): AdminOrderDetailCache | undefined {
  if (!current) return current;
  return {
    ...current,
    order: result.order,
    attempts: current.attempts.map((attempt) => (
      attempt.id === result.attempt.id ? result.attempt : attempt
    )),
  };
}

function applyRefund(
  current: AdminOrderDetailCache | undefined,
  refund: StoreRefundRecord,
): AdminOrderDetailCache | undefined {
  if (!current) return current;
  const exists = current.refunds.some((item) => item.id === refund.id);
  return {
    ...current,
    refunds: exists
      ? current.refunds.map((item) => (item.id === refund.id ? refund : item))
      : [refund, ...current.refunds],
  };
}

/** SB-UI-10O: the order statistics surface stands on its own admin sub-page. */
export function OrdersAdminPage() {
  const { t } = useTranslation();
  const [selectedOrderId, setSelectedOrderId] = useState<string | null>(null);

  const orders = useSWR(ORDERS_KEY, () => storeApi.admin.listOrders(100));
  const orderDetail = useSWR<AdminOrderDetailCache>(
    selectedOrderId
      ? `/api/dashboard/store/admin/orders/${encodeURIComponent(selectedOrderId)}`
      : null,
    () => storeApi.admin.getOrderDetail(selectedOrderId!),
  );

  const refreshSelectedOrder = async () => {
    await Promise.all([orders.mutate(), orderDetail.mutate()]);
  };

  const mutateSelectedOrderDetail = async <T,>(
    actionKey: string,
    request: () => Promise<T>,
    apply: (current: AdminOrderDetailCache | undefined, result: T) => AdminOrderDetailCache | undefined,
  ): Promise<T> => {
    const cachedDetail = orderDetail.data;
    if (!cachedDetail) throw new Error(t("common.error"));
    let result: T | undefined;
    await orderDetail.mutate(async (current) => {
      result = await request();
      const currentDetail = current ?? cachedDetail;
      const applied = apply(currentDetail, result);
      return { ...(applied ?? currentDetail), pending_action: undefined };
    }, {
      optimisticData: (current) => ({ ...(current ?? cachedDetail), pending_action: actionKey }),
      rollbackOnError: true,
      revalidate: false,
    });
    return result as T;
  };

  const querySelectedOrder = async (attemptId: string) => {
    if (!selectedOrderId) return;
    try {
      await mutateSelectedOrderDetail(
        `query:${attemptId}`,
        () => storeApi.admin.queryOrder(selectedOrderId, attemptId),
        applyOrderOperation,
      );
      await refreshSelectedOrder();
      toast.success(t("store.admin.orders.querySucceeded"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
    }
  };

  const closeSelectedOrder = async (attemptId: string) => {
    if (!selectedOrderId) return;
    try {
      await mutateSelectedOrderDetail(
        `close:${attemptId}`,
        () => storeApi.admin.closeOrder(selectedOrderId, attemptId),
        applyOrderOperation,
      );
      await refreshSelectedOrder();
      toast.success(t("store.admin.orders.closeSucceeded"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
    }
  };

  const createSelectedOrderRefund = async (currentPassword: string) => {
    if (!selectedOrderId) return;
    try {
      await mutateSelectedOrderDetail(
        "refund:create",
        async () => {
          const grant = await storeApi.admin.createReauthGrant(currentPassword, "refund");
          return storeApi.admin.createRefund(selectedOrderId, crypto.randomUUID(), grant.token);
        },
        applyRefund,
      );
      await refreshSelectedOrder();
      toast.success(t("store.admin.orders.refundCreated"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
      throw cause;
    }
  };

  const querySelectedOrderRefund = async (refundId: string, currentPassword: string) => {
    if (!selectedOrderId) return;
    try {
      await mutateSelectedOrderDetail(
        `refund:query:${refundId}`,
        async () => {
          const grant = await storeApi.admin.createReauthGrant(currentPassword, "refund");
          return storeApi.admin.queryRefund(selectedOrderId, refundId, grant.token);
        },
        applyRefund,
      );
      await refreshSelectedOrder();
      toast.success(t("store.admin.orders.refundQuerySucceeded"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
      throw cause;
    }
  };

  return (
    <PageWrapper className="space-y-6">
      <PageHeader
        title={t("store.admin.orders.title")}
        description={t("store.admin.orders.descriptionText")}
      />
      <AdminLoadState
        loading={orders.isLoading}
        error={orders.error}
        onRetry={() => void orders.mutate()}
      >
        <OrdersPanel orders={orders.data ?? []} onSelectOrder={setSelectedOrderId} />
      </AdminLoadState>

      <OrderDialog
        open={!!selectedOrderId}
        detail={orderDetail.data}
        loading={orderDetail.isLoading}
        error={orderDetail.error}
        actionLoading={orderDetail.data?.pending_action ?? null}
        onOpenChange={(open) => !open && setSelectedOrderId(null)}
        onRetry={() => void orderDetail.mutate()}
        onQueryAttempt={querySelectedOrder}
        onCloseAttempt={closeSelectedOrder}
        onCreateRefund={createSelectedOrderRefund}
        onQueryRefund={querySelectedOrderRefund}
      />
    </PageWrapper>
  );
}
