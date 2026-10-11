import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";

type User = { id: string; email: string; enabled: boolean; created_at: number };
type Summary = { users: User[] };
type Detail = { key: { id: string; enabled: boolean; total_requests: number; total_tokens_used: number } | null };
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
  const [detail, setDetail] = useState<{ email: string; data: Detail } | null>(null);
  const reload = useCallback(async () => { setSummary(await adminApi<Summary>("/users")); }, []);
  useEffect(() => { reload().catch((e) => setError(e.message)); }, [reload]);
  async function act(action: () => Promise<void>) {
    setBusy(true); setError("");
    try { await action(); await reload(); } catch (e) { setError(e instanceof Error ? e.message : "操作失败"); } finally { setBusy(false); }
  }
  return <div className="flex-1 min-w-0 overflow-auto p-6 space-y-6">
    <div className="flex justify-between items-center"><div><h1 className="text-2xl font-semibold">注册用户</h1>
      <p className="text-sm opacity-60 mt-1">管理账户状态，查看 API Key 用量。密钥的到期时间、IP 和访问时段在用户 Token 页面管理。</p></div>
      <button className="btn btn-sm" disabled={busy} onClick={() => act(async () => {})}>刷新</button></div>
    <Link className="btn btn-sm" to="/user-token">管理用户 Token</Link>
    {error && <p role="alert" className="text-error">{error}</p>}
    {detail && <div className="p-4 rounded-xl bg-base-200">
      {detail.email}：{detail.data.key ? <>累计请求 {detail.data.key.total_requests.toLocaleString()} · 用量 {detail.data.key.total_tokens_used.toLocaleString()} token · {detail.data.key.enabled ? "密钥正常" : "密钥已停用"}</> : "尚无密钥"}
      <button className="btn btn-xs ml-3" onClick={() => setDetail(null)}>关闭</button></div>}
    <div className="overflow-auto"><table className="table"><thead><tr><th>邮箱</th><th>状态</th><th>注册时间（北京时间）</th><th>操作</th></tr></thead>
      <tbody>{summary.users.map((user) => <tr key={user.id}><td>{user.email}</td><td>{user.enabled ? "正常" : "已停用"}</td>
        <td>{new Date(user.created_at * 1000).toLocaleString("zh-CN", { timeZone: "Asia/Shanghai" })}</td><td className="space-x-2">
          <button className="btn btn-xs" disabled={busy} onClick={() => act(async () => { setDetail({ email: user.email, data: await adminApi<Detail>(`/users/${user.id}`) }); })}>用量</button>
          <button className="btn btn-xs" disabled={busy} onClick={() => act(async () => { await adminApi(`/users/${user.id}`, "PATCH", { enabled: !user.enabled }); setDetail(null); })}>{user.enabled ? "停用账号" : "启用账号"}</button>
        </td></tr>)}</tbody></table>
      {!summary.users.length && <p className="text-center opacity-60 p-8">暂无注册用户</p>}
    </div>
  </div>;
}
