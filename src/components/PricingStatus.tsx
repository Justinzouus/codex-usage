import { ArrowClockwise } from "@phosphor-icons/react";
import { useState } from "react";
import { dateTime, openPricingSource, tokenText, type PricingSync } from "../lib/analytics";
import "../pricing.css";

export default function PricingStatus({ pricing, refreshing, onRefresh }: { pricing: PricingSync; refreshing: boolean; onRefresh: () => void }) {
  const [sourceError, setSourceError] = useState<string | null>(null);
  const updating = refreshing || pricing.status === "updating";
  const failed = pricing.status === "failed";
  const partial = pricing.status === "partial";
  const missingModelCount = new Set(pricing.missingModels.map(model => model.model)).size;
  const missingTokens = pricing.missingModels.reduce((total, model) => total + model.tokens, 0);
  const openSource = async (url: string) => {
    try { await openPricingSource(url); setSourceError(null); }
    catch { setSourceError("官方来源页面未能打开，请稍后重试。"); }
  };
  const status = updating ? "正在查询官方价格…" : failed ? "价格更新失败" : partial ? "官方价格已更新 · 部分旧价待核对" : pricing.status === "neverChecked" ? "尚未核对在线价格" : "官方价格已同步";

  return <div className={`du-pricing${failed || partial || pricing.missingModels.length ? " du-pricing--attention" : ""}`} aria-label="API 价格同步">
    <div className="du-pricing-row">
      <span className="du-pricing-state" role="status">{status}</span>
      <span className="du-pricing-time">{pricing.lastSuccessAt ? `上次成功 ${dateTime(pricing.lastSuccessAt)}` : "尚无成功同步记录"}</span>
      <button type="button" className="du-pricing-refresh" onClick={onRefresh} disabled={updating} aria-label="立即更新 API 价格"><ArrowClockwise className={updating ? "du-spinning" : ""}/>{updating ? "更新中" : "立即更新"}</button>
    </div>
    {failed && <p className="du-pricing-error" role="alert">{pricing.error || "无法读取官方价格，请重试。"}{pricing.usingCachedRates && pricing.lastSuccessAt ? " 仍使用上次成功缓存的单价。" : " 继续使用已有单价，估价可能不完整。"}</p>}
    <details className="du-pricing-details">
      <summary>{pricing.missingModels.length ? `价格详情 · 全部本机历史中 ${missingModelCount} 个模型未定价（${tokenText(missingTokens)} Token）` : "价格详情"}</summary>
      <div className="du-pricing-source"><span>已载入 {pricing.modelCount} 条模型单价{pricing.lastCheckedAt ? ` · 上次检查 ${dateTime(pricing.lastCheckedAt)}` : ""}</span><button type="button" onClick={() => void openSource(pricing.sourceUrl)}>查看官方价格表</button></div>
      {partial && <p className="du-pricing-error">{pricing.error || "最新价格已载入，部分旧模型仍使用此前核验价格。"}</p>}
      {pricing.missingModels.length > 0 && <>
        <p className="du-pricing-scope">以下覆盖全部本机历史，不随上方时间范围或项目筛选变化。未定价 Token 仍计入用量，费用只包含已知单价。</p>
        <ul className="du-pricing-missing">{pricing.missingModels.map(model => <li key={`${model.model}:${model.reason}`}>
          <div className="du-pricing-model"><strong>{model.model}</strong><span>{tokenText(model.tokens)} Token 未定价</span><button type="button" onClick={() => void openSource(model.sourceUrl)} aria-label={`查看 ${model.model} 的官方来源`}>官方来源</button></div>
          <p>{model.reason}</p>
        </li>)}</ul>
      </>}
      {sourceError && <p className="du-pricing-error" role="alert">{sourceError}</p>}
    </details>
  </div>;
}
