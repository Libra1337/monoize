import type {
  CreateStorePrivacyRecordInput,
  CreateStoreRetentionContainmentInput,
  MerchantCapabilityKind,
  PutStoreChannelReadinessInput,
  PutStoreMerchantCapabilityInput,
  StoreChannelReadinessView,
  StoreComplianceView,
  StoreMerchantCapabilitiesView,
  StoreMerchantCapability,
  StorePaymentChannel,
  StoreRetentionContainment,
  StoreRetentionOverview,
} from "@/lib/store-api";

export const MERCHANT_CAPABILITY_KINDS: MerchantCapabilityKind[] = [
  "payment_query",
  "refund",
  "refund_query",
  "dispute_event",
  "dispute_query",
  "bill_download",
  "settlement_report",
];

export interface PrivacyRecordDraft {
  policyVersion: string;
  jurisdiction: string;
  regions: string;
  legalBasis: string;
  evidenceDigest: string;
  financialDays: string;
  expiredGrantHours: string;
  reviewDays: string;
}

const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const REGION_PATTERN = /^[A-Za-z0-9._-]+$/;
const POSITIVE_INTEGER_PATTERN = /^[1-9][0-9]*$/;

function byteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

function integerInRange(value: string, minimum: number, maximum: number): number | null {
  if (!POSITIVE_INTEGER_PATTERN.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= minimum && parsed <= maximum ? parsed : null;
}

function decimalLessThanOrEqual(left: string, right: string): boolean {
  return left.length < right.length || (left.length === right.length && left <= right);
}

export function buildPrivacyRecordInput(draft: PrivacyRecordDraft): CreateStorePrivacyRecordInput | null {
  const policyVersion = draft.policyVersion.trim();
  const jurisdiction = draft.jurisdiction.trim();
  const legalBasis = draft.legalBasis.trim();
  const regions = draft.regions.split(",").map((value) => value.trim()).filter(Boolean);
  const allowedRegions = [...new Set(regions)];
  const financialDays = integerInRange(draft.financialDays, 1, 36_500);
  const expiredGrantHours = integerInRange(draft.expiredGrantHours, 1, 24);
  const reviewDays = integerInRange(draft.reviewDays, 1, 365);

  if (
    byteLength(policyVersion) < 1
    || byteLength(policyVersion) > 64
    || byteLength(jurisdiction) < 1
    || byteLength(jurisdiction) > 128
    || byteLength(legalBasis) < 1
    || byteLength(legalBasis) > 512
    || allowedRegions.length < 1
    || allowedRegions.length > 32
    || allowedRegions.some((region) => byteLength(region) > 64 || !REGION_PATTERN.test(region))
    || !DIGEST_PATTERN.test(draft.evidenceDigest)
    || financialDays === null
    || expiredGrantHours === null
    || reviewDays === null
  ) {
    return null;
  }

  return {
    policy_version: policyVersion,
    jurisdiction,
    allowed_regions: allowedRegions,
    retention: {
      raw_callback_days: 30,
      network_metadata_days: 90,
      financial_records_days: financialDays,
      redemption_audit_days: 730,
      expired_reauth_grant_hours: expiredGrantHours,
    },
    legal_basis: legalBasis,
    evidence_digest: draft.evidenceDigest,
    accepted: true,
    review_after_days: reviewDays,
  };
}

export function validateReadinessInput(
  adapterKind: StorePaymentChannel["adapter_kind"],
  input: PutStoreChannelReadinessInput,
): boolean {
  const currencies = input.supported_currencies;
  const actions = input.checkout_action_kinds;
  const uniqueCurrencies = new Set(currencies);
  const uniqueActions = new Set(actions);
  const limitKeys = Object.keys(input.amount_limits);
  const metadataValid =
    input.privacy_record_id === input.privacy_record_id.trim()
    && byteLength(input.privacy_record_id) >= 1
    && byteLength(input.privacy_record_id) <= 255
    && currencies.length > 0
    && currencies.length === uniqueCurrencies.size
    && actions.length > 0
    && actions.length === uniqueActions.size
    && limitKeys.length === currencies.length
    && limitKeys.every((currency) => uniqueCurrencies.has(currency as "CNY" | "USD"))
    && currencies.every((currency) => {
      const limit = input.amount_limits[currency];
      return Boolean(
        limit
        && POSITIVE_INTEGER_PATTERN.test(limit.min_minor)
        && POSITIVE_INTEGER_PATTERN.test(limit.max_minor)
        && decimalLessThanOrEqual(limit.min_minor, limit.max_minor),
      );
    })
    && [
      input.license_evidence_digest,
      input.runtime_evidence_digest,
      input.availability_evidence_digest,
    ].every((value) => DIGEST_PATTERN.test(value))
    && Number.isInteger(input.valid_for_days)
    && input.valid_for_days >= 1
    && input.valid_for_days <= 90;

  if (!metadataValid) return false;
  if (adapterKind === "alipay") {
    return currencies.length === 1 && currencies[0] === "CNY" && actions.length === 1 && actions[0] === "form";
  }
  if (adapterKind === "wechat") {
    return currencies.length === 1
      && currencies[0] === "CNY"
      && actions.every((action) => action === "qr" || action === "redirect");
  }
  if (adapterKind === "stripe") {
    return currencies.every((currency) => currency === "CNY" || currency === "USD")
      && actions.length === 1
      && actions[0] === "redirect";
  }
  return false;
}

export function optimisticReadiness(
  channel: StorePaymentChannel,
  input: PutStoreChannelReadinessInput,
  current: StoreChannelReadinessView | undefined,
): StoreChannelReadinessView {
  const now = new Date();
  return {
    readiness: {
      channel_id: channel.id,
      active_credential_digest: current?.readiness?.active_credential_digest ?? "pending",
      verifier_admin_id: current?.readiness?.verifier_admin_id ?? "pending",
      verified_at: now.toISOString(),
      expires_at: new Date(now.getTime() + input.valid_for_days * 86_400_000).toISOString(),
      privacy_record_id: input.privacy_record_id,
      callback_verification_passed: input.callback_verification_passed,
      supported_currencies: input.supported_currencies,
      amount_limits: input.amount_limits,
      checkout_action_kinds: input.checkout_action_kinds,
      license_evidence_digest: input.license_evidence_digest,
      runtime_evidence_digest: input.runtime_evidence_digest,
      availability_evidence_digest: input.availability_evidence_digest,
    },
  };
}

const NON_WHITESPACE_PATTERN = /^\S+$/;

export function validateCapabilityInput(input: PutStoreMerchantCapabilityInput): boolean {
  const environment = input.environment.trim();
  const providerProduct = input.provider_product.trim();
  const controlledId = input.controlled_transaction_id?.trim() ?? "";
  return (
    environment.length >= 1
    && environment.length <= 128
    && NON_WHITESPACE_PATTERN.test(environment)
    && providerProduct.length >= 1
    && providerProduct.length <= 128
    && NON_WHITESPACE_PATTERN.test(providerProduct)
    && DIGEST_PATTERN.test(input.evidence_digest)
    && (
      input.controlled_transaction_id === null
      || (
        controlledId.length >= 1
        && controlledId.length <= 256
        && NON_WHITESPACE_PATTERN.test(controlledId)
      )
    )
  );
}

export function optimisticCompliance(
  channelId: string,
  termsVersion: string,
  current: StoreComplianceView | undefined,
): StoreComplianceView {
  const now = new Date();
  return {
    current_terms_version: termsVersion,
    compliance: {
      id: `optimistic-${crypto.randomUUID()}`,
      channel_id: channelId,
      terms_version: termsVersion,
      admin_user_id: current?.compliance?.admin_user_id ?? "pending",
      source_ip: current?.compliance?.source_ip ?? "pending",
      confirmed_at: now.toISOString(),
      invalidated_at: null,
    },
  };
}

export function optimisticContainment(
  input: CreateStoreRetentionContainmentInput,
  current: StoreRetentionOverview | undefined,
): StoreRetentionOverview | undefined {
  if (!current) return current;
  const containment: StoreRetentionContainment = {
    id: `optimistic-${crypto.randomUUID()}`,
    alert_id: current.status.active_alert?.id ?? "pending",
    actor_id: "pending",
    reason: input.reason,
    evidence_digest: input.evidence_digest,
    created_at: new Date().toISOString(),
  };
  // SB-PR-12B: containment clears the pause and the active alert but keeps
  // the consecutive failure count, so those are the only status fields
  // mutated optimistically.
  return {
    ...current,
    status: {
      ...current.status,
      checkout_paused: false,
      active_alert: null,
      latest_containment_id: containment.id,
    },
    containments: [containment, ...current.containments],
  };
}

export function optimisticCapability(
  channelId: string,
  capability: MerchantCapabilityKind,
  input: PutStoreMerchantCapabilityInput,
  current: StoreMerchantCapabilitiesView | undefined,
): StoreMerchantCapabilitiesView {
  const now = new Date();
  const existing = current?.capabilities.find((item) => item.capability === capability);
  const record: StoreMerchantCapability = {
    id: existing?.id ?? `optimistic-${crypto.randomUUID()}`,
    channel_id: channelId,
    capability,
    state: input.state,
    environment: input.environment.trim(),
    merchant_account_digest: existing?.merchant_account_digest ?? "pending",
    provider_product: input.provider_product.trim(),
    evidence_digest: input.evidence_digest,
    controlled_transaction_id: input.controlled_transaction_id?.trim() ?? null,
    verifier_admin_id: existing?.verifier_admin_id ?? "pending",
    verified_at: now.toISOString(),
    expires_at: new Date(now.getTime() + 90 * 86_400_000).toISOString(),
  };
  const capabilities = [
    record,
    ...(current?.capabilities.filter((item) => item.capability !== capability) ?? []),
  ].sort((left, right) => left.capability.localeCompare(right.capability));
  return { capabilities };
}
