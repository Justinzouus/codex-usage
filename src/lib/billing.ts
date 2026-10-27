import { isTauri } from "./bridge";

export interface BillingSettings { monthlyUsd: number; extraCreditsUsd: number; day: number; timeZone: string }
export const billingTimeZones = [
  ["Asia/Shanghai", "北京时间"], ["America/Los_Angeles", "美国太平洋时间"],
  ["America/New_York", "美国东部时间"], ["Asia/Tokyo", "日本时间"],
  ["Europe/London", "伦敦时间"], ["UTC", "UTC"],
] as const;
export const billingTimeZoneLabel = (zone: string) => billingTimeZones.find(([id]) => id === zone)?.[1] ?? zone;
const previewKey = "codex-usage-demo-billing";

export function validateBilling(settings: BillingSettings): boolean {
  return Number.isFinite(settings.monthlyUsd) && settings.monthlyUsd > 0 && settings.monthlyUsd <= 1_000_000
    && Number.isFinite(settings.extraCreditsUsd) && settings.extraCreditsUsd >= 0 && settings.extraCreditsUsd <= 1_000_000
    && Number.isInteger(settings.day) && settings.day >= 1 && settings.day <= 31
    && billingTimeZones.some(([zone]) => zone === settings.timeZone);
}

export async function getBillingSettings(): Promise<BillingSettings | null> {
  if (!isTauri()) {
    const raw = localStorage.getItem(previewKey);
    if (!raw) return null;
    const value: BillingSettings = JSON.parse(raw);
    if (!validateBilling(value)) throw new Error("本地账期设置无效，请重新设置。");
    return value;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<BillingSettings | null>("get_billing_settings");
}

export async function saveBillingSettings(settings: BillingSettings): Promise<void> {
  if (!validateBilling(settings)) throw new Error("请填写有效的金额、扣款日与时区。");
  if (!isTauri()) { localStorage.setItem(previewKey, JSON.stringify(settings)); return; }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_billing_settings", { settings });
}
