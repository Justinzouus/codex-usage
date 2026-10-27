// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
import PricingStatus from "./PricingStatus";
import { openPricingSource, type PricingSync } from "../lib/analytics";

vi.mock("../lib/analytics", async importOriginal => ({ ...await importOriginal<typeof import("../lib/analytics")>(), openPricingSource: vi.fn(async () => {}) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const pricing: PricingSync = { status: "upToDate", lastCheckedAt: 1791840000, lastSuccessAt: 1791840000, sourceUrl: "https://developers.openai.com/api/docs/pricing", usingCachedRates: false, error: null, missingModels: [], modelCount: 30 };

describe("API pricing sync feedback", () => {
  it("distinguishes a successful current table refresh from retained historical rates", () => {
    render(<PricingStatus pricing={{ ...pricing, status: "partial", usingCachedRates: true, error: "官方本次价格表未包含 standard:old-model（2026-10-20）；这些项保留此前核验价格；old-model 补查失败：HTTP 404" }} refreshing={false} onRefresh={() => {}}/>);
    expect(screen.getByRole("status")).toHaveTextContent("官方价格已更新 · 部分旧价待核对");
    expect(screen.queryByText("价格更新失败")).not.toBeInTheDocument();
    expect(screen.getByText(/standard:old-model/)).not.toBeVisible();
    fireEvent.click(screen.getByText("价格详情"));
    expect(screen.getByText(/standard:old-model/)).toBeVisible();
    expect(screen.getByText(/old-model 补查失败：HTTP 404/)).toBeVisible();
    expect(screen.getByRole("button", { name: "立即更新 API 价格" })).toBeEnabled();
  });
  it("keeps successful sync compact and exposes its official source on demand", async () => {
    render(<PricingStatus pricing={pricing} refreshing={false} onRefresh={() => {}}/>);
    expect(screen.getByRole("status")).toHaveTextContent("官方价格已同步");
    expect(document.querySelector("details")).not.toHaveAttribute("open");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("价格详情"));
    fireEvent.click(screen.getByRole("button", { name: "查看官方价格表" }));
    expect(openPricingSource).toHaveBeenCalledWith(pricing.sourceUrl);
  });
  it("shows failed checks with the actual reason and cached-rate fallback", () => {
    render(<PricingStatus pricing={{ ...pricing, status: "failed", error: "官方价格页请求超时", usingCachedRates: true }} refreshing={false} onRefresh={() => {}}/>);
    expect(screen.getByRole("alert")).toHaveTextContent("官方价格页请求超时");
    expect(screen.getByRole("alert")).toHaveTextContent("仍使用上次成功缓存的单价");
    expect(screen.getByRole("status")).toHaveTextContent("价格更新失败");
  });
  it("names unresolved models, token totals and reasons in all-history scope", () => {
    render(<PricingStatus pricing={{ ...pricing, missingModels: [{ model: "gpt-unreleased", tokens: 1220000000, reason: "官方未公开此模型标识的 API 单价，不能推测别名价格。", sourceUrl: pricing.sourceUrl }] }} refreshing={false} onRefresh={() => {}}/>);
    expect(screen.getByText(/全部本机历史中 1 个模型未定价（1.22 B Token）/)).toBeInTheDocument();
    fireEvent.click(screen.getByText(/全部本机历史中/));
    expect(screen.getByText("gpt-unreleased")).toBeInTheDocument();
    expect(screen.getByText(/不能推测别名价格/)).toBeInTheDocument();
    expect(screen.getByText(/不随上方时间范围或项目筛选变化/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "查看 gpt-unreleased 的官方来源" })).toBeInTheDocument();
  });
  it("reports an unverified initial state and disables duplicate manual checks", () => {
    const onRefresh = vi.fn();
    const { rerender } = render(<PricingStatus pricing={{ ...pricing, status: "neverChecked", lastSuccessAt: null }} refreshing={false} onRefresh={onRefresh}/>);
    expect(screen.getByRole("status")).toHaveTextContent("尚未核对在线价格");
    expect(screen.getByText("尚无成功同步记录")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "立即更新 API 价格" }));
    expect(onRefresh).toHaveBeenCalledOnce();
    rerender(<PricingStatus pricing={pricing} refreshing onRefresh={onRefresh}/>);
    expect(screen.getByRole("button", { name: "立即更新 API 价格" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("正在查询官方价格");
  });
  it("reports source-opening failures", async () => {
    vi.mocked(openPricingSource).mockRejectedValueOnce(new Error("launch failed"));
    render(<PricingStatus pricing={pricing} refreshing={false} onRefresh={() => {}}/>);
    fireEvent.click(screen.getByText("价格详情"));
    fireEvent.click(screen.getByRole("button", { name: "查看官方价格表" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("官方来源页面未能打开");
  });
  it("counts a model once when several modes have different missing-price reasons", () => {
    render(<PricingStatus pricing={{ ...pricing, missingModels: [{ model: "gpt-future", tokens: 1000, reason: "Fast 单价未公开", sourceUrl: pricing.sourceUrl }, { model: "gpt-future", tokens: 2000, reason: "Ultrafast 单价未公开", sourceUrl: pricing.sourceUrl }] }} refreshing={false} onRefresh={() => {}}/>);
    expect(screen.getByText(/全部本机历史中 1 个模型未定价（3 K Token）/)).toBeInTheDocument();
    fireEvent.click(screen.getByText(/全部本机历史中/));
    expect(screen.getByText("Fast 单价未公开")).toBeInTheDocument();
    expect(screen.getByText("Ultrafast 单价未公开")).toBeInTheDocument();
  });
});
