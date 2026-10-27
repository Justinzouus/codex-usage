import { ArrowClockwise, ArrowDown, ArrowUp, ChartBar, ClockCounterClockwise, CloudSlash, SignIn, WarningCircle } from "@phosphor-icons/react";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { clampPercent, formatDateTime, formatResetDate, formatResetTime, formatUpdatedTime, quotaTier } from "../lib/format";
import { copy, normalizeLanguage } from "../lib/i18n";
import type { Language, ProviderSnapshot, WidgetPreferences } from "../types";

interface Props {
  snapshot: ProviderSnapshot;
  preferences: WidgetPreferences;
  providerCount: number;
  onPrevious: () => void;
  onNext: () => void;
  onLanguage: () => void;
  onRefresh?: () => void;
  onDashboard?: () => void;
  isRefreshing?: boolean;
  isConsuming?: boolean;
  notice?: string | null;
  initialShowCreditTip?: boolean;
}

function StatusIcon({ status, expired = false }: { status: ProviderSnapshot["status"]; expired?: boolean }) {
  if (status === "signed_out") return <SignIn weight="duotone" />;
  if (status === "stale" || expired) return <ClockCounterClockwise weight="duotone" />;
  if (status === "unavailable") return <CloudSlash weight="duotone" />;
  return <WarningCircle weight="duotone" />;
}

function localizedBackendMessage(message: string | null, language: Language): string | null {
  if (!message) return null;
  if (language === "en") return message;
  const normalized = message.toLowerCase();
  if (normalized.includes("sign in") || normalized.includes("login")) return "Codex 登录已失效，请重新登录。";
  if (normalized.includes("rate limited")) return "请求过于频繁，将稍后自动重试。";
  if (normalized.includes("network")) return "网络不可用，将自动重试。";
  if (normalized.includes("format")) return "额度响应格式已变化。";
  if (normalized.includes("missing the 5h")) return "额度响应缺少 5 小时窗口。";
  if (normalized.includes("primary quota window")) return "Codex 没有返回主要额度窗口。";
  if (normalized.includes("cli was not found")) return "未找到 Codex，请先安装 Codex 或设置 CODEX_CLI_PATH。";
  if (normalized.includes("does not expose quota")) return "当前 Codex 版本不支持读取额度，请先更新 Codex。";
  if (normalized.includes("timed out") || normalized.includes("too long")) return "读取额度超时，将自动重试。";
  if (normalized.includes("could not be started")) return "无法启动 Codex，请尝试更新或重新安装。";
  if (normalized.includes("refresh is already running")) return "额度正在刷新，请稍候。";
  return message;
}

export const QuotaCard = memo(function QuotaCard({
  snapshot,
  preferences,
  providerCount,
  onPrevious,
  onNext,
  onLanguage,
  onRefresh,
  onDashboard,
  isRefreshing = false,
  isConsuming = false,
  notice = null,
  initialShowCreditTip = false,
}: Props) {
  const [showCreditTip, setShowCreditTip] = useState(initialShowCreditTip);
  const resetCreditRowRef = useRef<HTMLDivElement>(null);
  const resetCreditTipRef = useRef<HTMLDivElement>(null);
  const language = normalizeLanguage(preferences.language);
  const t = copy[language];
  const short = snapshot.shortWindow ? clampPercent(snapshot.shortWindow.remainingPercent) : null;
  const weekly = snapshot.weeklyWindow ? clampPercent(snapshot.weeklyWindow.remainingPercent) : null;
  const staleAge = Date.now() - new Date(snapshot.updatedAt).getTime();
  const staleExpired = snapshot.status === "stale" && staleAge > 30 * 60_000;
  const available = snapshot.status === "ok" || (snapshot.status === "stale" && !staleExpired);
  const shortWindowLabel = short === null ? t.shortWindowInactive : t.availableLabel(short);
  const weeklyWindowLabel = weekly === null ? t.weeklyWindowUnavailable : t.weeklyAvailableLabel(weekly);
  const tier = quotaTier(weekly);
  const indicatorState = isRefreshing ? "refreshing" : isConsuming ? "active" : snapshot.status === "ok" ? "ok" : snapshot.status === "stale" ? "stale" : "error";
  const indicatorLabel = isRefreshing
    ? t.refreshing
    : isConsuming
      ? t.active
    : snapshot.status === "ok"
      ? t.dataSynced
      : snapshot.status === "stale"
        ? t.dataStale
        : snapshot.status === "signed_out"
          ? t.notSignedIn
          : t.unavailableStatus;
  const message = localizedBackendMessage(snapshot.message, language);
  const creditExpirations = useMemo(() => (snapshot.resetCreditExpiresAt ?? []).map((value, index) => {
    return t.creditItem(index, formatDateTime(value, language));
  }), [language, snapshot.resetCreditExpiresAt, t]);

  useEffect(() => {
    if (!showCreditTip) return;

    const closeOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (resetCreditRowRef.current?.contains(target) || resetCreditTipRef.current?.contains(target)) return;
      setShowCreditTip(false);
    };
    const closeOnWindowBlur = () => setShowCreditTip(false);

    document.addEventListener("pointerdown", closeOnOutsidePointer, true);
    window.addEventListener("blur", closeOnWindowBlur);
    return () => {
      document.removeEventListener("pointerdown", closeOnOutsidePointer, true);
      window.removeEventListener("blur", closeOnWindowBlur);
    };
  }, [showCreditTip]);

  return (
    <main className={`quota-card quota-card--${snapshot.status} quota-card--${tier}`}>
      <div className="aurora" aria-hidden="true" />
      <span className="sr-only" aria-live="polite">{available ? weeklyWindowLabel : message}</span>
      {notice ? <p className="operation-notice" role="status">{notice}</p> : null}
      <header className="card-header">
        <div>
          <p className="eyebrow">CODEX · USAGE</p>
          {snapshot.status !== "stale" ? <p className="updated">{t.weeklyRemaining}</p> : null}
        </div>
        <nav className="card-actions" aria-label={t.controls}>
            <div className="card-action-row card-action-row--status-refresh">
              <span className={`usage-indicator usage-indicator--${indicatorState}`} role="status" aria-label={indicatorLabel} title={indicatorLabel}><i /></span>
              <button className={`refresh-button${isRefreshing ? " is-refreshing" : ""}`} onClick={onRefresh} disabled={!onRefresh || isRefreshing} aria-label={t.refreshQuota} title={t.refreshQuota}>
                <ArrowClockwise />
              </button>
            </div>
            <div className="card-action-row card-action-row--language">
              <button className="language-button" onClick={onLanguage} aria-label={t.switchLanguage} title={t.switchLanguage}>{language === "en" ? "中" : "EN"}</button>
            </div>
            {onDashboard ? <div className="card-action-row"><button onClick={onDashboard} aria-label={language === "en" ? "Open usage dashboard" : "打开使用大盘"} title={language === "en" ? "Usage dashboard" : "使用大盘"}><ChartBar /></button></div> : null}
            {providerCount > 1 ? (
              <div className="card-action-row card-action-row--providers">
                <button onClick={onPrevious} aria-label={t.servicePrevious}><ArrowUp /></button>
                <button onClick={onNext} aria-label={t.serviceNext}><ArrowDown /></button>
              </div>
            ) : null}
        </nav>
      </header>

      {available ? (
        <>
          <section className={`primary-metric${weekly === null ? " primary-metric--placeholder" : ""}`} aria-label={weeklyWindowLabel}>
            <span>{weekly ?? "--"}</span>{weekly === null ? null : <small>%</small>}
          </section>
          {weekly === null ? (
            <div className="progress progress--placeholder" aria-hidden="true" />
          ) : (
            <div className="progress" role="progressbar" aria-label={weeklyWindowLabel} aria-valuemin={0} aria-valuemax={100} aria-valuenow={weekly}>
              <span style={{ width: `${weekly}%` }} />
            </div>
          )}
          <p className={`reset-time${weekly === null ? " reset-time--placeholder" : ""}`}>{weekly === null ? t.weeklyWindowUnavailable : t.weeklyUntil(formatResetDate(snapshot.weeklyWindow?.resetsAt ?? null, language))}</p>
          <footer className="card-footer">
            <div className="secondary-metric">
              <p>{t.shortRemaining}</p>
              <div className="secondary-quota" aria-label={shortWindowLabel}>
                <strong>{short ?? "--"}<small>{short === null ? "" : "%"}</small></strong>
                <span className="secondary-detail">{short === null ? t.shortWindowInactive : formatResetTime(snapshot.shortWindow?.resetsAt ?? null, new Date(), language)}</span>
              </div>
              <div ref={resetCreditRowRef} className="reset-credit-row" onMouseDown={(event) => event.stopPropagation()}>
                <span>{snapshot.resetCredits === null ? t.resetCreditUnknown : t.resetCredits(snapshot.resetCredits)}</span>
                {snapshot.resetCredits !== null && snapshot.resetCredits > 0 ? (
                  <button type="button" className="reset-credit-button" onClick={() => setShowCreditTip((value) => !value)} aria-expanded={showCreditTip} aria-label={t.view}>{t.view}</button>
                ) : null}
              </div>
              {showCreditTip ? (
                <div ref={resetCreditTipRef} className="reset-credit-tip" role="status" onMouseDown={(event) => event.stopPropagation()}>
                  {creditExpirations.length > 0 ? creditExpirations.map((item) => <p key={item}>{item}</p>) : <p>{t.noCreditExpiration}</p>}
                </div>
              ) : null}
            </div>
            <div className="provider-meta">
              <span>{t.syncedAt(formatUpdatedTime(snapshot.updatedAt, language))}</span>
            </div>
          </footer>
        </>
      ) : (
        <section className="error-state" aria-live="polite">
          <div className="status-icon" aria-hidden="true"><StatusIcon status={snapshot.status} expired={staleExpired} /></div>
          <strong>{snapshot.status === "signed_out" ? t.signedInRequired : staleExpired ? t.staleExpired : t.temporarilyUnavailable}</strong>
          <p>{message ?? t.errorUnavailable}</p>
          {snapshot.status === "stale" ? (
            <button type="button" className="error-refresh-button" onMouseDown={(event) => event.stopPropagation()} onClick={onRefresh} disabled={!onRefresh} aria-label={t.refreshQuota}>
              <ArrowClockwise />
              <span>{t.refresh}</span>
            </button>
          ) : null}
        </section>
      )}
    </main>
  );
});
