import { isTauri } from "./bridge";
import type { BillingSettings } from "./billing";

export interface ServiceTierUsage { tier: string; tokens: number; costUsd: number; unpricedTokens: number; requestModeTokens?: number; assumedStandardTokens?: number }
export interface Metrics {
  inputTokens: number; cachedInputTokens: number; cacheWriteInputTokens: number; outputTokens: number;
  totalTokens: number; costUsd: number; unpricedTokens: number; mainAgentCount: number; subAgentCount: number;
  serviceTiers: ServiceTierUsage[]; requestModeTokens: number; assumedStandardTokens: number;
}
export interface UsageComparisons { previous: Metrics }
export interface Cycle { id: string; start: number; end: number; ongoing: boolean; estimated: boolean }
export interface Share { id: string; name: string; kind: "model" | "project" | "conversation" | "unassigned" | "other"; tokens: number; percent: number }
export interface Execution { id: string; title: string; project: string; projectId: string | null; threadId: string; threadTitle: string; models: string[]; started: number; completed: number; metrics: Metrics }
export interface ConversationSummary { threadId: string; title: string; project: string; projectId: string | null; models: string[]; executionCount: number; started: number; lastCompleted: number; metrics: Metrics }
export interface UsageBucket { start: number; end: number; metrics: Metrics }
export interface ResultPage { page: number; pageSize: number; total: number; totalPages: number }
export type ProjectScope = "all" | "project" | "projectless";
export interface FilterOption { id: string; name: string }
export interface AnalyticsQuery { hours: number; cycleId: string | null; project: string | null; billingStart: number; executionHours?: number | null; projectScope?: ProjectScope; threadId?: string | null; page?: number; trendBefore?: number; refreshPricing?: boolean }
export interface PricingSync {
  status: "upToDate" | "partial" | "updating" | "failed" | "neverChecked";
  lastCheckedAt: number | null; lastSuccessAt: number | null; sourceUrl: string;
  usingCachedRates: boolean; error: string | null; modelCount: number;
  missingModels: { model: string; tokens: number; reason: string; sourceUrl: string }[];
}
export interface DashboardData {
  updatedAt: number; start: number; end: number; history: Metrics; comparisons: UsageComparisons;
  trend: UsageBucket[]; trendBucketSeconds: number; trendHasOlder: boolean;
  billingStart: number; billingIsRolling: boolean; billing: Metrics;
  cycles: Cycle[]; cycle: { cycle: Cycle; metrics: Metrics; models: Share[]; attributions: Share[] } | null;
  executions: Execution[]; projects: FilterOption[]; conversations: FilterOption[];
  executionPage: ResultPage;
  sessions: ConversationSummary[]; sessionPage: ResultPage;
  coverage: { files: number; unreadableFiles: number; malformedLines: number; oversizedLines: number; firstEventAt: number | null; lastEventAt: number | null; unassignedEvents: number; excludedInternalEvents: number };
  pricingDate: string; pricing: PricingSync;
}

/** Resolve the selected monthly billing date in its own timezone, clamping short months. */
export function billingStart(now = new Date(), settings: Pick<BillingSettings, "day" | "timeZone"> = { day: 1, timeZone: "UTC" }): number {
  const formatter = new Intl.DateTimeFormat("en-US", { timeZone: settings.timeZone, year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric", hourCycle: "h23" });
  const parts = (date: Date) => Object.fromEntries(formatter.formatToParts(date).map(p => [p.type, Number(p.value)]));
  const p = parts(now);
  const cappedDay = (year: number, month: number) => Math.min(settings.day, new Date(Date.UTC(year, month + 1, 0)).getUTCDate());
  const month = p.day >= cappedDay(p.year, p.month - 1) ? p.month - 1 : p.month - 2;
  const target = Date.UTC(p.year, month, cappedDay(p.year, month));
  let epoch = target;
  for (let i = 0; i < 3; i++) {
    const x = parts(new Date(epoch));
    const represented = Date.UTC(x.year, x.month - 1, x.day, x.hour, x.minute, x.second);
    epoch += target - represented;
  }
  return Math.floor(epoch / 1000);
}

export function formatTokens(n: number): { value: string; unit: string } {
  if (n >= 1e9) return { value: (n / 1e9).toFixed(2), unit: "B" };
  if (n >= 1e6) return { value: (n / 1e6).toFixed(2), unit: "M" };
  if (n >= 1e3) return { value: (n / 1e3).toFixed(n % 1000 === 0 ? 0 : 1), unit: "K" };
  return { value: Math.max(0, n).toLocaleString("en-US"), unit: "" };
}
export const tokenText = (n: number) => { const v = formatTokens(n); return `${v.value} ${v.unit}`.trim(); };
export const dollars = (n: number) => new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(n);
export function hasUnknownServiceTier(m: Pick<Metrics, "serviceTiers">) { return m.serviceTiers.some(row => row.tier === "unknown" && row.tokens > 0); }
export function hasAssumedStandardUsage(m: Partial<Pick<Metrics, "assumedStandardTokens">>) { return (m.assumedStandardTokens ?? 0) > 0; }
export function hasEstimatedServiceTier(m: Pick<Metrics, "serviceTiers"> & Partial<Pick<Metrics, "requestModeTokens" | "assumedStandardTokens">>) { return (m.requestModeTokens ?? 0) > 0 || hasAssumedStandardUsage(m) || hasUnknownServiceTier(m); }
export function costText(m: Pick<Metrics, "unpricedTokens" | "costUsd"> & Partial<Pick<Metrics, "serviceTiers" | "requestModeTokens" | "assumedStandardTokens">>) {
  const approximate = (m.requestModeTokens ?? 0) > 0 || hasAssumedStandardUsage(m) || !!m.serviceTiers && hasUnknownServiceTier({ serviceTiers: m.serviceTiers });
  if (m.unpricedTokens) return m.costUsd > 0 ? approximate ? `${dollars(m.costUsd)}（部分未定价）` : `≥ ${dollars(m.costUsd)}` : "暂无法估价";
  return dollars(m.costUsd);
}
export const cacheRate = (m: Metrics) => m.inputTokens > 0 ? `${(Math.min(m.inputTokens, m.cachedInputTokens) / m.inputTokens * 100).toFixed(1)}%` : "—";
export const dateTime = (seconds: number) => new Date(seconds * 1000).toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false });
export const shortDate = (seconds: number) => new Date(seconds * 1000).toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
export const duration = (start: number, end: number) => { const seconds = Math.max(0, end - start); return seconds >= 3600 ? `${Math.floor(seconds / 3600)} 小时 ${Math.floor(seconds % 3600 / 60)} 分` : `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`; };

export async function fetchAnalytics(query: AnalyticsQuery): Promise<DashboardData> {
  if (!isTauri()) return demoDashboard(query);
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<DashboardData>("get_analytics", { query });
}

export async function openPricingSource(url: string): Promise<void> {
  const source = new URL(url);
  if (source.protocol !== "https:" || source.hostname !== "developers.openai.com" || source.port || source.username || source.password || source.search || source.hash || !/^\/api\/docs\/(pricing\/?|models\/[a-z0-9][a-z0-9._-]*\/?)$/.test(source.pathname)) {
    throw new Error("只能打开 OpenAI 官方 API 价格或模型页面。");
  }
  if (!isTauri()) { window.open(source.href, "_blank", "noopener,noreferrer"); return; }
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_pricing_source", { url: source.href });
}

// Browser development only. The desktop path above NEVER falls back to demo data.
function sumServiceTiers(rows: ServiceTierUsage[]): ServiceTierUsage[] {
  const totals = new Map<string, ServiceTierUsage>();
  for (const row of rows) {
    const previous = totals.get(row.tier) ?? { tier: row.tier, tokens: 0, costUsd: 0, unpricedTokens: 0 };
    totals.set(row.tier, { tier: row.tier, tokens: previous.tokens + row.tokens, costUsd: previous.costUsd + row.costUsd, unpricedTokens: previous.unpricedTokens + row.unpricedTokens });
  }
  return [...totals.values()];
}
function demoExecutionPage(rows: Execution[], query: AnalyticsQuery, now: number) {
  const scoped = rows.filter(e => query.projectScope === "project" ? !!e.projectId : query.projectScope === "projectless" ? !e.projectId : true);
  const projects = [...new Map(scoped.filter(e => e.projectId).map(e => [e.projectId!, { id: e.projectId!, name: e.project }])).values()];
  const inProject = scoped.filter(e => (!query.executionHours || e.completed >= now-query.executionHours*3600) && (!query.project || e.projectId === query.project));
  const conversations = [...new Map(inProject.map(e => [e.threadId, { id: e.threadId, name: e.threadTitle }])).values()];
  const filtered = inProject.filter(e => !query.threadId || e.threadId === query.threadId);
  const total = filtered.length, pageSize = 5, totalPages = Math.max(1,Math.ceil(total/pageSize)), page = Math.max(1,Math.min(query.page ?? 1,totalPages));
  const threads = new Map<string, Execution[]>();
  filtered.forEach(e=>threads.set(e.threadId,[...(threads.get(e.threadId)??[]),e]));
  const sessions = [...threads.values()].map((rows): ConversationSummary => ({ threadId:rows[0].threadId, title:rows[0].threadTitle, project:rows[0].project, projectId:rows[0].projectId, executionCount:rows.length, started:Math.min(...rows.map(r=>r.started)), lastCompleted:rows[0].completed, models:[...new Set(rows.flatMap(r=>r.models))], metrics:rows.reduce<Metrics>((sum,r)=>({ ...sum, inputTokens:sum.inputTokens+r.metrics.inputTokens,cachedInputTokens:sum.cachedInputTokens+r.metrics.cachedInputTokens,outputTokens:sum.outputTokens+r.metrics.outputTokens,totalTokens:sum.totalTokens+r.metrics.totalTokens,costUsd:sum.costUsd+r.metrics.costUsd,serviceTiers:sumServiceTiers([...sum.serviceTiers,...r.metrics.serviceTiers]) }),{...rows[0].metrics,inputTokens:0,cachedInputTokens:0,outputTokens:0,totalTokens:0,costUsd:0,serviceTiers:[]}) }));
  const sessionTotal = sessions.length, sessionPages = Math.max(1,Math.ceil(sessionTotal/pageSize)), sessionPage = Math.max(1,Math.min(query.page??1,sessionPages));
  return { executions: filtered.slice((page-1)*pageSize,page*pageSize), projects, conversations, executionPage: { page, pageSize, total, totalPages }, sessions:sessions.slice((sessionPage-1)*pageSize,sessionPage*pageSize),sessionPage:{page:sessionPage,pageSize,total:sessionTotal,totalPages:sessionPages} };
}

export function demoDashboard(query: AnalyticsQuery): DashboardData {
  const now = Math.floor(Date.now() / 1000), factor = query.hours / 24;
  const m = (total: number, cost: number): Metrics => ({ inputTokens: Math.round(total * .94), cachedInputTokens: Math.round(total * .75), cacheWriteInputTokens: 0, outputTokens: total - Math.round(total * .94), totalTokens: total, costUsd: cost, unpricedTokens: 0, mainAgentCount: Math.max(1, Math.round(4 * factor)), subAgentCount: Math.max(1, Math.round(8 * factor)), requestModeTokens: 0, assumedStandardTokens: 0, serviceTiers: [{ tier: "standard", tokens: total - Math.round(total * .25), costUsd: cost * .6, unpricedTokens: 0 }, { tier: "fast", tokens: Math.round(total * .25), costUsd: cost * .4, unpricedTokens: 0 }] });
  const history = m(Math.round(3840000 * factor), 24.86 * factor);
  const comparisons: UsageComparisons = { previous: m(Math.round(history.totalTokens * .91), history.costUsd * .91) };
  const weights = [8, 3, 1, 0, 0, 2, 6, 14, 5, 9, 18, 10], sum = weights.reduce((a, b) => a + b, 0), start = now - query.hours * 3600;
  const bins = query.hours===168?14:query.hours===720?15:12, trendBucketSeconds = query.hours*3600/bins;
  const trendStart = (query.trendBefore??now)-query.hours*3600*3;
  const trend = Array.from({length:bins*3},(_,i)=>({start:trendStart+i*trendBucketSeconds,end:i===bins*3-1?(query.trendBefore??now+1):trendStart+(i+1)*trendBucketSeconds,metrics:m(Math.round(history.totalTokens*weights[i%12]/sum),history.costUsd*weights[i%12]/sum)}));
  const cycles = [0, 1, 2].map((n): Cycle => ({ id: `demo-${n}`, start: now - (2 + n * 7) * 86400, end: now + (5 - n * 7) * 86400, ongoing: n === 0, estimated: true }));
  const selected = cycles.find(c => c.id === query.cycleId) ?? cycles[0], f = selected.id === "demo-0" ? 1 : 2;
  const projects = ["Codex Usage", "数据分析", "个人网站"];
  const executions: Execution[] = ["优化周额度详情卡", "补充数据清洗与异常检查", "调整首页排版与响应式布局", "梳理接口边界与刷新逻辑", "整理独立对话笔记", "验证缓存统计结果", "更新筛选交互", "完善趋势图例", "检查移动端布局", "整理使用记录", "补充导出说明", "回顾旧版交互"].map((title, i) => {
    const projectId = i === 4 || i === 9 ? null : projects[i % 3], threadId = projectId ? `thread-${i % 4}` : "thread-solo";
    const age = i < 5 ? (i+1)*2200 : [0,0,0,0,0,50000,88000,170000,350000,650000,1200000,3200000][i];
    return { id: String(i), title, project: projectId ?? "未归属项目", projectId, threadId: `${projectId ?? "solo"}-${threadId}`, threadTitle: projectId ? `${projectId} · ${i%4===0?"界面优化":"功能完善"}` : "独立对话", models: [i === 1 ? "gpt-5.6-terra" : "gpt-5.6-sol"], started: now-age-272, completed: now-age, metrics: m([186400,124000,342800,268200][i%4], [1.24,.68,2.31,1.82][i%4]) };
  });
  for(let i=0;i<6;i++) executions.push({...executions[0],id:`extra-${i}`,title:`补充会话执行 ${i+1}`,started:now-(i+2)*9000-272,completed:now-(i+2)*9000,metrics:m(80000+i*10000,.5+i*.05)});
  executions.sort((a,b)=>b.completed-a.completed);
  return { updatedAt: now, start, end: now, history, comparisons, trend, trendBucketSeconds, trendHasOlder:trendStart>now-70*86400, billingStart: query.billingStart, billingIsRolling: false, billing: m(56000000,368.42), cycles, cycle: { cycle: selected, metrics: m(8640000 * f,58.72 * f), models: [["gpt-5.6-sol",58],["gpt-5.6-terra",27],["gpt-5.5",15]].map(([name,percent]) => ({ id: String(name), name: String(name), kind: "model" as const, percent: Number(percent), tokens: 8640000 * f * Number(percent) / 100 })), attributions: [{id:"demo-codex-usage",name:projects[0],kind:"project" as const,percent:52,tokens:8640000*f*.52},{id:"demo-data-analysis",name:projects[1],kind:"project" as const,percent:31,tokens:8640000*f*.31},{id:"demo-solo-thread",name:"整理独立对话笔记",kind:"conversation" as const,percent:17,tokens:8640000*f*.17}] }, ...demoExecutionPage(executions, query, now), coverage: { files: 60, unreadableFiles: 0, malformedLines: 0, oversizedLines: 0, firstEventAt: now - 70 * 86400, lastEventAt: now - 60, unassignedEvents: 0, excludedInternalEvents: 0 }, pricingDate: "2026-10-13", pricing: { status: "upToDate", lastCheckedAt: now, lastSuccessAt: now, sourceUrl: "https://developers.openai.com/api/docs/pricing", usingCachedRates: false, error: null, missingModels: [], modelCount: 24 } };
}
