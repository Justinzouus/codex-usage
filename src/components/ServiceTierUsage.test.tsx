// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it } from "vitest";
import ServiceTierUsage from "./ServiceTierUsage";
import { demoDashboard, type ServiceTierUsage as TierUsage } from "../lib/analytics";

afterEach(cleanup);
const metrics = demoDashboard({ hours: 24, billingStart: 0, cycleId: null, project: null }).history;
const usage = (rows: TierUsage[]) => ({ ...metrics, serviceTiers: rows });

describe("service mode usage and costs", () => {
  it("defaults to one collapsed methodology entry and reveals mode costs on demand", () => {
    render(<ServiceTierUsage metrics={{...usage([{ tier: "fast", tokens: 1000, costUsd: 2, unpricedTokens: 0, requestModeTokens: 1000 }]), requestModeTokens: 1000}} scope="当前时段"/>);
    const summary = screen.getByText("API等价费用计算口径");
    expect(summary.closest("details")).not.toHaveAttribute("open");
    expect(screen.getByText("$2.00")).not.toBeVisible();
    expect(document.querySelector(".du-tier-highlight")).not.toBeInTheDocument();
    expect(screen.queryByText(/逐轮记录识别|服务端实际处理模式未提供|模式未记录|≈/)).not.toBeInTheDocument();
    fireEvent.click(summary);
    expect(summary.closest("details")).toHaveAttribute("open");
    expect(screen.getByText("$2.00")).toBeVisible();
    fireEvent.click(summary);
    expect(summary.closest("details")).not.toHaveAttribute("open");
  });
  it("keeps API Fast cost separate from the subscription and credit multipliers", () => {
    render(<ServiceTierUsage metrics={usage([{ tier: "fast", tokens: 225000, costUsd: 5.1, unpricedTokens: 0 }])} scope="当前时段"/>);
    fireEvent.click(screen.getByText("API等价费用计算口径"));
    const rules = within(screen.getByRole("table", { name: "Fast 计费口径" }));
    expect(rules.getByRole("row", { name: /套餐内额度/ })).toHaveTextContent("2.5× Standard 额度");
    expect(rules.getByRole("row", { name: /额外 credits/ })).toHaveTextContent("2× Standard 费率");
    expect(rules.getByRole("row", { name: /API 等价费用/ })).toHaveTextContent("模型的 Fast Token 单价");
    expect(screen.queryByText("$6.38")).not.toBeInTheDocument();
    expect(screen.getByText(/套餐剩余额度以 Codex 返回值为准/)).toBeInTheDocument();
  });
  it("shows actual Fast tokens and mode-specific cost without multiplying tokens", () => {
    render(<ServiceTierUsage metrics={usage([{ tier: "fast", tokens: 1000, costUsd: 1.5, unpricedTokens: 0 }])} scope="当前时段"/>);
    fireEvent.click(screen.getByText("API等价费用计算口径"));
    expect(screen.getByText("1 K Token")).toBeInTheDocument();
    expect(screen.getByText("$1.50")).toBeInTheDocument();
    expect(screen.queryByText("1.5 K Token")).not.toBeInTheDocument();
    expect(screen.getByText(/Token 为实际记录数量/)).toBeInTheDocument();
  });
  it("includes missing historical modes in Standard and explains the classification in details", () => {
    render(<ServiceTierUsage metrics={{...usage([{ tier: "standard", tokens: 3000, costUsd: 3, unpricedTokens: 0, assumedStandardTokens: 2000 }]), assumedStandardTokens: 2000}} scope="本账期"/>);
    expect(screen.queryByText(/模式未记录|Fast 附加费用未确认/)).not.toBeInTheDocument();
    expect(document.querySelector(".du-tier-highlight")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("API等价费用计算口径"));
    expect(screen.getByText("Standard")).toBeInTheDocument();
    expect(screen.getByText("3 K Token")).toBeInTheDocument();
    expect(screen.getByText("$3.00")).toBeInTheDocument();
    expect(screen.getByText(/历史模式缺失的 2 K Token 已归入 Standard/)).toBeInTheDocument();
  });
  it("does not call an unknown mode or a missing Fast price free", () => {
    render(<ServiceTierUsage metrics={usage([{ tier: "fast", tokens: 2000, costUsd: 0, unpricedTokens: 2000 }, { tier: "special", tokens: 1000, costUsd: 0, unpricedTokens: 1000 }])} scope="当前时段"/>);
    fireEvent.click(screen.getByText("API等价费用计算口径"));
    expect(screen.getAllByText("暂无法估价")).toHaveLength(2);
    expect(screen.getByText("special")).toBeInTheDocument();
    expect(screen.getByText("2 K Token 暂无此模式的单价")).toBeInTheDocument();
    expect(screen.queryByText("$0.00")).not.toBeInTheDocument();
  });
});
