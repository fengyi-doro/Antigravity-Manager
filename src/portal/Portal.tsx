import { useCallback, useEffect, useRef, useState } from "react";
import "./portal.css";

type Config = {
  registration_open: boolean;
  payment_enabled: boolean;
  api_base_url: string;
  models: string[];
  packages: { cents: number; tokens: number }[];
};
type Order = {
  id: string;
  amount_cents: number;
  tokens: number;
  channel: string;
  state: string;
  created_at: number;
};
type Snapshot = {
  user: { email: string };
  csrf: string;
  data: {
    gift_available: number;
    paid_available: number;
    reserved: number;
    debt: number;
    available: number;
    gift_expires_at: number;
    key: { suffix: string } | null;
    usage: {
      id: string;
      model: string;
      charged: number | null;
      state: string;
      created_at: number;
    }[];
    orders: Order[];
  };
};
const number = (n: number) => new Intl.NumberFormat("zh-CN").format(n);
const date = (n: number) =>
  new Date(n * 1000).toLocaleString("zh-CN", {
    timeZone: "Asia/Shanghai",
    hour12: false,
  });
const statusName: Record<string, string> = {
  pending: "处理中",
  review: "待对账",
  settled: "已结算",
  released: "未扣费",
  paid: "已到账",
  closed: "已关闭",
};

async function api<T>(
  path: string,
  method = "GET",
  body?: unknown,
  csrf?: string,
): Promise<T> {
  const response = await fetch(`/portal/api${path}`, {
    method,
    credentials: "same-origin",
    headers: {
      ...(body ? { "Content-Type": "application/json" } : {}),
      ...(csrf ? { "X-Portal-CSRF": csrf } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  if (response.status === 204) return undefined as T;
  const result = await response.json().catch(() => ({}));
  if (!response.ok)
    throw new Error(
      result.error?.message ||
        (response.status === 401 ? "请先登录" : "请求失败，请稍后重试"),
    );
  return result as T;
}

export default function Portal() {
  const [config, setConfig] = useState<Config | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [ready, setReady] = useState(false);
  const [register, setRegister] = useState(
    window.location.pathname === "/register",
  );
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [fullKey, setFullKey] = useState("");
  const [recharge, setRecharge] = useState(false);
  const [amount, setAmount] = useState(100);
  const [channel, setChannel] = useState("wxpay");
  const [tab, setTab] = useState<"usage" | "orders">("usage");
  const [keyAction, setKeyAction] = useState<"rotate" | "revoke" | null>(null);
  const checkoutRef = useRef<HTMLDivElement>(null);
  const idempotency = useRef<string | null>(null);
  const reload = useCallback(async () => {
    const data = await api<Snapshot>("/me");
    setSnapshot(data);
    return data;
  }, []);

  useEffect(() => {
    let active = true;
    Promise.all([
      api<Config>("/config"),
      api<Snapshot>("/me").catch(() => null),
    ])
      .then(([c, s]) => {
        if (active) {
          setConfig(c);
          setSnapshot(s);
        }
      })
      .catch((e) => {
        if (active) setError(e.message);
      })
      .finally(() => {
        if (active) setReady(true);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!snapshot) return;
    const timer = window.setInterval(() => {
      reload().catch(() => {});
    }, 30000);
    return () => window.clearInterval(timer);
  }, [Boolean(snapshot), reload]);

  useEffect(() => {
    const order = new URLSearchParams(window.location.search).get("order");
    if (!order || !snapshot?.csrf) return;
    let cancelled = false;
    let attempts = 0;
    const check = async () => {
      try {
        const result = await api<Order>(
          `/orders/${encodeURIComponent(order)}/reconcile`,
          "POST",
          {},
          snapshot.csrf,
        );
        if (cancelled) return;
        if (result.state === "paid") {
          setNotice("支付成功，额度已到账");
          window.history.replaceState({}, "", "/console");
          await reload();
          window.clearInterval(timer);
        }
      } catch (e) {
        if (!cancelled)
          setNotice(e instanceof Error ? e.message : "等待支付结果");
      }
      if (++attempts >= 60) window.clearInterval(timer);
    };
    const timer = window.setInterval(check, 5000);
    void check();
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [snapshot?.csrf, reload]);

  useEffect(() => {
    if (!recharge && !keyAction) return;
    const previous = document.activeElement as HTMLElement | null;
    const dialog = document.querySelector<HTMLElement>(".portal-dialog");
    const controls = () =>
      Array.from(
        dialog?.querySelectorAll<HTMLElement>(
          "button:not(:disabled), input:not(:disabled), a[href]",
        ) || [],
      );
    controls()[0]?.focus();
    const keyboard = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) {
        setRecharge(false);
        setKeyAction(null);
      }
      if (event.key === "Tab") {
        const items = controls();
        const first = items[0],
          last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", keyboard);
    return () => {
      document.removeEventListener("keydown", keyboard);
      previous?.focus();
    };
  }, [recharge, keyAction, busy]);

  async function perform(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (e) {
      setError(e instanceof Error ? e.message : "操作失败");
    } finally {
      setBusy(false);
    }
  }
  async function authenticate(event: React.FormEvent) {
    event.preventDefault();
    await perform(async () => {
      if (register && password !== confirmation)
        throw new Error("两次输入的密码不一致");
      const result = await api<{ api_key?: string }>(
        register ? "/auth/register" : "/auth/login",
        "POST",
        { email, password },
      );
      setPassword("");
      setConfirmation("");
      if (result.api_key) setFullKey(result.api_key);
      await reload();
      window.history.replaceState({}, "", "/console");
    });
  }
  async function copy(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      setNotice("已复制");
    } catch {
      setError("复制失败，请手动选择文本复制");
    }
  }
  async function pay() {
    if (!snapshot) return;
    await perform(async () => {
      idempotency.current ||= crypto.randomUUID();
      const result = await api<{
        checkout: { action: string; fields: Record<string, string> };
      }>(
        "/orders",
        "POST",
        { amount_cents: amount, channel, idempotency_key: idempotency.current },
        snapshot.csrf,
      );
      const form = document.createElement("form");
      form.action = result.checkout.action;
      form.method = "POST";
      for (const [name, value] of Object.entries(result.checkout.fields)) {
        const input = document.createElement("input");
        input.type = "hidden";
        input.name = name;
        input.value = value;
        form.appendChild(input);
      }
      checkoutRef.current?.replaceChildren(form);
      form.submit();
    });
  }

  const data = snapshot?.data;
  return (
    <div className="portal-app">
      <div className="portal-shell">
        <header className="portal-header">
          <a href="/console" className="portal-brand">
            Antigravity <span>API</span>
          </a>
          {snapshot && (
            <div className="portal-account">
              <span>{snapshot.user.email}</span>
              <button
                disabled={busy}
                className="portal-text-button"
                onClick={() =>
                  perform(async () => {
                    await api("/auth/logout", "POST", {}, snapshot.csrf);
                    setSnapshot(null);
                    setFullKey("");
                    window.history.replaceState({}, "", "/console");
                  })
                }
              >
                退出
              </button>
            </div>
          )}
        </header>
        {error && (
          <div className="portal-alert portal-error" role="alert">
            {error}
          </div>
        )}
        {notice && (
          <div className="portal-alert" role="status">
            {notice}
          </div>
        )}
        {!ready ? (
          <p className="portal-loading">正在加载…</p>
        ) : !snapshot ? (
          <main className="portal-auth">
            <div className="portal-eyebrow">开始使用 API</div>
            <h1>{register ? "创建账号" : "欢迎回来"}</h1>
            <p className="portal-muted">
              注册赠送 500 万 token，仅注册当天有效
              <br />
              北京时间 24:00 到期
            </p>
            <form onSubmit={authenticate}>
              <label>
                邮箱
                <input
                  type="email"
                  autoComplete="username"
                  required
                  maxLength={254}
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                />
              </label>
              <label>
                密码
                <input
                  type="password"
                  autoComplete={register ? "new-password" : "current-password"}
                  required
                  minLength={register ? 10 : undefined}
                  maxLength={128}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder={register ? "至少 10 个字符" : ""}
                />
              </label>
              {register && (
                <label>
                  确认密码
                  <input
                    type="password"
                    autoComplete="new-password"
                    required
                    value={confirmation}
                    onChange={(e) => setConfirmation(e.target.value)}
                  />
                </label>
              )}
              <button
                className="portal-primary"
                type="submit"
                disabled={
                  busy || !config || (register && !config.registration_open)
                }
              >
                {busy
                  ? "正在处理…"
                  : register
                    ? config?.registration_open
                      ? "注册并领取额度"
                      : "注册暂未开放"
                    : "登录"}
              </button>
            </form>
            <button
              className="portal-text-button"
              onClick={() => {
                setRegister(!register);
                setError("");
              }}
            >
              {register ? "已有账号？登录" : "还没有账号？注册"}
            </button>
          </main>
        ) : (
          data && (
            <main>
              <div className="portal-title">
                <div>
                  <div className="portal-eyebrow">我的用量</div>
                  <h1>简单接入，按量使用</h1>
                </div>
                <button
                  className="portal-primary"
                  onClick={() => {
                    setRecharge(true);
                    idempotency.current = null;
                  }}
                  disabled={!config?.payment_enabled}
                >
                  充值
                </button>
              </div>
              <section className="portal-balance" aria-label="账户额度">
                <div>
                  <span className="portal-muted">可用额度</span>
                  <strong>
                    {number(data.available)} <small>token</small>
                  </strong>
                </div>
                <div>
                  <span className="portal-muted">当日赠送剩余</span>
                  <strong>{number(data.gift_available)}</strong>
                  <small>
                    {Date.now() / 1000 < data.gift_expires_at
                      ? `${date(data.gift_expires_at)} 到期`
                      : "注册赠送已到期"}
                  </small>
                </div>
                <div>
                  <span className="portal-muted">充值剩余额度</span>
                  <strong>{number(data.paid_available)}</strong>
                  <small>长期有效 · 优先使用赠送额度</small>
                </div>
              </section>
              {(data.reserved > 0 || data.debt > 0) && (
                <p className="portal-muted">
                  请求预留：{number(data.reserved)} token
                  {data.debt > 0 && ` · 待补足：${number(data.debt)} token`}
                  。异常中断的请求会保留预算，等待对账。
                </p>
              )}
              <section className="portal-section">
                <h2>API 接入</h2>
                <div className="portal-value-row">
                  <div>
                    <span className="portal-muted">接口地址</span>
                    <code>{config?.api_base_url}</code>
                  </div>
                  <button
                    className="portal-secondary"
                    onClick={() => copy(config?.api_base_url || "")}
                  >
                    复制地址
                  </button>
                </div>
                <div className="portal-value-row">
                  <div>
                    <span className="portal-muted">API Key</span>
                    <code>
                      {fullKey ||
                        (data.key
                          ? `sk-portal-••••••••${data.key.suffix}`
                          : "尚无有效密钥")}
                    </code>
                  </div>
                  <div className="portal-actions">
                    {fullKey && (
                      <button
                        className="portal-secondary"
                        onClick={() => copy(fullKey)}
                      >
                        复制密钥
                      </button>
                    )}
                    <button
                      disabled={busy}
                      className="portal-secondary"
                      onClick={() => setKeyAction("rotate")}
                    >
                      {data.key ? "重置" : "创建密钥"}
                    </button>
                    {data.key && (
                      <button
                        disabled={busy}
                        className="portal-text-button"
                        onClick={() => setKeyAction("revoke")}
                      >
                        停用
                      </button>
                    )}
                  </div>
                </div>
                {fullKey ? (
                  <p className="portal-key-note">
                    请保存这份密钥，离开页面后无法再次查看。
                    <button
                      className="portal-text-button"
                      onClick={() => setFullKey("")}
                    >
                      我已保存
                    </button>
                  </p>
                ) : (
                  <p className="portal-muted">
                    密钥仅创建时完整显示。重置后，原密钥立即失效。
                  </p>
                )}
                <details>
                  <summary>可用模型与使用方式</summary>
                  <p className="portal-muted">
                    {config?.models?.length
                      ? config.models.join("、")
                      : "管理员正在配置可用模型"}
                  </p>
                  <p className="portal-muted">
                    在兼容客户端填写上面的接口地址和 API
                    Key。当前支持文本请求；输入与输出 token 按 1:1 扣减。
                  </p>
                </details>
              </section>
              <section className="portal-section">
                <div
                  className="portal-tabs"
                  role="tablist"
                  aria-label="账户记录"
                >
                  <button
                    role="tab"
                    aria-selected={tab === "usage"}
                    onClick={() => setTab("usage")}
                  >
                    用量记录
                  </button>
                  <button
                    role="tab"
                    aria-selected={tab === "orders"}
                    onClick={() => setTab("orders")}
                  >
                    充值记录
                  </button>
                  <button
                    className="portal-text-button"
                    onClick={() =>
                      perform(async () => {
                        await reload();
                      })
                    }
                    disabled={busy}
                  >
                    刷新
                  </button>
                </div>
                <div className="portal-records" role="tabpanel">
                  {tab === "usage" ? (
                    data.usage.length ? (
                      data.usage.map((row) => (
                        <div className="portal-record" key={row.id}>
                          <div>
                            <strong>{row.model}</strong>
                            <small>{date(row.created_at)}</small>
                          </div>
                          <div>
                            <strong>
                              {row.charged === null
                                ? "—"
                                : `${number(row.charged)} token`}
                            </strong>
                            <small>{statusName[row.state] || row.state}</small>
                          </div>
                        </div>
                      ))
                    ) : (
                      <p className="portal-empty">还没有调用记录</p>
                    )
                  ) : data.orders.length ? (
                    data.orders.map((row) => (
                      <div className="portal-record" key={row.id}>
                        <div>
                          <strong>
                            ¥{(row.amount_cents / 100).toFixed(2)} ·{" "}
                            {number(row.tokens)} token
                          </strong>
                          <small>
                            {date(row.created_at)} ·{" "}
                            {row.channel === "wxpay" ? "微信" : "支付宝"}
                          </small>
                        </div>
                        <div>
                          <strong>{statusName[row.state] || row.state}</strong>
                          {row.state === "pending" && (
                            <button
                              className="portal-text-button"
                              disabled={busy || !config?.payment_enabled}
                              onClick={() =>
                                perform(async () => {
                                  await api(
                                    `/orders/${row.id}/reconcile`,
                                    "POST",
                                    {},
                                    snapshot.csrf,
                                  );
                                  await reload();
                                })
                              }
                            >
                              核对到账
                            </button>
                          )}
                        </div>
                      </div>
                    ))
                  ) : (
                    <p className="portal-empty">还没有充值记录</p>
                  )}
                </div>
              </section>
              <footer className="portal-footer">
                ¥1 = 1,000,000 token
                {!config?.payment_enabled && " · 充值暂未开放"}
              </footer>
            </main>
          )
        )}
        {recharge && (
          <div className="portal-overlay">
            <section
              className="portal-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="recharge-title"
            >
              <h2 id="recharge-title">充值额度</h2>
              <p className="portal-muted">
                1 元 = 100 万 token，充值额度长期有效
              </p>
              <div className="portal-packages">
                {config?.packages.map((pkg) => (
                  <button
                    className={amount === pkg.cents ? "selected" : ""}
                    key={pkg.cents}
                    onClick={() => {
                      setAmount(pkg.cents);
                      idempotency.current = null;
                    }}
                  >
                    <strong>¥{pkg.cents / 100}</strong>
                    <span>{number(pkg.tokens / 10000)} 万 token</span>
                  </button>
                ))}
              </div>
              <fieldset className="portal-channels">
                <legend>支付方式</legend>
                <label>
                  <input
                    type="radio"
                    name="channel"
                    value="wxpay"
                    checked={channel === "wxpay"}
                    onChange={() => {
                      setChannel("wxpay");
                      idempotency.current = null;
                    }}
                  />
                  微信支付
                </label>
                <label>
                  <input
                    type="radio"
                    name="channel"
                    value="alipay"
                    checked={channel === "alipay"}
                    onChange={() => {
                      setChannel("alipay");
                      idempotency.current = null;
                    }}
                  />
                  支付宝
                </label>
              </fieldset>
              <div className="portal-dialog-actions">
                <button
                  className="portal-secondary"
                  disabled={busy}
                  onClick={() => setRecharge(false)}
                >
                  取消
                </button>
                <button
                  className="portal-primary"
                  disabled={busy || !config?.payment_enabled}
                  onClick={pay}
                >
                  {busy ? "正在创建订单…" : `前往支付 ¥${amount / 100}`}
                </button>
              </div>
            </section>
          </div>
        )}
        {keyAction && (
          <div className="portal-overlay">
            <section
              className="portal-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="key-title"
            >
              <h2 id="key-title">
                {keyAction === "rotate" ? "创建新密钥" : "停用密钥"}
              </h2>
              <p>现有密钥将立即失效，已经开始的请求仍会结算。</p>
              <div className="portal-dialog-actions">
                <button
                  className="portal-secondary"
                  disabled={busy}
                  onClick={() => setKeyAction(null)}
                >
                  取消
                </button>
                <button
                  className="portal-primary"
                  disabled={busy}
                  onClick={() =>
                    perform(async () => {
                      if (!snapshot) return;
                      if (keyAction === "rotate") {
                        const result = await api<{ api_key: string }>(
                          "/key",
                          "POST",
                          {},
                          snapshot.csrf,
                        );
                        setFullKey(result.api_key);
                      } else {
                        await api("/key", "DELETE", {}, snapshot.csrf);
                        setFullKey("");
                      }
                      setKeyAction(null);
                      await reload();
                    })
                  }
                >
                  确认
                </button>
              </div>
            </section>
          </div>
        )}
        <div ref={checkoutRef} hidden />
      </div>
    </div>
  );
}
