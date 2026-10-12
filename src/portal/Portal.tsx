import { useCallback, useEffect, useState } from "react";
import "./portal.css";
import { billingState, money, type Wallet } from "./billing";

type Usage = { id: string; model: string; input_tokens: number | null; output_tokens: number | null; request_time: number; status: number };
type NativeToken = { id: string; suffix: string; enabled: boolean; expires_at: number | null; total_requests: number; total_tokens_used: number; last_used_at: number | null; usage: Usage[] };
type Legacy = { paid_available: number; usage: { id: string; model: string; charged: number | null; created_at: number }[]; orders: { id: string; amount_cents: number; state: string; created_at: number }[] };
type Snapshot = { user: { email: string }; csrf: string; data: { key: NativeToken | null; legacy: Legacy; wallet: Wallet } };
type Config = { registration_open: boolean; api_base_url: string };
const number = (value: number) => new Intl.NumberFormat("zh-CN").format(value);
const date = (value: number) => new Date(value * 1000).toLocaleString("zh-CN", { timeZone: "Asia/Shanghai", hour12: false });
const orderStatus: Record<string, string> = { pending: "待支付", paid: "已到账", closed: "已关闭" };

async function api<T>(path: string, method = "GET", body?: unknown, csrf?: string): Promise<T> {
  const response = await fetch(`/portal/api${path}`, {
    method, credentials: "same-origin", cache: "no-store",
    headers: { ...(body ? { "Content-Type": "application/json" } : {}), ...(csrf ? { "X-Portal-CSRF": csrf } : {}) },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  if (response.status === 204) return undefined as T;
  const result = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(result.error?.message || (response.status === 401 ? "请先登录" : "请求失败，请稍后重试"));
  return result as T;
}

function WelcomeCharacter() {
  const [greeting, setGreeting] = useState(0);
  useEffect(() => {
    if (!greeting) return;
    const timer = window.setTimeout(() => setGreeting(0), 3200);
    return () => window.clearTimeout(timer);
  }, [greeting]);

  return (
    <div className="portal-welcome">
      {greeting > 0 && <div key={greeting} id="portal-welcome-message" className="portal-welcome-bubble" role="status">欢迎进入幻想乡</div>}
      <button type="button" className="portal-welcome-character" aria-label="点击人物，欢迎进入幻想乡"
        aria-describedby={greeting > 0 ? "portal-welcome-message" : undefined} onClick={() => setGreeting((value) => value + 1)}>
        <img src="/images/portal/koishi-welcome-v2.png" alt="" width={212} height={212} draggable={false} />
      </button>
    </div>
  );
}

export default function Portal() {
  const [config, setConfig] = useState<Config | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [ready, setReady] = useState(false);
  const [register, setRegister] = useState(window.location.pathname === "/register");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [fullKey, setFullKey] = useState<{ id: string; value: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const reload = useCallback(async () => { const data = await api<Snapshot>("/me"); setSnapshot(data); return data; }, []);
  useEffect(() => {
    let active = true;
    Promise.all([api<Config>("/config"), api<Snapshot>("/me").catch(() => null)])
      .then(([c, s]) => { if (active) { setConfig(c); setSnapshot(s); } })
      .catch((e) => { if (active) setError(e.message); })
      .finally(() => { if (active) setReady(true); });
    return () => { active = false; };
  }, []);
  const signedIn = Boolean(snapshot);
  useEffect(() => {
    if (!signedIn) return;
    const timer = window.setInterval(() => { reload().catch(() => {}); }, 30000);
    return () => window.clearInterval(timer);
  }, [signedIn, reload]);
  async function perform(action: () => Promise<void>) {
    setBusy(true); setError(""); setNotice("");
    try { await action(); } catch (e) { setError(e instanceof Error ? e.message : "操作失败"); } finally { setBusy(false); }
  }
  async function authenticate(event: React.FormEvent) {
    event.preventDefault();
    await perform(async () => {
      if (register && password !== confirmation) throw new Error("两次输入的密码不一致");
      await api(register ? "/auth/register" : "/auth/login", "POST", { email, password });
      setPassword(""); setConfirmation(""); await reload();
      window.history.replaceState({}, "", "/console");
    });
  }
  async function copy(value: string) {
    try { await navigator.clipboard.writeText(value); setNotice("已复制"); } catch { throw new Error("复制失败，请手动选择并复制。"); }
  }
  const key = snapshot?.data.key;
  const shownKey = fullKey?.id === key?.id ? fullKey?.value : undefined;
  const legacy = snapshot?.data.legacy;
  const wallet = snapshot?.data.wallet;
  const expired = key?.expires_at != null && key.expires_at <= Date.now() / 1000;
  const keyStatus = !key ? "尚无密钥" : !key.enabled ? "已停用" : expired ? "已到期" : wallet?.enabled && wallet.balance_nano <= 0 ? "余额不足" : "正常";
  return (
    <div className="portal-app"><div className="portal-shell">
      <header className="portal-header">
        <a href="/console" className="portal-brand">Antigravity API</a>
        {snapshot && <div className="portal-account"><span>{snapshot.user.email}</span><button className="portal-text-button" disabled={busy}
          onClick={() => perform(async () => { await api("/auth/logout", "POST", {}, snapshot.csrf); setSnapshot(null); setFullKey(null); })}>退出登录</button></div>}
      </header>
      {error && <div className="portal-alert portal-error" role="alert">{error}</div>}
      {notice && <div className="portal-alert" role="status">{notice}</div>}
      {!ready ? <p className="portal-loading">正在加载…</p> : !snapshot ? (
        <main className="portal-auth-layout">
          <div className="portal-auth">
            <div className="portal-eyebrow">开始使用 API</div><h1>{register ? "创建账号" : "欢迎回来"}</h1>
            <p className="portal-muted">登录后管理 API Key，查看余额与调用费用。</p>
            <form onSubmit={authenticate}>
              <label>邮箱<input type="email" autoComplete="username" required maxLength={254} value={email} onChange={(e) => setEmail(e.target.value)} /></label>
              <label>密码<input type="password" autoComplete={register ? "new-password" : "current-password"} required minLength={register ? 6 : undefined} maxLength={128} value={password} onChange={(e) => setPassword(e.target.value)} /></label>
              {register && <label>确认密码<input type="password" autoComplete="new-password" required minLength={6} maxLength={128} value={confirmation} onChange={(e) => setConfirmation(e.target.value)} /></label>}
              <button className="portal-primary" disabled={busy || (register && config?.registration_open === false)}>{busy ? "请稍候…" : register ? "注册" : "登录"}</button>
            </form>
            <button className="portal-text-button" onClick={() => { setRegister(!register); setError(""); window.history.replaceState({}, "", register ? "/console" : "/register"); }}>{register ? "已有账号？登录" : "还没有账号？注册"}</button>
          </div>
          <WelcomeCharacter />
        </main>
      ) : (
        <main>
          <div className="portal-title"><div><div className="portal-eyebrow">我的用量</div><h1>API 调用概览</h1></div>
            <button className="portal-secondary" disabled={busy} onClick={() => perform(async () => { await reload(); })}>刷新</button></div>
          <section className="portal-balance" aria-label="当前密钥用量">
            <div><span className="portal-muted">{wallet?.enabled ? "账户余额" : "累计用量"}</span><strong>{wallet?.enabled ? money(wallet.balance_nano) : <>{number(key?.total_tokens_used ?? 0)} <small>token</small></>}</strong><small>{wallet?.enabled ? "余额长期有效 · 联系管理员充值" : "当前 API Key 的累计统计"}</small></div>
            <div><span className="portal-muted">{wallet?.enabled ? "累计消费" : "累计请求"}</span><strong>{wallet?.enabled ? money(wallet.spent_nano) : number(key?.total_requests ?? 0)}</strong><small>最近调用：{key?.last_used_at ? date(key.last_used_at) : "暂无"}</small></div>
            <div><span className="portal-muted">密钥状态</span><strong>{keyStatus}</strong><small>{key?.expires_at ? `${date(key.expires_at)} 到期` : key ? "长期有效" : "请创建密钥"}</small></div>
          </section>
          {wallet?.enabled && <section className="portal-section"><h2>计费标准</h2>
            <div className="portal-record"><span>普通输入，未命中缓存</span><strong>¥0.60 / 百万 token</strong></div>
            <div className="portal-record"><span>缓存命中的输入</span><strong>¥0.06 / 百万 token</strong></div>
            <div className="portal-record"><span>输出，包含思考 token</span><strong>¥3.00 / 百万 token</strong></div>
            <p className="portal-muted">按实际用量扣费，失败请求不收费。所有可用模型采用上述价格。</p>
          </section>}
          <section className="portal-section"><h2>API 接入</h2>
            <div className="portal-value-row"><div><span className="portal-muted">接口地址</span><code>{config?.api_base_url}</code></div><button className="portal-secondary" onClick={() => perform(() => copy(config?.api_base_url || ""))}>复制地址</button></div>
            <div className="portal-value-row"><div><span className="portal-muted">API Key</span><code>{shownKey || (key ? `sk-••••••••${key.suffix}` : "尚无密钥")}</code></div>
              <div className="portal-actions">
                {key && <button className="portal-secondary" disabled={busy} onClick={() => perform(async () => { const result = await api<{ api_key: string }>("/key"); setFullKey({ id: key.id, value: result.api_key }); await copy(result.api_key); })}>查看并复制</button>}
                <button className="portal-secondary" disabled={busy} onClick={() => {
                  if (key && !window.confirm("重置后原密钥会立即停用，用量记录仍保留，原有访问限制继续生效。确认重置？")) return;
                  void perform(async () => { const result = await api<{ api_key: string }>("/key", "POST", {}, snapshot.csrf); const updated = await reload(); if (updated.data.key) setFullKey({ id: updated.data.key.id, value: result.api_key }); setNotice("新密钥已创建，请更新客户端配置。"); });
                }}>{key ? "重置密钥" : "创建密钥"}</button>
                {key?.enabled && <button className="portal-text-button" disabled={busy} onClick={() => {
                  if (!window.confirm("停用后使用此密钥的客户端将无法继续调用，确认停用？")) return;
                  void perform(async () => { await api("/key", "DELETE", undefined, snapshot.csrf); setFullKey(null); await reload(); });
                }}>停用</button>}
                {shownKey && <button className="portal-text-button" onClick={() => setFullKey(null)}>隐藏密钥</button>}
              </div>
            </div>
            <p className="portal-muted">在兼容客户端填写接口地址和 API Key。可用模型通过模型列表接口查询。</p>
          </section>
          <section className="portal-section"><h2>调用记录</h2>
            {Boolean(wallet?.review_count) && <p role="status" className="portal-alert">有 {wallet?.review_count} 笔调用费用待核对，请联系管理员并提供下方请求编号。核对完成前暂停新的计费调用。</p>}
            {wallet?.enabled ? <div className="portal-records">{wallet.usage.length ? wallet.usage.map((row) => <div className="portal-record" key={row.id}>
              <div><strong>{row.model}</strong><small>{date(row.created_at)}</small><small>请求编号 <code>{row.id}</code></small></div>
              <div><strong>普通输入 {row.uncached_tokens == null ? "—" : number(row.uncached_tokens)} · 缓存 {row.cached_tokens == null ? "—" : number(row.cached_tokens)} · 输出 {row.output_tokens == null ? "—" : number(row.output_tokens)}</strong>
                <small>{billingState[row.state] || row.state}{row.charged_nano != null ? ` · ${money(row.charged_nano)}` : ""}{row.status != null ? ` · HTTP ${row.status}` : ""}</small>
                {row.breakdown && <small>普通输入 {money(row.breakdown.input_nano)} + 缓存 {money(row.breakdown.cached_nano)} + 输出 {money(row.breakdown.output_nano)}</small>}
                {row.settled_at != null && <small>结算于 {date(row.settled_at)}</small>}
              </div>
            </div>) : <p className="portal-empty">还没有调用记录</p>}</div> : <div className="portal-records">{key?.usage.length ? key.usage.map((row) => <div className="portal-record" key={row.id}>
              <div><strong>{row.model || "unknown"}</strong><small>{date(row.request_time)}</small></div>
              <div><strong>输入 {number(row.input_tokens ?? 0)} · 输出 {number(row.output_tokens ?? 0)}</strong><small>HTTP {row.status}</small></div>
            </div>) : <p className="portal-empty">还没有调用记录</p>}</div>}
          </section>
          {legacy && (legacy.paid_available > 0 || legacy.orders.length > 0 || legacy.usage.length > 0) && (
            <details className="portal-section"><summary>历史账户记录</summary>
              <p className="portal-muted">独立充值与扣费已停用。以下记录仅供查看，不参与当前 API 用量统计。</p>
              {legacy.paid_available > 0 && <p>历史充值剩余额度：{number(legacy.paid_available)} token</p>}
              {legacy.orders.map((order) => <div className="portal-record" key={order.id}><div><strong>¥{(order.amount_cents / 100).toFixed(2)}</strong><small>{date(order.created_at)}</small></div><span>{orderStatus[order.state] || order.state}</span></div>)}
              {legacy.usage.map((row) => <div className="portal-record" key={row.id}><div><strong>{row.model}</strong><small>{date(row.created_at)}</small></div><span>{row.charged == null ? "未结算" : `${number(row.charged)} token`}</span></div>)}
            </details>
          )}
        </main>
      )}
    </div></div>
  );
}
