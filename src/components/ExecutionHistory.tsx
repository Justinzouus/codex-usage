import { useState } from "react";
import { costText, dateTime, formatTokens, type DashboardData, type ProjectScope } from "../lib/analytics";

import { openConversation } from "../lib/bridge";
import UsagePagination from "./UsagePagination";

export interface ExecutionFilters { executionHours: number | null; projectScope: ProjectScope; project: string | null; threadId: string | null; page: number }
export const initialExecutionFilters: ExecutionFilters = { executionHours: null, projectScope: "all", project: null, threadId: null, page: 1 };
const timeRanges = [[0,"全部时间"],[1,"最近 1 小时"],[12,"最近 12 小时"],[24,"最近 1 天"],[168,"最近 1 周"],[720,"最近 30 天"]] as const;

export default function ExecutionHistory({ data, filters, onChange, loading }: { data: DashboardData; filters: ExecutionFilters; onChange: (filters: ExecutionFilters) => void; loading: boolean }) {
  const [error, setError] = useState<string | null>(null);
  const showDetails = async (threadId: string) => { try { await openConversation(threadId); setError(null); } catch { setError("会话详情未能打开，请重试；浏览器预览需允许弹出窗口。"); } };
  const update = (patch: Partial<ExecutionFilters>) => { setError(null); onChange({ ...filters, page: 1, ...patch }); };
  const hasFilters = !!filters.executionHours || filters.projectScope !== "all" || !!filters.project || !!filters.threadId;
  return <section className="du-module" aria-labelledby="module-04">
    <header className="du-section-head"><div className="du-heading-group"><span className="du-number" aria-hidden="true">04</span><div><h2 id="module-04">最近执行明细</h2><p>每行一个会话 · 点击详情查看逐轮用量</p></div></div><span className="du-caption">每页 5 条</span></header>
    <div className="du-execution-filters" aria-label="执行明细筛选">
      <label><span>时间范围</span><select aria-label="筛选最近执行的时间范围" value={filters.executionHours ?? 0} onChange={e=>update({ executionHours: Number(e.target.value)||null, threadId: null })}>{timeRanges.map(([value,label])=><option key={value} value={value}>{label}</option>)}</select></label>
      <label><span>项目归属</span><select aria-label="筛选是否属于项目" value={filters.projectScope} onChange={e=>update({ projectScope:e.target.value as ProjectScope, project:null, threadId:null })}><option value="all">全部执行</option><option value="project">属于项目</option><option value="projectless">不属于项目</option></select></label>
      <label><span>具体项目</span><select aria-label="筛选最近执行的项目" value={filters.project??""} disabled={filters.projectScope === "projectless"} onChange={e=>update({ project:e.target.value||null, threadId:null })}><option value="">{filters.projectScope === "projectless" ? "不适用" : "全部项目"}</option>{data.projects.map(p=><option value={p.id} key={p.id}>{p.name}</option>)}</select></label>
      <label><span>具体对话</span><select aria-label="筛选最近执行的对话" value={filters.threadId??""} onChange={e=>update({ threadId:e.target.value||null })}><option value="">全部对话</option>{filters.threadId&&!data.conversations.some(t=>t.id===filters.threadId)&&<option value={filters.threadId}>所选对话（当前时段无执行）</option>}{data.conversations.map(t=><option value={t.id} key={t.id}>{t.name} · {t.id.slice(-6)}</option>)}</select></label>
      <button className="du-clear-filters" disabled={!hasFilters} onClick={()=>update(initialExecutionFilters)}>清除筛选</button>
      <p className="du-filter-note">汇总所选时间内的已完成执行 · 详情展示会话全部历史</p>
    </div>
    {error&&<p role="alert" className="du-notice du-error">{error}</p>}
    <div className="du-table-body" aria-busy={loading}><div className="du-table-wrap"><table className="du-session-table"><thead><tr><th>会话</th><th>模型</th><th>最近执行</th><th>Token</th><th>API 等价费用</th><th><span className="sr-only">操作</span></th></tr></thead><tbody>
      {data.sessions.map(t=>{const n=formatTokens(t.metrics.totalTokens);return <tr key={t.threadId} className="du-session-row"><td><span className="du-task-title">{t.title}</span><span className="du-task-project">{t.project} · {t.executionCount} 轮执行</span></td><td><span className="du-model" title={t.models.join("、")}>{t.models[0]??"未知"}{t.models.length>1?` +${t.models.length-1}`:""}</span></td><td className="du-time">{dateTime(t.lastCompleted)}</td><td className="du-token-cell">{n.value}<span className="du-unit">{n.unit}</span></td><td className="du-cost-cell">{costText(t.metrics)}</td><td><button className="du-session-detail" aria-label={`查看会话详情：${t.title}`} onClick={()=>void showDetails(t.threadId)}>详情<span aria-hidden="true"> ↗</span></button></td></tr>;})}
    </tbody></table></div>
      {!data.sessions.length&&<p className="du-empty">{hasFilters?"没有符合筛选条件的会话，可调整或清除筛选。":"暂无已完成执行的会话"}</p>}
      <UsagePagination value={data.sessionPage} name="会话明细" loading={loading} onChange={page=>update({page})}/>
      <div className="du-table-footer">已包含关联子 Agent 的用量</div>
    </div>
  </section>;
}
