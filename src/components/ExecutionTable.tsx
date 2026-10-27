import { Fragment, useState } from "react";
import { costText, dateTime, duration, formatTokens, type Execution } from "../lib/analytics";

export default function ExecutionTable({ rows }: { rows: Execution[] }) {
  const [expanded, setExpanded] = useState<string|null>(null);
  return <div className="du-table-wrap"><table><thead><tr><th>执行内容</th><th>模型</th><th>完成时间</th><th>Token</th><th>API 等价费用</th></tr></thead><tbody>{rows.map(t=>{const n=formatTokens(t.metrics.totalTokens);return <Fragment key={t.id}>
    <tr className="du-task-row" data-open={expanded===t.id}><td><button className="du-task-button" aria-expanded={expanded===t.id} aria-controls={`execution-${t.id}`} onClick={()=>setExpanded(expanded===t.id?null:t.id)}><span aria-hidden="true" className="du-task-arrow">›</span>{t.title}</button></td><td><span className="du-model" title={t.models.join("、")}>{t.models[0]??"未知"}{t.models.length>1?` +${t.models.length-1}`:""}</span></td><td className="du-time">{dateTime(t.completed)}</td><td className="du-token-cell">{n.value}<span className="du-unit">{n.unit}</span></td><td className="du-cost-cell">{costText(t.metrics)}</td></tr>
    {expanded===t.id&&<tr className="du-expanded" id={`execution-${t.id}`}><td colSpan={5}><div className="du-detail-grid"><div><span>普通输入</span><b>{Math.max(0,t.metrics.inputTokens-t.metrics.cachedInputTokens-t.metrics.cacheWriteInputTokens).toLocaleString()}</b></div><div><span>缓存输入 / 写入</span><b>{t.metrics.cachedInputTokens.toLocaleString()} / {t.metrics.cacheWriteInputTokens.toLocaleString()}</b></div><div><span>输出（已含推理）</span><b>{t.metrics.outputTokens.toLocaleString()}</b></div><div><span>执行时长 · 子 Agent</span><b>{duration(t.started,t.completed)} · {t.metrics.subAgentCount} 个</b></div></div></td></tr>}
  </Fragment>;})}</tbody></table></div>;
}
