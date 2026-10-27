// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import UsageTrend, { grainLabel } from "./UsageTrend";
import { demoDashboard, fetchAnalytics, type AnalyticsQuery } from "../lib/analytics";
vi.mock("../lib/analytics",async original=>{const actual=await original<typeof import("../lib/analytics")>();return {...actual,fetchAnalytics:vi.fn(async(q:AnalyticsQuery)=>actual.demoDashboard(q))};});
beforeAll(()=>vi.stubGlobal("ResizeObserver",class{observe(){}disconnect(){}}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
const fixture=()=>demoDashboard({hours:24,cycleId:null,project:null,billingStart:0});
describe("scrollable usage trend",()=>{
  it("keeps a hovered tooltip during refresh but clears it when the granularity changes",()=>{
    const data=fixture();const {rerender}=render(<UsageTrend data={data} hours={24} metric="tokens"/>);
    fireEvent.mouseEnter(screen.getAllByRole("button",{name:/查看趋势点/})[0]);
    rerender(<UsageTrend data={{...data,trend:data.trend.map(b=>({...b,start:b.start+60,end:b.end+60}))}} hours={24} metric="tokens"/>);
    expect(screen.getByRole("tooltip")).toBeInTheDocument();
    rerender(<UsageTrend data={demoDashboard({hours:1,cycleId:null,project:null,billingStart:0})} hours={1} metric="tokens"/>);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
  it("returns to the newest position even when no new data array is needed",()=>{
    render(<UsageTrend data={fixture()} hours={24} metric="tokens"/>);
    const scroll=screen.getByRole("region",{name:"可横向滚动的用量趋势"});
    Object.defineProperty(scroll,"scrollWidth",{value:2000,configurable:true});
    Object.defineProperty(scroll,"clientWidth",{value:500,configurable:true});
    scroll.scrollLeft=200;fireEvent.scroll(scroll);
    fireEvent.click(screen.getByRole("button",{name:"趋势回到最新"}));
    expect(scroll.scrollLeft).toBe(1500);
  });
  it("shows the selected point's token total and API cost on hover and keyboard focus",()=>{
    const data=fixture();
    data.trend[0].metrics={...data.trend[0].metrics,totalTokens:1234567,costUsd:12.34};
    data.trend[1].metrics={...data.trend[1].metrics,totalTokens:89000,costUsd:.56};
    render(<UsageTrend data={data} hours={24} metric="tokens"/>);
    const [hovered,focused]=screen.getAllByRole("button",{name:/查看趋势点/});
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    fireEvent.mouseEnter(hovered);
    const tip=within(screen.getByRole("tooltip"));
    expect(tip.getByText("消耗 Token").parentElement).toHaveTextContent("1.23 M");
    expect(tip.getByText("API 等价费用").parentElement).toHaveTextContent("$12.34");
    fireEvent.mouseLeave(document.querySelector(".du-trend-stage")!);expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    fireEvent.focus(focused);
    const focusedTip=within(screen.getByRole("tooltip"));
    expect(focusedTip.getByText("消耗 Token").parentElement).toHaveTextContent("89 K");
    expect(focusedTip.getByText("API 等价费用").parentElement).toHaveTextContent("$0.56");
    fireEvent.keyDown(focused,{key:"Escape"});expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
  it("loads older contiguous data without changing the selected granularity",async()=>{
    const data=fixture();render(<UsageTrend data={data} hours={24} metric="tokens"/>);
    fireEvent.click(screen.getByRole("button",{name:"更早记录"}));
    await waitFor(()=>expect(fetchAnalytics).toHaveBeenCalledWith(expect.objectContaining({hours:24,trendBefore:data.trend[0].start})));
    await waitFor(()=>expect(screen.getByRole("button",{name:"更早记录"})).not.toBeDisabled());
    expect(screen.getByText("每点 2 小时")).toBeInTheDocument();
    expect(screen.getAllByRole("button",{name:/查看趋势点/}).length).toBeLessThan(20);
  });
  it("stops at the earliest history and distinguishes unpriced cost",()=>{
    const data=fixture();data.trendHasOlder=false;data.trend.forEach(b=>{b.metrics.unpricedTokens=100;b.metrics.costUsd=0;});
    render(<UsageTrend data={data} hours={24} metric="cost"/>);
    expect(screen.getByRole("button",{name:"已到最早记录"})).toBeDisabled();
    fireEvent.mouseEnter(screen.getAllByRole("button",{name:/查看趋势点/})[0]);
    expect(screen.getByRole("tooltip")).toHaveTextContent("暂无法估价");
    expect(screen.getByText(/费用曲线仅含已知价格/)).toBeInTheDocument();
    expect(grainLabel(300)).toBe("5 分钟");expect(grainLabel(172800)).toBe("2 天");
  });
});
