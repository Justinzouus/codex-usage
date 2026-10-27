import BillingSettingsEditor from "./BillingSettings";
import { getBillingSettings, billingTimeZoneLabel, type BillingSettings } from "../lib/billing";
import { ArrowClockwise, ChartBar, Coins, Database, Robot, Stack } from "@phosphor-icons/react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { billingStart, cacheRate, costText, dateTime, dollars, fetchAnalytics, formatTokens, hasAssumedStandardUsage, hasEstimatedServiceTier, shortDate, tokenText, type DashboardData, type Share } from "../lib/analytics";
import UsageTrend from "./UsageTrend";
import ExecutionHistory, { initialExecutionFilters } from "./ExecutionHistory";
import PricingStatus from "./PricingStatus";
import ServiceTierUsage from "./ServiceTierUsage";
import { isTauri } from "../lib/bridge";
import "../dashboard.css";

const ranges = [[1, "1 小时"], [12, "12 小时"], [24, "24 小时"], [168, "7 天"], [720, "30 天"]] as const;

function TokenValue({ value }: { value: number }) { const n = formatTokens(value); return <>{n.value}<span className="du-unit">{n.unit}</span></>; }
function ChangeIndicator({ label, current, previous, complete = true, incompleteReason = "部分 Token 暂无 API 单价，无法完整比较" }: { label: string; current: number; previous: number; complete?: boolean; incompleteReason?: string }) {
  if (!complete) return <span className="du-change du-change--unknown" title={incompleteReason}>{label} 未完整</span>;
  const direction = current > previous ? "up" : current < previous ? "down" : "flat";
  const change = previous === 0 ? (current > 0 ? "新增" : "0.0%") : `${(Math.abs(current - previous) / previous * 100).toFixed(1)}%`;
  const arrow = direction === "up" ? "↑" : direction === "down" ? "↓" : "—";
  const trend = direction === "up" ? "上升" : direction === "down" ? "下降" : "持平";
  return <span className={`du-change du-change--${direction}`} aria-label={`${label}${trend} ${change}`}><span>{label}</span><span className="du-change-arrow" aria-hidden="true">{arrow}</span><b>{change}</b></span>;
}
function Module({ number, title, subtitle, controls, children }: { number: string; title: string; subtitle: string; controls?: ReactNode; children: ReactNode }) {
  return <section className="du-module" aria-labelledby={`module-${number}`}><header className="du-section-head"><div className="du-heading-group"><span className="du-number" aria-hidden="true">{number}</span><div><h2 id={`module-${number}`}>{title}</h2><p>{subtitle}</p></div></div>{controls}</header>{children}</section>;
}
function Kpi({ label, icon, children, note }: { label: string; icon: ReactNode; children: ReactNode; note: string }) {
  return <div className="du-kpi"><div className="du-label">{icon}{label}</div><div className="du-value">{children}</div><div className="du-note">{note}</div></div>;
}
function Shares({ rows, attribution = false }: { rows: Share[]; attribution?: boolean }) {
  const unassigned = attribution ? rows.find(r => r.kind === "unassigned") : undefined;
  const visible = rows.length > 6 && unassigned ? [...rows.filter(r => r !== unassigned).slice(0, 4), unassigned] : rows.slice(0, 5);
  const hidden = rows.filter(r => !visible.includes(r));
  const shown: Share[] = rows.length > 6 ? [...visible, { id: "other", name: "其他", kind: "other", tokens: hidden.reduce((a, r) => a + r.tokens, 0), percent: hidden.reduce((a, r) => a + r.percent, 0) }] : rows;
  if (!shown.length) return <p className="du-empty">这个周期尚无可统计用量</p>;
  return <div className="du-share-list">{shown.map(r => {const label = attribution && r.kind === "project" ? `项目 · ${r.name}` : attribution && r.kind === "conversation" ? `对话 · ${r.name}` : r.name;return <div className="du-share-row" key={`${r.kind}:${r.id}`}><span className="du-share-name" title={label}>{label}</span><div className="du-share-track" title={`${label} · ${tokenText(r.tokens)} Token`}><span style={{ width: `${r.percent}%` }} /></div><span className="du-share-pct">{r.percent.toFixed(1)}%</span></div>;})}</div>;
}

export default function Dashboard() {
  const [billing, setBilling] = useState<BillingSettings | null>(null);
  const [billingReady, setBillingReady] = useState(false);
  const [billingError, setBillingError] = useState<string | null>(null);
  useEffect(() => { let active = true; void getBillingSettings().then(value => { if (active) setBilling(value); }).catch(() => { if (active) setBillingError("账期设置无法读取，请重新设置；未使用任何预设订阅金额。"); }).finally(() => { if (active) setBillingReady(true); }); return () => { active = false; }; }, []);
  const [hours, setHours] = useState(24), [cycleId, setCycleId] = useState<string | null>(null);
  const [executionFilters, setExecutionFilters] = useState(initialExecutionFilters);
  const [metric, setMetric] = useState<"tokens" | "cost">("tokens");
  const [data, setData] = useState<DashboardData | null>(null), [loading, setLoading] = useState(true), [error, setError] = useState<string | null>(null);
  const [pricingRefreshing, setPricingRefreshing] = useState(false);
  const request = useRef(0), mounted = useRef(true), hasData = useRef(false);
  const refresh = useCallback(async (refreshPricing = false) => {
    const id = ++request.current; setLoading(!refreshPricing || !hasData.current); setPricingRefreshing(refreshPricing);
    try { const value = await fetchAnalytics({ hours, cycleId, ...executionFilters, billingStart: billingStart(new Date(), billing ?? undefined), ...(refreshPricing ? { refreshPricing: true } : {}) }); if (mounted.current && id === request.current) { hasData.current = true; setData(value); setError(null); } }
    catch (cause) { if (mounted.current && id === request.current) setError(refreshPricing ? `价格更新请求失败，已保留上次单价和统计结果。${cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "请稍后重试。"}` : "历史统计暂时无法读取，已保留上次结果。请稍后重试。"); }
    finally { if (mounted.current && id === request.current) { setLoading(false); setPricingRefreshing(false); } }
  }, [hours, cycleId, executionFilters, billing]);
  useEffect(() => { mounted.current = true; document.documentElement.classList.add("dashboard-mode"); return () => { mounted.current = false; request.current++; document.documentElement.classList.remove("dashboard-mode"); }; }, []);
  useEffect(() => { if (!billingReady) return; void refresh(); const timer = window.setInterval(() => { if (document.visibilityState === "visible") void refresh(); }, 60_000); const focus = () => void refresh(); window.addEventListener("focus", focus); return () => { clearInterval(timer); window.removeEventListener("focus", focus); request.current++; }; }, [refresh, billingReady]);
  const range = ranges.find(r => r[0] === hours)?.[1] ?? "24 小时";
  const partial = !!data?.history.unpricedTokens;
  const savings = data && billing ? data.billing.costUsd - billing.monthlyUsd - billing.extraCreditsUsd : 0;
  const hasBillingCoverage = !!data?.coverage.firstEventAt && data.coverage.firstEventAt <= data.billingStart;
  const billingUnknownMode = !!data && hasEstimatedServiceTier(data.billing);
  const billingComplete = !!data && !billingUnknownMode && !data.billing.unpricedTokens && hasBillingCoverage && !data.coverage.unreadableFiles && !data.coverage.malformedLines;
  return <main className="usage-dashboard"><div className="du-chrome"><span className="du-wordmark">CODEX · USAGE</span>{!isTauri() && <span className="du-sync">浏览器预览 · 示例数据</span>}</div><div className="du-main"><header className="du-page-head"><div><h1>使用概览</h1><p>历史用量、重置周期与会话消耗，一目了然。</p></div><button className="du-refresh" onClick={() => void refresh(true)} disabled={loading || pricingRefreshing} aria-label="刷新使用大盘"><ArrowClockwise className={loading || pricingRefreshing ? "du-spinning" : ""}/>{loading || pricingRefreshing ? "读取中" : "刷新"}</button></header>
    {error && <div role="alert" className="du-notice du-error">{error}</div>}
    {!data ? <div className="du-loading" role="status"><ChartBar size={30}/><h2>{loading ? "正在整理本机使用记录…" : "暂未读取到使用记录"}</h2><p>首次读取可能需要一些时间，之后会增量更新。不会上传聊天记录。</p>{!loading && <button onClick={() => void refresh()}>重新读取</button>}</div> : <>
    <PricingStatus pricing={data.pricing} refreshing={pricingRefreshing} onRefresh={() => void refresh(true)}/>
    {data.coverage.files === 0 && <div className="du-notice" role="status">未发现本地 Codex 执行记录。使用 Codex 完成一轮执行后，点击刷新；其他设备或仅云端的记录不会自动计入。</div>}
    {data.coverage.unreadableFiles > 0 && <div className="du-notice">有 {data.coverage.unreadableFiles} 个文件未能读取，当前统计可能不完整。</div>}
    <div className="du-sections" aria-busy={loading}>
      <Module number="01" title="历史时段用量" subtitle="按固定时长回看使用情况" controls={<div className="du-segment du-range-control" aria-label="历史统计时间范围">{ranges.map(([n, label]) => <button type="button" key={n} aria-pressed={hours===n} onClick={() => setHours(n)}>{label}</button>)}</div>}>
        <div className="du-body"><div className="du-period-row"><span>{dateTime(data.start)} — {dateTime(data.end)}</span><span>全部项目 · 含子 Agent 用量</span></div><div className="du-kpis" aria-live="polite"><Kpi label="Token 总用量" icon={<Stack/>} note="输入已含缓存，未重复累加"><TokenValue value={data.history.totalTokens}/></Kpi><Kpi label="API 等价费用" icon={<Coins/>} note={partial ? `其中 ${tokenText(data.history.unpricedTokens)} Token 未定价` : "按可确认的 API 单价估算"}>{costText(data.history)}</Kpi><Kpi label="Agent 使用数" icon={<Robot/>} note={`主 Agent ${data.history.mainAgentCount} · 子 Agent ${data.history.subAgentCount}`}><span title="按所选时段内产生用量的 Agent 实例去重计数">{data.history.mainAgentCount+data.history.subAgentCount}<span className="du-unit">个</span></span></Kpi><Kpi label="输入缓存命中率" icon={<Database/>} note="缓存输入 / 全部输入">{cacheRate(data.history)}</Kpi></div><ServiceTierUsage metrics={data.history} scope="当前时段"/></div>
      </Module>
      <Module number="02" title="用量趋势与节约费用" subtitle="趋势可向左回看历史 · 节省按订阅账期估算"><div className="du-body du-trend-grid"><div className="du-trend-panel"><div className="du-panel-head"><div><h3>用量趋势</h3><span className="du-caption">默认回看最近 {range}</span></div><div className="du-segment du-small-toggle" aria-label="趋势指标"><button aria-pressed={metric==="tokens"} onClick={()=>setMetric("tokens")}>Token</button><button aria-pressed={metric==="cost"} onClick={()=>setMetric("cost")}>费用</button></div></div><UsageTrend data={data} metric={metric} hours={(data.end-data.start)/3600}/><div className="du-chart-summary"><span className="du-summary-period">最近 {range}</span><b className="du-summary-total">{tokenText(data.history.totalTokens)} Token <i>·</i> {costText(data.history)} API</b><span className="du-summary-change" title={`与前一个 ${range} 对比：${tokenText(data.comparisons.previous.totalTokens)} Token · ${costText(data.comparisons.previous)} API`}><span className="du-summary-change-label">环比</span><ChangeIndicator label="Token" current={data.history.totalTokens} previous={data.comparisons.previous.totalTokens}/><ChangeIndicator label="费用" current={data.history.costUsd} previous={data.comparisons.previous.costUsd} complete={!data.history.unpricedTokens&&!data.comparisons.previous.unpricedTokens&&!hasEstimatedServiceTier(data.history)&&!hasEstimatedServiceTier(data.comparisons.previous)} incompleteReason={data.history.unpricedTokens||data.comparisons.previous.unpricedTokens ? "部分 Token 暂无 API 单价，无法完整比较" : hasAssumedStandardUsage(data.history)||hasAssumedStandardUsage(data.comparisons.previous) ? "部分历史记录已按 Standard 归类，费用为估算，无法完整比较" : "部分记录只有请求模式，实际处理模式未提供，无法完整比较"}/></span></div></div><aside className="du-saving"><div className="du-saving-top"><h3>{!billing ? "账期对比" : billingUnknownMode ? "本账期基准等价差额" : billingComplete ? (savings >= 0 ? "本账期估算节省" : "本账期等价差额") : "本账期已知等价差额"}</h3><span className="du-caption">{billing ? `${billingTimeZoneLabel(billing.timeZone)} · 每月 ${billing.day} 日` : "尚未设置个人账期"}</span><div className="du-money-big">{!billing || data.billing.totalTokens===0 ? "—" : `${billingUnknownMode || billingComplete ? "" : "≥ "}${dollars(savings)}`}</div><span className="du-value-badge">{!billing ? "填写账期设置后查看比较" : data.billing.totalTokens===0 ? "本账期暂无可统计用量" : `等价价值 / 订阅费 ${(data.billing.costUsd/billing.monthlyUsd).toFixed(2)}×`}</span></div><div className="du-money-details"><div className="du-money-row"><span>API 等价费用</span><b>{costText(data.billing)}</b></div><div className="du-money-row"><span>订阅实付</span><b>{billing ? `− ${dollars(billing.monthlyUsd)}` : "待设置"}</b></div><div className="du-money-row"><span>额外额度购买</span><b>{billing ? dollars(billing.extraCreditsUsd) : "待设置"}</b></div></div><ServiceTierUsage metrics={data.billing} scope="本账期">{!billingComplete&&<p>{data.billing.unpricedTokens>0?`${tokenText(data.billing.unpricedTokens)} Token 暂无价格。`:""}{!hasBillingCoverage?"仅统计本机已记录的账期用量。":""}</p>}<p>本账期等价差额 = API 等价费用 − 订阅实付 − 额外额度购买。API 等价价值用于比较，不代表实际扣款。</p></ServiceTierUsage>{billingError && <p role="alert">{billingError}</p>}<BillingSettingsEditor key={JSON.stringify(billing)} value={billing} onSave={value => { setBilling(value); setBillingError(null); }}/></aside></div></Module>
      <Module number="03" title="重置周期用量" subtitle="从一次额度重置，到下一次额度重置" controls={<select aria-label="选择额度重置周期" value={data.cycle?.cycle.id ?? ""} onChange={e=>setCycleId(e.target.value)} disabled={!data.cycles.length}>{data.cycles.length?data.cycles.map(c=><option key={c.id} value={c.id}>{c.ongoing?"当前周期":"历史周期"} · {shortDate(c.start)} — {c.ongoing?"至今":shortDate(c.end)}</option>):<option value="">暂无可确认周期</option>}</select>}><div className="du-body">{data.cycle ? <><div className="du-cycle-caption"><span className="du-status">{data.cycle.cycle.ongoing ? "进行中" : "已结束"}</span><span>{dateTime(data.cycle.cycle.start)} → {dateTime(Math.min(data.cycle.cycle.end,data.end))}</span><span className="du-caption">按额度窗口推定 · 非精确重置日志</span></div><div className="du-cycle-summary"><div className="du-cycle-stat"><div><div className="du-label">周期总 Token</div><div className="du-note">所选周期内累计</div></div><div className="du-cycle-value"><TokenValue value={data.cycle.metrics.totalTokens}/></div></div><div className="du-cycle-stat"><div><div className="du-label">周期 API 等价费用</div><div className="du-note">{data.cycle.metrics.unpricedTokens ? `含 ${tokenText(data.cycle.metrics.unpricedTokens)} 未定价 Token` : "非实际支付金额"}</div></div><div className="du-cycle-value">{costText(data.cycle.metrics)}</div></div></div><div className="du-shares-grid"><div className="du-share-panel"><div className="du-panel-head"><h3>模型 Token 占比</h3><span className="du-caption">占周期总用量</span></div><Shares rows={data.cycle.models}/></div><div className="du-share-panel du-share-project"><div className="du-panel-head"><h3>项目与对话 Token 占比</h3><span className="du-caption">项目汇总 · 对话分别统计</span></div><Shares rows={data.cycle.attributions} attribution/></div></div></> : <p className="du-empty">尚未读取到周额度重置窗口。请先刷新菜单栏额度；不会用任意日期代替真实周期。</p>}</div></Module>
      <ExecutionHistory data={data} filters={executionFilters} onChange={setExecutionFilters} loading={loading}/>
    </div><footer className="du-footer"><span>仅统计本机记录，费用为估算；未定价或缺失数据可能影响结果。</span><span>USD · {dateTime(data.updatedAt)} 更新{loading ? " · 正在刷新" : ""}</span></footer>
    </>}
  </div></main>;
}
