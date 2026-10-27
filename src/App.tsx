import { useCallback, useEffect, useRef, useState } from "react";
import { QuotaCard } from "./components/QuotaCard";
import { fetchSnapshots, getPreferences, hidePopover, listenDesktopEvents, openDashboard, updatePreferences } from "./lib/bridge";
import { copy, nextLanguage, normalizeLanguage } from "./lib/i18n";
import { mergeSnapshots } from "./lib/snapshots";
import type { ProviderSnapshot, WidgetPreferences } from "./types";

const DEFAULT_PREFS: WidgetPreferences = { locked: false, alwaysOnTop: true, pinnedProvider: null, autoRotateSeconds: 12, language: "zh-CN" };

export default function App() {
  const [snapshots, setSnapshots] = useState<ProviderSnapshot[]>([]);
  const [preferences, setPreferences] = useState(DEFAULT_PREFS);
  const [activeIndex, setActiveIndex] = useState(0);
  const [consumingProviders, setConsumingProviders] = useState<Set<string>>(() => new Set());
  const [refreshing, setRefreshing] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const previousPrimary = useRef(new Map<string, number>());
  const consumptionTimers = useRef(new Map<string, number>());
  const refreshInFlight = useRef<Promise<void> | null>(null);
  const language = normalizeLanguage(preferences.language);
  const t = copy[language];

  const applySnapshots = useCallback((values: ProviderSnapshot[]) => {
    for (const item of values) {
      const nextPrimary = item.weeklyWindow?.remainingPercent ?? item.shortWindow?.remainingPercent;
      const previous = previousPrimary.current.get(item.provider);
      if (nextPrimary !== undefined && previous !== undefined && nextPrimary < previous) {
        setConsumingProviders((current) => new Set(current).add(item.provider));
        const oldTimer = consumptionTimers.current.get(item.provider);
        if (oldTimer !== undefined) window.clearTimeout(oldTimer);
        const timer = window.setTimeout(() => {
          setConsumingProviders((current) => { const next = new Set(current); next.delete(item.provider); return next; });
          consumptionTimers.current.delete(item.provider);
        }, 5 * 60_000);
        consumptionTimers.current.set(item.provider, timer);
      }
      if (nextPrimary !== undefined) previousPrimary.current.set(item.provider, nextPrimary);
    }
    setSnapshots((current) => mergeSnapshots(current, values));
  }, []);

  const refresh = useCallback((force = false): Promise<void> => {
    if (refreshInFlight.current) return refreshInFlight.current;
    setRefreshing(true);
    const task = (async () => {
      try {
        applySnapshots(await fetchSnapshots(force));
      } catch {
        setSnapshots((current) => current.length > 0
          ? current.map((item) => ({ ...item, status: "stale", message: "Refresh failed. Please try again later." }))
          : [{ provider: "codex", displayName: "CODEX", plan: null, shortWindow: null, weeklyWindow: null, resetCredits: null, resetCreditExpiresAt: [], updatedAt: new Date().toISOString(), status: "unavailable", message: "Quota is temporarily unavailable. It will retry automatically." }]);
      }
    })().finally(() => {
      if (refreshInFlight.current === task) refreshInFlight.current = null;
      setRefreshing(false);
    });
    refreshInFlight.current = task;
    return task;
  }, [applySnapshots]);

  useEffect(() => {
    void refresh();
    void getPreferences().then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) })).catch(() => setOperationError("Unable to read settings. Defaults are in use."));
    return () => { for (const timer of consumptionTimers.current.values()) window.clearTimeout(timer); consumptionTimers.current.clear(); };
  }, [refresh]);

  useEffect(() => {
    let cancelled = false;
    let cleanup: () => void = () => {};
    void listenDesktopEvents({
      onPreferences: (value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }),
      onRefresh: () => void refresh(true),
      onSnapshots: applySnapshots,
    }).then((value) => {
      if (cancelled) value(); else cleanup = value;
    }).catch(() => setOperationError("Desktop event listener failed to start."));
    return () => { cancelled = true; cleanup(); };
  }, [applySnapshots, refresh]);

  useEffect(() => {
    const refreshWhenActive = () => { if (document.visibilityState === "visible") void refresh(true); };
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") void hidePopover(); };
    window.addEventListener("focus", refreshWhenActive);
    window.addEventListener("keydown", closeOnEscape);
    document.addEventListener("visibilitychange", refreshWhenActive);
    return () => {
      window.removeEventListener("focus", refreshWhenActive);
      window.removeEventListener("keydown", closeOnEscape);
      document.removeEventListener("visibilitychange", refreshWhenActive);
    };
  }, [refresh]);

  useEffect(() => {
    if (preferences.pinnedProvider || snapshots.length < 2) return;
    const id = window.setInterval(() => setActiveIndex((value) => (value + 1) % snapshots.length), preferences.autoRotateSeconds * 1000);
    return () => window.clearInterval(id);
  }, [preferences.autoRotateSeconds, preferences.pinnedProvider, snapshots.length]);

  const current = preferences.pinnedProvider
    ? snapshots.find((item) => item.provider === preferences.pinnedProvider) ?? snapshots[0]
    : snapshots[activeIndex % Math.max(1, snapshots.length)];

  const savePreferences = useCallback((next: WidgetPreferences) => {
    const previous = preferences;
    setPreferences(next);
    setOperationError(null);
    void updatePreferences(next).catch(() => { setPreferences(previous); setOperationError("Settings could not be saved. Previous state restored."); });
  }, [preferences]);

  if (!current) return <div className="loading-card" aria-label={t.loadingQuota}><span /><span /><span /></div>;

  return (
    <QuotaCard
      snapshot={current}
      preferences={preferences}
      providerCount={snapshots.length}
      onPrevious={() => setActiveIndex((value) => (value - 1 + snapshots.length) % snapshots.length)}
      onNext={() => setActiveIndex((value) => (value + 1) % snapshots.length)}
      onLanguage={() => savePreferences({ ...preferences, language: nextLanguage(language) })}
      onRefresh={() => refresh(true)}
      onDashboard={() => void openDashboard().catch(() => setOperationError(language === "en" ? "Unable to open dashboard." : "无法打开使用大盘，请重试。"))}
      isRefreshing={refreshing}
      isConsuming={consumingProviders.has(current.provider)}
      notice={operationError}
    />
  );
}
