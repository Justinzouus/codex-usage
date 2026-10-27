// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
import ConversationDetails from "./ConversationDetails";
import { fetchAnalytics, type AnalyticsQuery } from "../lib/analytics";
vi.mock("../lib/analytics",async original=>{const actual=await original<typeof import("../lib/analytics")>();return {...actual,fetchAnalytics:vi.fn(async(q:AnalyticsQuery)=>actual.demoDashboard(q))};});
afterEach(()=>{cleanup();vi.clearAllMocks();});
describe("conversation details window",()=>{
  it("loads all history for only the selected conversation and paginates executions",async()=>{
    render(<ConversationDetails threadId="Codex Usage-thread-0"/>);
    const task=await screen.findByRole("button",{name:"优化周额度详情卡"});
    expect(document.querySelectorAll(".du-task-row")).toHaveLength(5);
    expect(screen.getByText("816.4 K")).toBeInTheDocument();
    expect(screen.getByText("$4.99")).toBeInTheDocument();
    expect(fetchAnalytics).toHaveBeenCalledWith(expect.objectContaining({threadId:"Codex Usage-thread-0",page:1,project:null}));
    expect(vi.mocked(fetchAnalytics).mock.calls[0][0].executionHours).toBeUndefined();
    fireEvent.click(task);expect(screen.getByText("输出（已含推理）")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button",{name:"下一页会话执行"}));
    await screen.findByText("第 2 / 2 页");
    expect(document.querySelectorAll(".du-task-row")).toHaveLength(2);
    expect(screen.queryByText("输出（已含推理）")).not.toBeInTheDocument();
    expect(screen.getByText("816.4 K")).toBeInTheDocument();
    expect(screen.getByRole("button",{name:"下一页会话执行"})).toBeDisabled();
  });
  it("does not display another conversation when the requested thread is missing",async()=>{
    render(<ConversationDetails threadId="missing"/>);
    await screen.findByText("这个会话暂无已完成的执行记录。");
    expect(document.querySelectorAll(".du-task-row")).toHaveLength(0);
  });
  it("does not query all history for an empty thread identifier",async()=>{
    render(<ConversationDetails threadId=""/>);
    await screen.findByRole("alert");expect(fetchAnalytics).not.toHaveBeenCalled();
  });
});
