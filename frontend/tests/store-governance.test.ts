import { describe, expect, test } from "bun:test";
import * as governanceState from "../src/pages/store-admin/governance-state";

type PrivacyDraft = {
  policyVersion: string;
  jurisdiction: string;
  regions: string;
  legalBasis: string;
  evidenceDigest: string;
  financialDays: string;
  expiredGrantHours: string;
  reviewDays: string;
};

type PrivacyBuilder = (draft: PrivacyDraft) => unknown | null;
type ReadinessValidator = (
  adapterKind: "alipay" | "wechat" | "stripe" | "epay",
  input: {
    privacy_record_id?: string;
    callback_verification_passed: boolean;
    supported_currencies: Array<"CNY" | "USD">;
    amount_limits: Partial<Record<"CNY" | "USD", { min_minor: string; max_minor: string }>>;
    checkout_action_kinds: Array<"redirect" | "qr" | "form">;
    license_evidence_digest?: string;
    runtime_evidence_digest?: string;
    availability_evidence_digest?: string;
    valid_for_days: number;
  },
) => boolean;

const privacyDraft: PrivacyDraft = {
  policyVersion: "privacy-2026-08",
  jurisdiction: "CN",
  regions: "CN, HK",
  legalBasis: "Contract and accounting retention",
  evidenceDigest: "a".repeat(64),
  financialDays: "2557",
  expiredGrantHours: "24",
  reviewDays: "90",
};

const readinessInput = {
  privacy_record_id: "privacy-1",
  callback_verification_passed: true,
  supported_currencies: ["CNY", "USD"] as Array<"CNY" | "USD">,
  amount_limits: {
    CNY: { min_minor: "1", max_minor: "100000000" },
    USD: { min_minor: "1", max_minor: "100000000" },
  },
  checkout_action_kinds: ["redirect"] as Array<"redirect" | "qr" | "form">,
  license_evidence_digest: "b".repeat(64),
  runtime_evidence_digest: "c".repeat(64),
  availability_evidence_digest: "d".repeat(64),
  valid_for_days: 30,
};

describe("Store governance form validation", () => {
  test("builds only an exact valid Privacy Record request", () => {
    const build = (governanceState as Record<string, unknown>).buildPrivacyRecordInput as PrivacyBuilder | undefined;
    expect(typeof build).toBe("function");
    if (!build) return;

    expect(build(privacyDraft)).toEqual({
      policy_version: "privacy-2026-08",
      jurisdiction: "CN",
      allowed_regions: ["CN", "HK"],
      retention: {
        raw_callback_days: 30,
        network_metadata_days: 90,
        financial_records_days: 2557,
        redemption_audit_days: 730,
        expired_reauth_grant_hours: 24,
      },
      legal_basis: "Contract and accounting retention",
      evidence_digest: "a".repeat(64),
      accepted: true,
      review_after_days: 90,
    });

    expect(build({ ...privacyDraft, regions: "CN, invalid region" })).toBeNull();
    expect(build({ ...privacyDraft, financialDays: "0" })).toBeNull();
    expect(build({ ...privacyDraft, expiredGrantHours: "25" })).toBeNull();
    expect(build({ ...privacyDraft, reviewDays: "366" })).toBeNull();
    expect(build({ ...privacyDraft, policyVersion: "x".repeat(65) })).toBeNull();
  });

  test("rejects noncanonical, inverted, or adapter-incompatible Readiness limits", () => {
    const validate = (governanceState as Record<string, unknown>).validateReadinessInput as ReadinessValidator | undefined;
    expect(typeof validate).toBe("function");
    if (!validate) return;

    expect(validate("stripe", readinessInput)).toBe(true);
    expect(validate("stripe", {
      ...readinessInput,
      amount_limits: { ...readinessInput.amount_limits, USD: { min_minor: "010", max_minor: "20" } },
    })).toBe(false);
    expect(validate("stripe", {
      ...readinessInput,
      amount_limits: { ...readinessInput.amount_limits, USD: { min_minor: "21", max_minor: "20" } },
    })).toBe(false);
    expect(validate("alipay", readinessInput)).toBe(false);
  });

  // SB-C-38: EPay has no issuer for a licence, runtime, availability, or privacy attestation.
  // The server rejects a partially filled profile, so the exempt form must carry none of them.
  test("accepts an EPay Readiness profile that carries no attestation fields", () => {
    const validate = (governanceState as Record<string, unknown>).validateReadinessInput as ReadinessValidator | undefined;
    expect(typeof validate).toBe("function");
    if (!validate) return;

    const epayInput = {
      callback_verification_passed: true,
      supported_currencies: ["CNY"] as Array<"CNY" | "USD">,
      amount_limits: { CNY: { min_minor: "1", max_minor: "100000000" } },
      checkout_action_kinds: ["qr", "redirect"] as Array<"redirect" | "qr" | "form">,
      valid_for_days: 30,
    };

    expect(validate("epay", epayInput)).toBe(true);

    // A gated adapter still needs every attestation, so the same payload must fail for Stripe.
    expect(validate("stripe", epayInput)).toBe(false);

    // An exempt adapter must not smuggle an attestation the gate will never read.
    expect(validate("epay", { ...epayInput, privacy_record_id: "privacy-1" })).toBe(false);
    expect(validate("epay", { ...epayInput, license_evidence_digest: "b".repeat(64) })).toBe(false);

    // The adapter's own currency and action constraints still apply.
    expect(validate("epay", { ...epayInput, supported_currencies: ["USD"], amount_limits: { USD: { min_minor: "1", max_minor: "2" } } })).toBe(false);
    expect(validate("epay", { ...epayInput, checkout_action_kinds: ["form"] })).toBe(false);
  });

  test("rejects a Stripe Readiness profile that omits an attestation", () => {
    const validate = (governanceState as Record<string, unknown>).validateReadinessInput as ReadinessValidator | undefined;
    expect(typeof validate).toBe("function");
    if (!validate) return;

    expect(validate("stripe", { ...readinessInput, privacy_record_id: undefined })).toBe(false);
    expect(validate("stripe", { ...readinessInput, license_evidence_digest: undefined })).toBe(false);
    expect(validate("stripe", { ...readinessInput, runtime_evidence_digest: undefined })).toBe(false);
    expect(validate("stripe", { ...readinessInput, availability_evidence_digest: undefined })).toBe(false);
  });

  test("accepts only canonical capability verification input", () => {
    const validate = (governanceState as Record<string, unknown>).validateCapabilityInput as ((input: {
      state: "supported" | "unsupported" | "manual";
      environment: string;
      provider_product: string;
      evidence_digest: string;
      controlled_transaction_id: string | null;
    }) => boolean) | undefined;
    expect(typeof validate).toBe("function");
    if (!validate) return;

    expect(validate({
      state: "supported",
      environment: "sandbox",
      provider_product: "checkout",
      evidence_digest: "a".repeat(64),
      controlled_transaction_id: "controlled-refund",
    })).toBe(true);
    expect(validate({
      state: "supported",
      environment: "   ",
      provider_product: "checkout",
      evidence_digest: "a".repeat(64),
      controlled_transaction_id: null,
    })).toBe(false);
    expect(validate({
      state: "supported",
      environment: "sandbox",
      provider_product: "checkout",
      evidence_digest: "A".repeat(64),
      controlled_transaction_id: null,
    })).toBe(false);
  });
});
