import { describe, expect, it } from "vitest";
import { billingStart, cacheRate, costText, demoDashboard, formatTokens, hasEstimatedServiceTier, openPricingSource } from "./analytics";

describe("analytics formatting and subscription period", () => {
  it("does not call request-mode estimates a confirmed lower bound", () => {
    const m = demoDashboard({ hours: 24, billingStart: 0, cycleId: null, project: null }).history;
    expect(costText({ ...m, requestModeTokens: 100, costUsd: 2 })).toBe("$2.00");
    expect(costText({ ...m, requestModeTokens: 100, costUsd: 2, unpricedTokens: 10 })).toBe("$2.00（部分未定价）");
  });
  it("displays plain currency when missing history is classified as Standard", () => {
    const m = { ...demoDashboard({ hours: 24, billingStart: 0, cycleId: null, project: null }).history, assumedStandardTokens: 100, costUsd: 2, serviceTiers: [{ tier: "standard", tokens: 100, costUsd: 2, unpricedTokens: 0, assumedStandardTokens: 100 }] };
    expect(hasEstimatedServiceTier(m)).toBe(true);
    expect(costText(m)).toBe("$2.00");
    expect(costText({ ...m, unpricedTokens: 10 })).toBe("$2.00（部分未定价）");
  });
  it("uses the configured Pacific billing day during daylight time", () => {
    expect(new Date(billingStart(new Date("2026-09-21T12:00:00Z"), { day: 10, timeZone: "America/Los_Angeles" }) * 1000).toISOString()).toBe("2026-09-10T07:00:00.000Z");
  });
  it("uses previous month before the 16th and handles standard time", () => {
    expect(new Date(billingStart(new Date("2026-01-02T12:00:00Z"), { day: 10, timeZone: "America/Los_Angeles" }) * 1000).toISOString()).toBe("2025-12-10T08:00:00.000Z");
    expect(new Date(billingStart(new Date("2026-09-10T06:59:00Z"), { day: 10, timeZone: "America/Los_Angeles" }) * 1000).toISOString()).toBe("2026-08-10T07:00:00.000Z");
  });
  it("clamps billing days for short months and respects the selected timezone", () => {
    expect(new Date(billingStart(new Date("2026-03-01T00:00:00Z"), { day: 31, timeZone: "UTC" }) * 1000).toISOString()).toBe("2026-02-28T00:00:00.000Z");
    expect(new Date(billingStart(new Date("2028-02-29T01:00:00Z"), { day: 31, timeZone: "UTC" }) * 1000).toISOString()).toBe("2028-02-29T00:00:00.000Z");
    expect(new Date(billingStart(new Date("2026-10-01T00:00:00Z"), { day: 1, timeZone: "Asia/Shanghai" }) * 1000).toISOString()).toBe("2026-09-30T16:00:00.000Z");
  });
  it("does not call unpriced usage free", () => {
    const m = demoDashboard({ hours: 24, billingStart: 0, cycleId: null, project: null }).history;
    expect(costText({ ...m, costUsd: 0, unpricedTokens: 100 })).toBe("暂无法估价");
    expect(costText({ ...m, costUsd: 2, unpricedTokens: 100 })).toBe("≥ $2.00");
    expect(cacheRate({ ...m, inputTokens: 0 })).toBe("—");
  });
  it("retains readable billion-scale totals", () => { expect(formatTokens(10210000000)).toEqual({ value: "10.21", unit: "B" }); });
  it("formats mode-based values without an approximation symbol and aggregates demo modes consistently", () => {
    const data = demoDashboard({ hours: 24, billingStart: 0, cycleId: null, project: null });
    expect(costText({ ...data.history, serviceTiers: [{ tier: "unknown", tokens: 1000, costUsd: data.history.costUsd, unpricedTokens: 0 }] })).toBe("$24.86");
    for (const metrics of [data.history, data.billing, ...data.sessions.map(row => row.metrics)]) {
      expect(metrics.serviceTiers.reduce((sum, row) => sum + row.tokens, 0)).toBe(metrics.totalTokens);
      expect(metrics.serviceTiers.reduce((sum, row) => sum + row.costUsd, 0)).toBeCloseTo(metrics.costUsd);
    }
  });
  it("only opens official HTTPS price or model sources", async () => {
    for (const url of ["javascript:alert(1)", "https://developers.openai.com.evil.test/api/docs/pricing", "https://developers.openai.com/api/docs/pricing?next=https://evil.test", "https://name:secret@developers.openai.com/api/docs/pricing", "https://developers.openai.com/api/docs/other", "http://developers.openai.com/api/docs/pricing"]) {
      await expect(openPricingSource(url)).rejects.toThrow("只能打开 OpenAI 官方 API 价格或模型页面");
    }
  });
});
