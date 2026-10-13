export type BillingRequest = {
  id: string; model: string; input_tokens: number | null; cached_tokens: number | null;
  uncached_tokens: number | null; output_tokens: number | null; charged_nano: number | null;
  status: number | null; state: string; created_at: number; settled_at: number | null;
  breakdown: { input_nano: number; cached_nano: number; output_nano: number } | null;
};
export type Wallet = {
  enabled: boolean; balance_nano: number; spent_nano: number; review_count: number;
  usage: BillingRequest[];
};
export const billingState: Record<string, string> = {
  pending: "调用中", review: "费用待核对", settled: "已结算", released: "未收费",
};
export const money = (nano: number) => new Intl.NumberFormat("zh-CN", {
  style: "currency", currency: "CNY", minimumFractionDigits: 2, maximumFractionDigits: 8,
}).format(nano / 1_000_000_000);
