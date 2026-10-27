import { useState, type FormEvent } from "react";
import { billingTimeZones, saveBillingSettings, type BillingSettings as Settings } from "../lib/billing";

export default function BillingSettings({ value, onSave }: { value: Settings | null; onSave: (value: Settings) => void }) {
  const [monthly, setMonthly] = useState(value ? String(value.monthlyUsd) : "");
  const [extra, setExtra] = useState(value ? String(value.extraCreditsUsd) : "0");
  const [day, setDay] = useState(value ? String(value.day) : "");
  const [zone, setZone] = useState(value?.timeZone ?? "Asia/Shanghai");
  const [saving, setSaving] = useState(false), [error, setError] = useState<string | null>(null);
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault(); setSaving(true); setError(null);
    const settings = { monthlyUsd: Number(monthly), extraCreditsUsd: Number(extra), day: Number(day), timeZone: zone };
    try { await saveBillingSettings(settings); onSave(settings); }
    catch (cause) { setError(cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "保存失败，请重试。"); }
    finally { setSaving(false); }
  }
  return <details className="du-billing-settings" key={JSON.stringify(value)}>
    <summary>账期设置</summary>
    <form onSubmit={event => void submit(event)}>
      <p>按自己的订阅填写，仅保存在这台 Mac。月末不足扣款日时，按当月最后一天估算。</p>
      <label>月订阅费（USD）<input type="number" min="0.01" max="1000000" step="0.01" required value={monthly} onChange={event => setMonthly(event.target.value)}/></label>
      <label>本账期额外额度（USD）<input type="number" min="0" max="1000000" step="0.01" required value={extra} onChange={event => setExtra(event.target.value)}/></label>
      <label>每月扣款日<input type="number" min="1" max="31" step="1" required value={day} onChange={event => setDay(event.target.value)}/></label>
      <label>账期时区<select value={zone} onChange={event => setZone(event.target.value)}>{billingTimeZones.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label>
      <p>额外额度为本账期固定比较值；下个账期请自行更新。</p>
      {error && <p role="alert">{error}</p>}
      <button type="submit" disabled={saving}>{saving ? "正在保存…" : "保存账期设置"}</button>
    </form>
  </details>;
}
