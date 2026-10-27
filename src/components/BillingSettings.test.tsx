// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it, vi } from "vitest";
import BillingSettings from "./BillingSettings";
import { getBillingSettings } from "../lib/billing";

afterEach(() => { cleanup(); localStorage.clear(); });

describe("local billing settings", () => {
  it("starts without a personal baseline and persists the user's own settings", async () => {
    expect(await getBillingSettings()).toBeNull();
    const onSave = vi.fn();
    render(<BillingSettings value={null} onSave={onSave}/>);
    fireEvent.click(screen.getByText("账期设置"));
    fireEvent.change(screen.getByLabelText("月订阅费（USD）"), { target: { value: "20" } });
    fireEvent.change(screen.getByLabelText("本账期额外额度（USD）"), { target: { value: "5" } });
    fireEvent.change(screen.getByLabelText("每月扣款日"), { target: { value: "31" } });
    fireEvent.change(screen.getByLabelText("账期时区"), { target: { value: "UTC" } });
    fireEvent.click(screen.getByRole("button", { name: "保存账期设置" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith({ monthlyUsd: 20, extraCreditsUsd: 5, day: 31, timeZone: "UTC" }));
    expect(await getBillingSettings()).toEqual({ monthlyUsd: 20, extraCreditsUsd: 5, day: 31, timeZone: "UTC" });
  });

  it("keeps unsaved values and reports storage failure without applying them", async () => {
    const onSave = vi.fn();
    vi.spyOn(Storage.prototype, "setItem").mockImplementationOnce(() => { throw new Error("本地存储不可用"); });
    render(<BillingSettings value={{ monthlyUsd: 20, extraCreditsUsd: 0, day: 1, timeZone: "UTC" }} onSave={onSave}/>);
    fireEvent.click(screen.getByText("账期设置"));
    fireEvent.change(screen.getByLabelText("月订阅费（USD）"), { target: { value: "40" } });
    fireEvent.click(screen.getByRole("button", { name: "保存账期设置" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("本地存储不可用");
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByLabelText("月订阅费（USD）")).toHaveValue(40);
    vi.restoreAllMocks();
  });
});
