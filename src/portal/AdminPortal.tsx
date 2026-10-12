import WalletHistory, { type WalletHistoryData } from './WalletHistory';
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { billingState, money, type Wallet } from "./billing";

type User = { id: string; email: string; enabled: boolean; created_at: number };
type Summary = { users: User[] };
type Detail = { history: WalletHistoryData; key: { id: string; enabled: boolean; total_requests: number; total_tokens_used: number } | null; wallet: Wallet };
async function adminApi<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const response = await fetch(`/api/portal${path}`, {
    method, cache: "no-store",
    headers: { Authorization: `Bearer ${sessionStorage.getItem("abv_admin_api_key") || ""}`, ...(body ? { "Content-Type": "application/json" } : {}) },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  if (!response.ok) { const result = await response.json().catch(() => ({})); throw new Error(result.error?.message || "管理操作失败"); }
  return response.status === 204 ? undefined as T : response.json();
}
export default function AdminPortal() {
  const [summary, setSummary] = useState<Summary>({ users: [] });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [detail, setDetail] = useState<{ id: string; email: string; data: Detail } | null>(null);
  const reload = useCallback(async () => { setSummary(await adminApi<Summary>("/users")); }, []);
  useEffect(() => { reload().catch((e) => setError(e.message)); }, [reload]);
  async function act(action: () => Promise<void>) {
    setBusy(true); setError("");
    try { await action(); await reload(); } catch (e) { setError(e instanceof Error ? e.message : "操作失败"); } finally { setBusy(false); }
  }
  return <div className="flex-1 min-w-0 overflow-auto p-6 space-y-6">
    <div className="flex justify-between items-center"><div><h1 className="text-2xl font-semibold">注册用户</h1>
      <p className="text-sm opacity-60 mt-1">管理账户状态、余额和充值。密钥的到期时间、IP 和访问时段在用户 Token 页面管理。</p></div>
      <button className="btn btn-sm" disabled={busy} onClick={() => act(async () => {})}>刷新</button></div>
    <Link className="btn btn-sm" to="/user-token">管理用户 Token</Link>
    {error && <p role="alert" className="text-error">{error}</p>}
    {detail && <div className="p-4 rounded-xl bg-base-200">
      {detail.email}：{detail.data.key ? <>累计请求 {detail.data.key.total_requests.toLocaleString()} · 用量 {detail.data.key.total_tokens_used.toLocaleString()} token · {detail.data.key.enabled ? "密钥正常" : "密钥已停用"}</> : "尚无密钥"}
      {detail.data.wallet.enabled && <p className="mt-2">余额 {money(detail.data.wallet.balance_nano)} · 累计消费 {money(detail.data.wallet.spent_nano)}</p>}
      <button className="btn btn-xs ml-3" onClick={() => setDetail(null)}>关闭</button>
      <WalletHistory history={detail.data.history} busy={busy} action={(id, action, payment_reference) => { void act(async () => { await adminApi(`/refunds/${id}`, "POST", { action, payment_reference }); setDetail({ ...detail, data: await adminApi<Detail>(`/users/${detail.id}`) }); }); }} />
      {detail.data.wallet.review_count > 0 && <p className="text-warning mt-2">{detail.data.wallet.review_count} 笔费用待核对；仅根据上游记录核对用量，缺少证据时可按失败释放。</p>}
      <div className="overflow-auto mt-3"><table className="table table-xs"><thead><tr><th>请求 / 模型</th><th>普通输入 / 缓存 / 输出</th><th>状态 / 费用</th><th>操作</th></tr></thead>
        <tbody>{detail.data.wallet.usage.map(row => <tr key={row.id}><td><code>{row.id}</code><div>{row.model}</div></td><td>{row.uncached_tokens ?? "—"} / {row.cached_tokens ?? "—"} / {row.output_tokens ?? "—"}
          {row.breakdown && <div>{money(row.breakdown.input_nano)} + {money(row.breakdown.cached_nano)} + {money(row.breakdown.output_nano)}</div>}</td>
          <td>{billingState[row.state] || row.state}{row.status != null && ` · HTTP ${row.status}`}<div>{row.charged_nano == null ? "—" : money(row.charged_nano)}</div></td>
          <td>{row.state === "review" && <button className="btn btn-xs" disabled={busy} onClick={() => {
            const raw = window.prompt("填写已核实的 HTTP 状态、总输入（含缓存）、缓存、输出 token，使用逗号分隔；如确认失败未收费，填写 502,0,0,0");
            if (raw == null) return;
            const values = raw.split(",").map(v => Number(v.trim()));
            if (values.length !== 4 || values.some(v => !Number.isSafeInteger(v) || v < 0) || values[0] < 200 || values[0] > 599 || values[2] > values[1] || values.slice(1).some(v => v > 4294967295)) { setError("状态或用量格式不正确"); return; }
            if (!window.confirm(`确认核对请求 ${row.id}？成功状态将按所填用量扣费。`)) return;
            void act(async () => { await adminApi(`/requests/${row.id}/settle`, "POST", { status: values[0], input_tokens: values[1], cached_tokens: values[2], output_tokens: values[3] }); setDetail({ ...detail, data: await adminApi<Detail>(`/users/${detail.id}`) }); });
          }}>核对结算</button>}</td></tr>)}</tbody></table></div>
    </div>}
    <div className="overflow-auto"><table className="table"><thead><tr><th>邮箱</th><th>状态</th><th>注册时间（北京时间）</th><th>操作</th></tr></thead>
      <tbody>{summary.users.map((user) => <tr key={user.id}><td>{user.email}</td><td>{user.enabled ? "正常" : "已停用"}</td>
        <td>{new Date(user.created_at * 1000).toLocaleString("zh-CN", { timeZone: "Asia/Shanghai" })}</td><td className="space-x-2">
          <button className="btn btn-xs" disabled={busy} onClick={() => act(async () => { setDetail({ id: user.id, email: user.email, data: await adminApi<Detail>(`/users/${user.id}`) }); })}>余额 / 用量</button>
          <button className="btn btn-xs" disabled={busy} onClick={() => {
            const amount = window.prompt(`为 ${user.email} 充值（元）`);
            if (amount == null) return;
            if (!/^\d{1,7}(\.\d{1,2})?$/.test(amount) || Number(amount) <= 0 || Number(amount) > 1000000) { setError("请输入 0.01–1,000,000 元，最多两位小数"); return; }
            const reference = window.prompt("填写本次实际充值流水号（重试时使用同一流水号，避免重复充值）");
            if (!reference?.trim() || reference.length > 128) return;
            void act(async () => { await adminApi(`/users/${user.id}/credits`, "POST", { amount_cents: Math.round(Number(amount) * 100), reference }); setDetail({ id: user.id, email: user.email, data: await adminApi<Detail>(`/users/${user.id}`) }); });
          }}>充值</button>
          <button className="btn btn-xs" disabled={busy} onClick={() => act(async () => { await adminApi(`/users/${user.id}`, "PATCH", { enabled: !user.enabled }); setDetail(null); })}>{user.enabled ? "停用账号" : "启用账号"}</button>
        </td></tr>)}</tbody></table>
      {!summary.users.length && <p className="text-center opacity-60 p-8">暂无注册用户</p>}
    </div>
  </div>;
}
