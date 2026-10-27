import { CaretLeft, CaretRight } from "@phosphor-icons/react";
import type { ResultPage } from "../lib/analytics";

export default function UsagePagination({ value, loading, name, onChange }: { value: ResultPage; loading: boolean; name: string; onChange: (page: number)=>void }) {
  const { page, pageSize, total, totalPages } = value;
  return <div className="du-pagination"><span role="status" aria-live="polite">共 {total} 条{total>0?` · 显示 ${(page-1)*pageSize+1}–${Math.min(page*pageSize,total)} 条`:""}</span><nav aria-label={`${name}分页`}><button aria-label={`上一页${name}`} disabled={loading||page<=1} onClick={()=>onChange(page-1)}><CaretLeft/>上一页</button><span>第 {page} / {totalPages} 页</span><button aria-label={`下一页${name}`} disabled={loading||page>=totalPages} onClick={()=>onChange(page+1)}>下一页<CaretRight/></button></nav></div>;
}
