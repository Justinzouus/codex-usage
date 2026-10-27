// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import Dashboard from "./Dashboard";
import { demoDashboard, type AnalyticsQuery, type DashboardData } from "../lib/analytics";
import { openConversation } from "../lib/bridge";
import { getBillingSettings } from "../lib/billing";

vi.mock("../lib/billing", async importOriginal => ({ ...await importOriginal<typeof import("../lib/billing")>(), getBillingSettings: vi.fn(async () => ({ monthlyUsd: 20, extraCreditsUsd: 0, day: 1, timeZone: "UTC" })) }));

vi.mock("../lib/analytics", async importOriginal => {
  const actual = await importOriginal<typeof import("../lib/analytics")>();
  return { ...actual, fetchAnalytics: vi.fn(async (query: AnalyticsQuery) => actual.demoDashboard(query)) };
});
vi.mock("../lib/bridge", async importOriginal => ({ ...await importOriginal<typeof import("../lib/bridge")>(), openConversation: vi.fn(async()=>{}) }));
beforeAll(() => { vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} }); });
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe("conversation-level usage dashboard", () => {
  it("keeps four modules and replaces verbose methodology with short reminders", async () => {
    render(<Dashboard />);
    await screen.findByRole("heading", { name: "历史时段用量" });
    for (const name of ["用量趋势与节约费用","重置周期用量","最近执行明细"]) expect(screen.getByRole("heading",{name})).toBeInTheDocument();
    expect(screen.queryByText("本地统计范围与准确性")).not.toBeInTheDocument();
    expect(screen.queryByText("估算差额，非实际返现 · 计算口径")).not.toBeInTheDocument();
    expect(screen.getByText(/仅统计本机记录，费用为估算/)).toBeInTheDocument();
    const summaries = screen.getAllByText("API等价费用计算口径");
    expect(summaries).toHaveLength(2);
    for (const summary of summaries) expect(summary.closest("details")).not.toHaveAttribute("open");
    expect(screen.queryByText(/逐轮记录识别|实际处理模式可能不同|不是实际账单或返现/)).not.toBeInTheDocument();
    expect(screen.queryByText(/上一次执行/)).not.toBeInTheDocument();
  });
  it("aggregates repeated executions into one conversation and opens its child window", async () => {
    render(<Dashboard />);
    const detail=await screen.findByRole("button",{name:"查看会话详情：Codex Usage · 界面优化"});
    const table=within(within(screen.getByRole("region", { name: "最近执行明细" })).getByRole("table"));
    expect(table.queryByText("优化周额度详情卡")).not.toBeInTheDocument();
    expect(table.getByText("816.4")).toBeInTheDocument();
    expect(table.getByText("$4.99")).toBeInTheDocument();
    expect(table.getAllByText("Codex Usage · 界面优化")).toHaveLength(1);
    fireEvent.click(detail);
    expect(openConversation).toHaveBeenCalledWith("Codex Usage-thread-0");
  });
  it("paginates conversations in groups of five rather than executions", async () => {
    render(<Dashboard />);await screen.findByRole("heading",{name:"最近执行明细"});
    expect(document.querySelectorAll(".du-session-row")).toHaveLength(5);
    fireEvent.click(screen.getByRole("button",{name:"下一页会话明细"}));
    await screen.findByText("第 2 / 3 页");expect(document.querySelectorAll(".du-session-row")).toHaveLength(5);
    fireEvent.click(screen.getByRole("button",{name:"下一页会话明细"}));
    await screen.findByText("第 3 / 3 页");expect(document.querySelectorAll(".du-session-row")).toHaveLength(1);
    expect(screen.getByRole("button",{name:"下一页会话明细"})).toBeDisabled();
  });
  it("combines time, membership, project and conversation filters without changing KPIs", async () => {
    render(<Dashboard />);await screen.findByRole("heading",{name:"最近执行明细"});
    fireEvent.click(screen.getByRole("button",{name:"下一页会话明细"}));await screen.findByText("第 2 / 3 页");
    fireEvent.change(screen.getByLabelText("筛选最近执行的时间范围"),{target:{value:"24"}});
    await screen.findByText("第 1 / 2 页");
    fireEvent.change(screen.getByLabelText("筛选最近执行的项目"),{target:{value:"Codex Usage"}});
    await screen.findByText("共 2 条 · 显示 1–2 条");
    fireEvent.change(screen.getByLabelText("筛选最近执行的对话"),{target:{value:"Codex Usage-thread-0"}});
    await screen.findByText("共 1 条 · 显示 1–1 条");
    expect(screen.getByText("3.84")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("筛选是否属于项目"),{target:{value:"projectless"}});
    await screen.findByRole("button",{name:"查看会话详情：独立对话"});
    expect(screen.getByLabelText("筛选最近执行的项目")).toBeDisabled();
    fireEvent.click(screen.getByRole("button",{name:"清除筛选"}));
    await screen.findByText("第 1 / 3 页");
  });
  it("preserves selected cycles when the history range changes", async () => {
    render(<Dashboard />);await screen.findByRole("heading",{name:"最近执行明细"});
    fireEvent.change(screen.getByLabelText("选择额度重置周期"),{target:{value:"demo-1"}});
    await waitFor(()=>expect(screen.getByLabelText("选择额度重置周期")).toHaveValue("demo-1"));
    fireEvent.click(screen.getByRole("button",{name:"7 天"}));await screen.findByText("26.88");
    expect(screen.getByLabelText("选择额度重置周期")).toHaveValue("demo-1");
  });
  it("shows an empty filtered state and keeps the project selection", async () => {
    render(<Dashboard />);await screen.findByRole("heading",{name:"最近执行明细"});
    fireEvent.change(screen.getByLabelText("筛选最近执行的项目"),{target:{value:"个人网站"}});
    await screen.findByText("共 4 条 · 显示 1–4 条");
    fireEvent.change(screen.getByLabelText("筛选最近执行的时间范围"),{target:{value:"1"}});
    await screen.findByText("没有符合筛选条件的会话，可调整或清除筛选。");
    expect(screen.getByLabelText("筛选最近执行的项目")).toHaveValue("个人网站");
    expect(screen.getByRole("button",{name:"下一页会话明细"})).toBeDisabled();
  });
  it("handles empty local history without invented sessions", async () => {
    const { fetchAnalytics }=await import("../lib/analytics");
    vi.mocked(fetchAnalytics).mockImplementationOnce(async q=>({...demoDashboard(q),cycles:[],cycle:null,sessions:[],sessionPage:{page:1,pageSize:5,total:0,totalPages:1}}));
    render(<Dashboard />);await screen.findByText("暂无可确认周期");
    expect(screen.getByText("暂无已完成执行的会话")).toBeInTheDocument();
  });
  it("reports child-window failures instead of silently doing nothing", async () => {
    vi.mocked(openConversation).mockRejectedValueOnce(new Error("blocked"));
    render(<Dashboard />);fireEvent.click(await screen.findByRole("button",{name:"查看会话详情：Codex Usage · 界面优化"}));
    expect(await screen.findByRole("alert")).toHaveTextContent("会话详情未能打开");
  });
  it("keeps history browsable during an explicit price update and passes its refresh flag", async () => {
    const { fetchAnalytics } = await import("../lib/analytics");
    render(<Dashboard/>);
    await screen.findByRole("heading", { name: "历史时段用量" });
    let finish!: (data: DashboardData) => void;
    vi.mocked(fetchAnalytics).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    fireEvent.click(screen.getByRole("button", { name: "立即更新 API 价格" }));
    expect(fetchAnalytics).toHaveBeenLastCalledWith(expect.objectContaining({ refreshPricing: true }));
    expect(within(screen.getByRole("region", { name: "最近执行明细" })).getByRole("table")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "下一页会话明细" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "立即更新 API 价格" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "刷新使用大盘" })).toBeDisabled();
    finish(demoDashboard({ hours: 24, cycleId: null, project: null, billingStart: 0 }));
    await waitFor(() => expect(screen.getByRole("button", { name: "立即更新 API 价格" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "刷新使用大盘" }));
    expect(fetchAnalytics).toHaveBeenLastCalledWith(expect.objectContaining({ refreshPricing: true }));
    await waitFor(() => expect(screen.getByRole("button", { name: "刷新使用大盘" })).toBeEnabled());
  });
  it("preserves existing usage and reports the reason when a price update request fails", async () => {
    const { fetchAnalytics } = await import("../lib/analytics");
    render(<Dashboard/>);
    await screen.findByRole("heading", { name: "历史时段用量" });
    vi.mocked(fetchAnalytics).mockRejectedValueOnce(new Error("连接已断开"));
    fireEvent.click(screen.getByRole("button", { name: "立即更新 API 价格" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("价格更新请求失败，已保留上次单价和统计结果。连接已断开");
    expect(screen.getByText("3.84")).toBeInTheDocument();
  });
  it("displays Standard historical totals without an approximation prefix", async () => {
    const { fetchAnalytics } = await import("../lib/analytics");
    vi.mocked(fetchAnalytics).mockImplementationOnce(async q => {
      const data = demoDashboard(q);
      return { ...data, history: { ...data.history, assumedStandardTokens: data.history.totalTokens, serviceTiers: [{ tier: "standard", tokens: data.history.totalTokens, costUsd: data.history.costUsd, unpricedTokens: 0, assumedStandardTokens: data.history.totalTokens }] }, billing: { ...data.billing, assumedStandardTokens: data.billing.totalTokens, serviceTiers: [{ tier: "standard", tokens: data.billing.totalTokens, costUsd: data.billing.costUsd, unpricedTokens: 0, assumedStandardTokens: data.billing.totalTokens }] } };
    });
    render(<Dashboard/>);
    await screen.findByRole("heading", { name: "本账期基准等价差额" });
    expect(screen.getByText("$348.42")).toBeInTheDocument();
    expect(screen.queryByText(/模式未记录|Fast 附加费用未确认/)).not.toBeInTheDocument();
    expect(screen.getAllByText(/已归入 Standard/)).toHaveLength(2);
    for (const note of screen.getAllByText(/已归入 Standard/)) expect(note).not.toBeVisible();
    expect(document.body.textContent).not.toContain("≈");
    expect(screen.getByText("费用 未完整")).toHaveAttribute("title", "部分历史记录已按 Standard 归类，费用为估算，无法完整比较");
    expect(screen.queryByText(/只展示已知数据的估算下限/)).not.toBeInTheDocument();
  });
  it("asks new users to configure billing without inventing subscription costs", async () => {
    vi.mocked(getBillingSettings).mockResolvedValueOnce(null);
    render(<Dashboard/>);
    await screen.findByText("填写账期设置后查看比较");
    expect(screen.getByText("尚未设置个人账期")).toBeInTheDocument();
    expect(screen.getByLabelText("月订阅费（USD）")).toHaveValue(null);
    expect(screen.getByLabelText("每月扣款日")).toHaveValue(null);
  });
});
