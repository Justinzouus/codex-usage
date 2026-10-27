import { CaretLeft, ArrowClockwise } from "@phosphor-icons/react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { billingStart, costText, dateTime, fetchAnalytics, tokenText, type DashboardData, type UsageBucket } from "../lib/analytics";

export const grainLabel = (seconds: number) => seconds>=86400?`${seconds/86400} 天`:seconds>=3600?`${seconds/3600} 小时`:`${seconds/60} 分钟`;

function smoothPath(points: { x: number; y: number }[]) {
  if (points.length < 2) return points.length ? `M ${points[0].x} ${points[0].y}` : "";
  const slopes = points.slice(0, -1).map((point, index) => (points[index + 1].y - point.y) / (points[index + 1].x - point.x));
  const tangents = points.map((_, index) => index === 0 ? slopes[0] : index === points.length - 1 ? slopes.at(-1)! : slopes[index - 1] * slopes[index] <= 0 ? 0 : (slopes[index - 1] + slopes[index]) / 2);
  for (let i = 0; i < slopes.length; i++) {
    if (slopes[i] === 0) { tangents[i] = 0; tangents[i + 1] = 0; continue; }
    const a = tangents[i] / slopes[i], b = tangents[i + 1] / slopes[i], magnitude = a * a + b * b;
    if (magnitude > 9) { const scale = 3 / Math.sqrt(magnitude); tangents[i] = scale * a * slopes[i]; tangents[i + 1] = scale * b * slopes[i]; }
  }
  return points.slice(0, -1).map((point, index) => {
    const next = points[index + 1], dx = next.x - point.x;
    return `${index ? "" : `M ${point.x} ${point.y} `}C ${point.x + dx / 3} ${point.y + tangents[index] * dx / 3}, ${next.x - dx / 3} ${next.y - tangents[index + 1] * dx / 3}, ${next.x} ${next.y}`;
  }).join(" ");
}

export default function UsageTrend({ data, metric, hours }: { data: DashboardData; metric: "tokens"|"cost"; hours: number }) {
  const scrollRef=useRef<HTMLDivElement>(null), pinLatest=useRef(true), added=useRef(0), jumpAfterPrepend=useRef(0), generation=useRef(0), loadingRef=useRef(false), currentHours=useRef(hours);
  const [rows,setRows]=useState<UsageBucket[]>(data.trend), [hasOlder,setHasOlder]=useState(data.trendHasOlder), [loading,setLoading]=useState(false), [error,setError]=useState(false);
  const [width,setWidth]=useState(500), [offset,setOffset]=useState(0), [active,setActive]=useState<number|null>(null);
  const bins=hours===168?14:hours===720?15:12, step=Math.max(40,(width-58)/bins), contentWidth=Math.max(width,58+rows.length*step);
  useEffect(()=>{const node=scrollRef.current;if(!node)return;const observer=new ResizeObserver(()=>setWidth(node.clientWidth||500));observer.observe(node);setWidth(node.clientWidth||500);return()=>observer.disconnect();},[]);
  useEffect(()=>()=>{generation.current++;},[]);
  useLayoutEffect(()=>{
    const changed=currentHours.current!==hours;
    if(changed||pinLatest.current){generation.current++;currentHours.current=hours;pinLatest.current=true;loadingRef.current=false;added.current=0;setLoading(false);setRows(data.trend);setHasOlder(data.trendHasOlder);if(changed)setActive(null);setError(false);}
  },[data.trend,data.trendHasOlder,hours]);
  useLayoutEffect(()=>{
    const node=scrollRef.current;if(!node)return;
    if(added.current){node.scrollLeft+=added.current*step-jumpAfterPrepend.current;added.current=0;jumpAfterPrepend.current=0;}
    else if(pinLatest.current)node.scrollLeft=Math.max(0,node.scrollWidth-node.clientWidth);
    setOffset(node.scrollLeft);
  },[rows,step,width]);

  const loadOlder=async(navigate=false)=>{
    if(loadingRef.current||!hasOlder||!rows.length)return;
    const id=generation.current, before=rows[0].start;
    loadingRef.current=true;setLoading(true);setError(false);pinLatest.current=false;
    try {
      const result=await fetchAnalytics({hours,cycleId:null,project:null,billingStart:billingStart(),trendBefore:before});
      if(id!==generation.current)return;
      const older=result.trend.filter(b=>b.end<=before);
      setRows(current=>{if(current[0]?.start!==before)return current;added.current=older.length;jumpAfterPrepend.current=navigate?Math.max(0,width-58):0;return [...older,...current];});
      setHasOlder(result.trendHasOlder&&older.length>0);setActive(null);
    } catch {if(id===generation.current)setError(true);}
    finally {if(id===generation.current){loadingRef.current=false;setLoading(false);}}
  };
  const browseEarlier=()=>{const node=scrollRef.current;if(node&&node.scrollLeft>1){node.scrollLeft=Math.max(0,node.scrollLeft-node.clientWidth+58);}else void loadOlder(true);};
  const resetLatest=()=>{generation.current++;pinLatest.current=true;loadingRef.current=false;added.current=0;jumpAfterPrepend.current=0;setRows(data.trend);setHasOlder(data.trendHasOlder);setLoading(false);setError(false);setActive(null);const node=scrollRef.current;if(node){node.scrollLeft=Math.max(0,node.scrollWidth-node.clientWidth);setOffset(node.scrollLeft);}};
  const first=Math.max(0,Math.floor((offset-49)/step)-1), last=Math.min(rows.length,Math.ceil((offset+width)/step)+1);
  const shown=rows.slice(first,last), value=(b:UsageBucket)=>metric==="tokens"?b.metrics.totalTokens:b.metrics.costUsd;
  const max=(Math.max(0,...shown.map(value))*1.2)||1, divisor=metric==="cost"?1:max>=1e6?1e6:max>=1e3?1e3:1;
  const unit=metric==="cost"?"USD":divisor===1e6?"M":divisor===1e3?"K":"";
  const x=(i:number)=>49+(i+.5)*step, y=(v:number)=>180-v/max*145;
  const points=shown.map((b,i)=>({x:x(first+i),y:y(value(b))}));
  const path=smoothPath(points);
  const selected=active===null?null:rows[active];
  const tooltipLeft=active===null?0:Math.max(8,Math.min(width-198,x(active)-offset-94));
  const visibleStart=rows[Math.min(rows.length-1,Math.max(0,Math.floor(offset/step)))], visibleEnd=rows[Math.min(rows.length-1,Math.max(0,Math.floor((offset+width-58)/step)))];
  return <div className="du-trend-browser">
    <div className="du-trend-hint"><span>每点 {grainLabel(data.trendBucketSeconds)}</span><span>左右滚动浏览 · 悬停查看详情</span></div>
    <div className="du-trend-stage" onMouseLeave={()=>setActive(null)} onKeyDown={e=>{if(e.key==="Escape")setActive(null);}}>
      <div className="du-trend-scroll" ref={scrollRef} tabIndex={0} role="region" aria-label="可横向滚动的用量趋势" onScroll={e=>{const n=e.currentTarget;setOffset(n.scrollLeft);setActive(null);pinLatest.current=n.scrollLeft>=n.scrollWidth-n.clientWidth-4;if(n.scrollLeft<step&&!error)void loadOlder();}}>
        <svg className="du-trend-canvas" width={contentWidth} height="230" role="group" aria-label={`${metric==="tokens"?"Token":"API 等价费用"}趋势数据点`}>
          <defs><linearGradient id="usage-trend-fill" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="#6389c8" stopOpacity=".24"/><stop offset="100%" stopColor="#6389c8" stopOpacity=".01"/></linearGradient></defs>
          {[0,1,2,3].map(i=><line key={i} x1={offset+45} x2={offset+width} y1={y(max*i/3)} y2={y(max*i/3)} stroke="#dbe5eb"/>)}
          {points.length>0&&<><path d={`${path} L ${points.at(-1)!.x} 180 L ${points[0].x} 180 Z`} fill="url(#usage-trend-fill)"/><path d={path} fill="none" stroke="#6389c8" strokeWidth="2.4" strokeLinejoin="round"/></>}
          {shown.map((b,j)=>{const i=first+j;return <g key={b.start}>
            {i%Math.max(1,Math.ceil(90/step))===0&&<text x={x(i)} y="211" textAnchor="middle">{dateTime(b.start)}</text>}
            {active===i&&<line x1={x(i)} x2={x(i)} y1="25" y2="182" stroke="#7294bc" strokeDasharray="3 4"/>}
            <circle cx={x(i)} cy={y(value(b))} r={active===i?5:3} fill="#6389c8" stroke="#f9fbfd" strokeWidth="1.5" pointerEvents="none"/>
            <rect x={x(i)-step/2} y="24" width={step} height="164" fill="transparent" tabIndex={0} role="button" aria-label={`查看趋势点 ${dateTime(b.start)}`} onMouseEnter={()=>setActive(i)} onFocus={()=>setActive(i)} onBlur={()=>setActive(null)} onClick={()=>setActive(i)} onKeyDown={e=>{if(e.key==="Enter"||e.key===" "){e.preventDefault();setActive(i);}}}/>
          </g>;})}
        </svg>
      </div>
      <svg className="du-trend-axis" width="47" height="198" aria-hidden="true"><text x="2" y="16">{unit||"Token"}</text>{[0,1,2,3].map(i=><text key={i} x="39" y={y(max*i/3)+4} textAnchor="end">{(max*i/3/divisor).toLocaleString("en-US",{maximumFractionDigits:max/divisor<10?2:1})}</text>)}</svg>
      {selected&&<div role="tooltip" className="du-trend-tooltip" style={{left:tooltipLeft}}><strong>{dateTime(selected.start)} — {dateTime(Math.min(selected.end,data.updatedAt))}</strong><div><span>消耗 Token</span><b>{tokenText(selected.metrics.totalTokens)}</b></div><div><span>API 等价费用</span><b>{costText(selected.metrics)}</b></div></div>}
    </div>
    <div className="du-trend-navigation"><button className="du-trend-earlier" disabled={loading||(!hasOlder&&offset<=1)} onClick={browseEarlier}><CaretLeft/>{loading?"加载中…":hasOlder||offset>1?"更早记录":"已到最早记录"}</button><span>{visibleStart&&visibleEnd?`${dateTime(visibleStart.start)} — ${dateTime(Math.min(visibleEnd.end,data.updatedAt))}`:"暂无数据"}</span><button onClick={resetLatest} aria-label="趋势回到最新"><ArrowClockwise/>最新</button></div>
    {error&&<p role="alert" className="du-note">更早记录暂时无法加载，请点击“更早记录”重试。</p>}
    {metric==="cost"&&shown.some(b=>b.metrics.unpricedTokens>0)&&<p className="du-note">费用曲线仅含已知价格，未定价 Token 仍计入用量。</p>}
  </div>;
}
