import { ArrowClockwise } from "@phosphor-icons/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { billingStart, costText, dateTime, fetchAnalytics, tokenText, type DashboardData } from "../lib/analytics";
import { isTauri } from "../lib/bridge";
import ExecutionTable from "./ExecutionTable";
import UsagePagination from "./UsagePagination";
import "../dashboard.css";

export default function ConversationDetails({ threadId }: { threadId: string }) {
  const [page,setPage]=useState(1), [data,setData]=useState<DashboardData|null>(null), [loading,setLoading]=useState(true), [error,setError]=useState<string|null>(null);
  const request=useRef(0);
  const refresh=useCallback(async()=>{
    const id=++request.current; setLoading(true);
    if(!threadId){setError("未找到会话，请关闭窗口后重新打开详情。");setLoading(false);return;}
    try {const result=await fetchAnalytics({hours:24,cycleId:null,project:null,billingStart:billingStart(),threadId,page});if(id===request.current){setData(result);setError(null);}}
    catch {if(id===request.current)setError("会话用量暂时无法读取，请稍后重试。");}
    finally {if(id===request.current)setLoading(false);}
  },[threadId,page]);
  useEffect(()=>{document.documentElement.classList.add("dashboard-mode");return()=>document.documentElement.classList.remove("dashboard-mode");},[]);
  useEffect(()=>{void refresh();const timer=window.setInterval(()=>{if(document.visibilityState==="visible")void refresh();},60000);const focus=()=>void refresh();window.addEventListener("focus",focus);return()=>{request.current++;clearInterval(timer);window.removeEventListener("focus",focus);};},[refresh]);
  const session=data?.sessions[0];
  return <main className="usage-dashboard du-conversation-window"><div className="du-chrome"><span className="du-wordmark">CODEX · USAGE</span><span className="du-sync">{isTauri()?"会话用量详情":"浏览器预览 · 示例数据"}</span></div><div className="du-main">
    <header className="du-page-head"><div><p className="du-detail-eyebrow">会话历史</p><h1>{session?.title??"会话用量详情"}</h1><p>{session?.project??(data?"未找到会话":"正在读取")} · 全部已完成执行</p></div><button className="du-refresh" disabled={loading} onClick={()=>void refresh()} aria-label="刷新会话详情"><ArrowClockwise className={loading?"du-spinning":""}/>{loading?"读取中":"刷新"}</button></header>
    {error&&<p className="du-notice du-error" role="alert">{error}</p>}
    {session&&<div className="du-conversation-summary"><div><span>累计 Token</span><strong>{tokenText(session.metrics.totalTokens)}</strong></div><div><span>API 等价费用</span><strong>{costText(session.metrics)}</strong></div><div><span>已完成执行</span><strong>{session.executionCount}<small> 轮</small></strong></div></div>}
    <section className="du-module" aria-label="会话执行历史"><header className="du-section-head"><div><h2>逐轮执行明细</h2><p>点击执行内容查看 Token 构成 · 每页 5 条</p></div></header><div className="du-table-body" aria-busy={loading}>
      {data?<><ExecutionTable key={`${threadId}-${data.executionPage.page}`} rows={data.executions}/>{!data.executions.length&&<p className="du-empty">这个会话暂无已完成的执行记录。</p>}<UsagePagination value={data.executionPage} name="会话执行" loading={loading} onChange={setPage}/></>:<p className="du-empty" role="status">{loading?"正在整理这个会话的历史用量…":"暂无可展示数据"}</p>}
    </div></section><footer className="du-footer"><span>仅统计本机记录，含关联子 Agent；费用为估算。</span>{data&&<span>{dateTime(data.updatedAt)} 更新</span>}</footer>
  </div></main>;
}
