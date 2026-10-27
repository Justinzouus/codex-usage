// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it } from "vitest";
import type { ProviderSnapshot, WidgetPreferences } from "../types";
import { QuotaCard } from "./QuotaCard";

const preferences: WidgetPreferences = {
  locked: false,
  alwaysOnTop: true,
  pinnedProvider: "codex",
  autoRotateSeconds: 12,
  language: "zh-CN",
};

const baseSnapshot: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "PRO",
  shortWindow: { remainingPercent: 64, resetsAt: "2026-07-13T16:00:00+09:00", windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 80, resetsAt: "2026-07-20T00:00:00+09:00", windowSeconds: 604_800 },
  resetCredits: 0,
  updatedAt: "2026-07-13T12:00:00+09:00",
  status: "ok",
  message: null,
};

afterEach(cleanup);

describe("QuotaCard quota hierarchy", () => {
  it("shows weekly quota as the primary metric and keeps 5-hour quota secondary", () => {
    render(
      <QuotaCard
        snapshot={baseSnapshot}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onLanguage={() => {}}
      />,
    );

    expect(screen.getByText("本周剩余")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "周额度剩余 80%" })).toHaveTextContent("80%");
    expect(screen.getByRole("progressbar", { name: "周额度剩余 80%" })).toHaveAttribute("aria-valuenow", "80");
    expect(screen.getByLabelText("5 小时额度剩余 64%")).toHaveTextContent("64%");
  });

  it("keeps the weekly quota primary while the 5-hour limit is inactive", () => {
    render(
      <QuotaCard
        snapshot={{ ...baseSnapshot, shortWindow: null }}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onLanguage={() => {}}
      />,
    );

    expect(screen.getByRole("region", { name: "周额度剩余 80%" })).toHaveTextContent("80%");
    expect(screen.getByLabelText("5 小时额度当前未启用")).toHaveTextContent("--");
    expect(screen.getByText("CODEX · USAGE")).toBeInTheDocument();
    expect(screen.getByText("5 小时剩余")).toBeInTheDocument();
  });

  it("does not substitute the 5-hour value when weekly quota is unavailable", () => {
    const { container } = render(
      <QuotaCard
        snapshot={{ ...baseSnapshot, weeklyWindow: null }}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onLanguage={() => {}}
      />,
    );

    expect(screen.getByLabelText("周额度当前不可用")).toHaveTextContent("--");
    expect(screen.getByLabelText("5 小时额度剩余 64%")).toHaveTextContent("64%");
    expect(container.querySelector(".progress--placeholder")).toBeInTheDocument();
  });
});

describe("QuotaCard action layout", () => {
  it("places status and refresh together, followed by language", () => {
    const { container } = render(
      <QuotaCard
        snapshot={baseSnapshot}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onLanguage={() => {}}
        onRefresh={() => {}}
      />,
    );

    const rows = container.querySelectorAll(".card-action-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector(".usage-indicator")).toBeInTheDocument();
    expect(rows[0].querySelector(".refresh-button")).toBeInTheDocument();
    expect(rows[1].querySelector(".language-button")).toBeInTheDocument();
    expect(container.querySelector(".provider-meta")).toHaveTextContent("同步于");
    expect(container.querySelector(".provider-mark")).toBeNull();
  });
});

describe("QuotaCard reset credit details", () => {
  const snapshotWithCredits: ProviderSnapshot = {
    ...baseSnapshot,
    resetCredits: 2,
    resetCreditExpiresAt: ["2026-07-27T07:34:00+08:00", "2026-08-01T03:50:00+08:00"],
  };

  function renderCard() {
    return render(
      <QuotaCard
        snapshot={snapshotWithCredits}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onLanguage={() => {}}
      />,
    );
  }

  it("closes the details after clicking elsewhere in the card", () => {
    renderCard();

    fireEvent.click(screen.getByRole("button", { name: "查看" }));
    expect(screen.getByText(/第 1 次/)).toBeInTheDocument();

    fireEvent.pointerDown(screen.getByText("CODEX · USAGE"));
    expect(screen.queryByText(/第 1 次/)).not.toBeInTheDocument();
  });

  it("closes the details when the window loses focus", () => {
    renderCard();

    fireEvent.click(screen.getByRole("button", { name: "查看" }));
    expect(screen.getByText(/第 1 次/)).toBeInTheDocument();

    fireEvent.blur(window);
    expect(screen.queryByText(/第 1 次/)).not.toBeInTheDocument();
  });
});
