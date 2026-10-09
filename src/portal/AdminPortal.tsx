import { useCallback, useEffect, useState } from "react";

type User = { id: string; email: string; enabled: boolean; created_at: number };
type Pending = {
  id: string;
  user_id: string;
  model: string;
  reserved: number;
  created_at: number;
};
type Summary = { users: User[]; review: Pending[] };
async function adminApi<T>(
  path: string,
  method = "GET",
  body?: unknown,
): Promise<T> {
  const key = sessionStorage.getItem("abv_admin_api_key") || "";
  const response = await fetch(`/api/portal${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${key}`,
      ...(body ? { "Content-Type": "application/json" } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  if (!response.ok) {
    const result = await response.json().catch(() => ({}));
    throw new Error(result.error?.message || "管理操作失败");
  }
  return response.status === 204 ? (undefined as T) : response.json();
}
export default function AdminPortal() {
  const [summary, setSummary] = useState<Summary>({ users: [], review: [] });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<Pending | null>(null);
  const [input, setInput] = useState("0");
  const [output, setOutput] = useState("0");
  const [reason, setReason] = useState("");
  const [detail, setDetail] = useState<{
    email: string;
    data: { available: number; reserved: number; debt: number };
  } | null>(null);
  const reload = useCallback(async () => {
    setSummary(await adminApi<Summary>("/users"));
  }, []);
  useEffect(() => {
    reload().catch((e) => setError(e.message));
  }, [reload]);
  async function act(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
      await reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : "操作失败");
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="flex-1 overflow-auto p-6 space-y-6">
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-2xl font-semibold">注册用户</h1>
          <p className="text-sm opacity-60 mt-1">
            用户状态、余额与异常请求对账
          </p>
        </div>
        <button
          className="btn btn-sm"
          onClick={() => act(reload)}
          disabled={busy}
        >
          刷新
        </button>
      </div>
      {error && (
        <p role="alert" className="text-error">
          {error}
        </p>
      )}
      {detail && (
        <div className="p-4 rounded-xl bg-base-200">
          {detail.email}：可用 {detail.data.available.toLocaleString()} · 预留{" "}
          {detail.data.reserved.toLocaleString()} · 待补足{" "}
          {detail.data.debt.toLocaleString()} token{" "}
          <button className="btn btn-xs ml-3" onClick={() => setDetail(null)}>
            关闭
          </button>
        </div>
      )}
      <div className="overflow-auto">
        <table className="table">
          <thead>
            <tr>
              <th>邮箱</th>
              <th>状态</th>
              <th>注册时间（北京时间）</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {summary.users.map((user) => (
              <tr key={user.id}>
                <td>{user.email}</td>
                <td>{user.enabled ? "正常" : "已停用"}</td>
                <td>
                  {new Date(user.created_at * 1000).toLocaleString("zh-CN", {
                    timeZone: "Asia/Shanghai",
                  })}
                </td>
                <td className="space-x-2">
                  <button
                    className="btn btn-xs"
                    disabled={busy}
                    onClick={() =>
                      act(async () => {
                        setDetail({
                          email: user.email,
                          data: await adminApi(`/users/${user.id}`),
                        });
                      })
                    }
                  >
                    余额
                  </button>
                  <button
                    className="btn btn-xs"
                    disabled={busy}
                    onClick={() =>
                      act(async () => {
                        await adminApi(`/users/${user.id}`, "PATCH", {
                          enabled: !user.enabled,
                        });
                      })
                    }
                  >
                    {user.enabled ? "停用账号" : "启用账号"}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!summary.users.length && (
          <p className="text-center opacity-60 p-8">暂无注册用户</p>
        )}
      </div>
      <div>
        <h2 className="text-lg font-medium">待对账请求</h2>
        <p className="text-sm opacity-60 mt-1">
          缺少完整用量的请求保留预留额度。根据上游记录填写实际输入、输出
          token；确认未产生用量时填写 0。
        </p>
      </div>
      <div className="overflow-auto">
        <table className="table">
          <thead>
            <tr>
              <th>请求</th>
              <th>模型</th>
              <th>预留 token</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {summary.review.map((row) => (
              <tr key={row.id}>
                <td>
                  <code title={row.id}>{row.id.slice(0, 18)}…</code>
                </td>
                <td>{row.model}</td>
                <td>{row.reserved.toLocaleString()}</td>
                <td>
                  <button
                    className="btn btn-xs"
                    onClick={() => {
                      setSelected(row);
                      setInput("0");
                      setOutput("0");
                      setReason("");
                    }}
                  >
                    对账
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!summary.review.length && (
          <p className="text-center opacity-60 p-8">没有待对账请求</p>
        )}
      </div>
      {selected && (
        <div className="modal modal-open">
          <form
            className="modal-box"
            onSubmit={(event) => {
              event.preventDefault();
              void act(async () => {
                await adminApi(`/requests/${selected.id}/settle`, "POST", {
                  input_tokens: Number(input),
                  output_tokens: Number(output),
                  reason,
                });
                setSelected(null);
              });
            }}
          >
            <h3 className="font-semibold text-lg">确认实际用量</h3>
            <div className="grid grid-cols-2 gap-4 my-5">
              <label>
                输入 token
                <input
                  className="input input-bordered w-full mt-1"
                  type="number"
                  min="0"
                  step="1"
                  required
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                />
              </label>
              <label>
                输出 token
                <input
                  className="input input-bordered w-full mt-1"
                  type="number"
                  min="0"
                  step="1"
                  required
                  value={output}
                  onChange={(e) => setOutput(e.target.value)}
                />
              </label>
            </div>
            <label>
              对账依据
              <textarea
                className="textarea textarea-bordered w-full mt-1"
                required
                minLength={4}
                maxLength={500}
                value={reason}
                onChange={(e) => setReason(e.target.value)}
              />
            </label>
            <div className="modal-action">
              <button
                type="button"
                className="btn"
                disabled={busy}
                onClick={() => setSelected(null)}
              >
                取消
              </button>
              <button className="btn btn-primary" disabled={busy}>
                确认结算
              </button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
}
