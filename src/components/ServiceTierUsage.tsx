import { costText, tokenText, type Metrics, type ServiceTierUsage as TierUsage } from "../lib/analytics";
import type { ReactNode } from "react";
import "../pricing.css";

const tierName = (tier: string) => ({ standard: "Standard", fast: "Fast", ultrafast: "Ultrafast" }[tier] ?? tier);
const tierCost = (row: TierUsage) => costText({ ...row, serviceTiers: [row] });

export default function ServiceTierUsage({ metrics, scope, children }: { metrics: Metrics; scope: string; children?: ReactNode }) {
  const rows = metrics.serviceTiers.filter(row => row.tokens > 0);
  if (!rows.length) return null;
  return <div className="du-tier-usage" aria-label={`${scope}API等价费用计算口径`}>
    <details>
      <summary>API等价费用计算口径</summary>
      <ul>{rows.map(row => <li key={row.tier}><span>{tierName(row.tier)}</span><span>{tokenText(row.tokens)} Token</span><b>{tierCost(row)}</b>{row.unpricedTokens > 0 && <small>{tokenText(row.unpricedTokens)} Token 暂无此模式的单价</small>}</li>)}</ul>
      {metrics.assumedStandardTokens > 0 && <p>历史模式缺失的 {tokenText(metrics.assumedStandardTokens)} Token 已归入 Standard，使用 Standard 单价估算。</p>}
      <table className="du-tier-rules">
        <caption>Fast 计费口径</caption>
        <thead><tr><th>使用方式</th><th>规则</th></tr></thead>
        <tbody>
          <tr><th scope="row">套餐内额度</th><td>同等用量消耗 2.5× Standard 额度</td></tr>
          <tr><th scope="row">额外 credits / 企业按量付费</th><td>按 2× Standard 费率</td></tr>
          <tr><th scope="row">API 等价费用</th><td>按模型的 Fast Token 单价计算</td></tr>
        </tbody>
      </table>
      <p>套餐与 credits 倍率分别适用于同一模型的 Standard 用量和费率。上方 API 金额使用独立价格表；套餐剩余额度以 Codex 返回值为准。</p>
      <p>Token 为实际记录数量；费用按模型及模式对应的 API 单价估算。</p>
      {children}
    </details>
  </div>;
}
