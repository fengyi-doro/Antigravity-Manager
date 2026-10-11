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
type ModelSettings = { allowed_models: string[] };
async function adminApi<T>(
  path: string,
  method = "GET",
  body?: unknown,
): Promise<T> {
  const key = sessionStorage.getItem("abv_admin_api_key") || "";
  const response = await fetch(`/api/portal${path}`, {
    method,
    cache: "no-store",
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
  const [models, setModels] = useState<string[]>([]);
  const [modelDraft, setModelDraft] = useState("");
  const [modelsLoaded, setModelsLoaded] = useState(false);
  const [savingModels, setSavingModels] = useState(false);
  const [modelError, setModelError] = useState("");
  const [modelNotice, setModelNotice] = useState("");
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
  const loadModels = useCallback(async () => {
    const settings = await adminApi<ModelSettings>("/settings");
    setModelError("");
    setModels(settings.allowed_models);
    setModelDraft(settings.allowed_models.join("\n"));
    setModelsLoaded(true);
  }, []);
  useEffect(() => {
    reload().catch((e) => setError(e.message));
    loadModels().catch((e) => setModelError(e.message));
  }, [reload, loadModels]);
  const draftModels = [...new Set(modelDraft.split(/[\n,，]+/).map((model) => model.trim()).filter(Boolean))];
  const modelsChanged = JSON.stringify(draftModels) !== JSON.stringify(models);
  async function saveModels() {
    setSavingModels(true);
    setModelError("");
    setModelNotice("");
    try {
      const settings = await adminApi<ModelSettings>("/settings", "PUT", {
        allowed_models: draftModels,
      });
      setModels(settings.allowed_models);
      setModelDraft(settings.allowed_models.join("\n"));
      setModelNotice("已保存，新的模型白名单已立即生效。");
    } catch (e) {
      setModelError(e instanceof Error ? e.message : "保存失败，请稍后重试");
    } finally {
      setSavingModels(false);
    }
  }
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
    <div className="flex-1 min-w-0 overflow-auto p-6 space-y-6">
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-2xl font-semibold">注册用户</h1>
          <p className="text-sm opacity-60 mt-1">
            模型开放范围、用户状态、余额与异常请求对账
          </p>
        </div>
        <button
          className="btn btn-sm"
          onClick={() => act(async () => { await loadModels(); setModelNotice(""); })}
          disabled={busy || savingModels}
        >
          刷新
        </button>
      </div>
      {error && (
        <p role="alert" className="text-error">
          {error}
        </p>
      )}
      <section className="rounded-xl border border-base-300 bg-base-100 p-5 space-y-4" aria-labelledby="portal-models-title">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 id="portal-models-title" className="text-lg font-medium">用户模型白名单</h2>
          {modelsLoaded && <span className="rounded-full border border-base-300 px-3 py-1 text-xs">当前开放 {models.length} 个模型</span>}
        </div>
        <p className="text-sm opacity-70">
          适用于所有用户中心 API Key。填写完整模型 ID，每行一个，也可用逗号分隔。保存后立即生效。
        </p>
        {modelsLoaded && models.length > 0 && (
          <div className="flex flex-wrap gap-2" aria-label="当前开放模型">
            {models.map((model) => <span key={model} className="rounded-md bg-base-200 px-2 py-1 text-xs break-all">{model}</span>)}
          </div>
        )}
        <form onSubmit={(event) => { event.preventDefault(); void saveModels(); }} className="space-y-3">
          <label className="block text-sm font-medium" htmlFor="portal-allowed-models">开放的模型 ID</label>
          <textarea
            id="portal-allowed-models"
            className="w-full min-h-32 rounded-lg border border-base-300 bg-base-100 px-3 py-2 font-mono text-sm outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-60"
            rows={4}
            maxLength={40_200}
            value={modelDraft}
            disabled={!modelsLoaded || savingModels}
            placeholder={modelsLoaded ? "每行填写一个模型 ID" : "正在读取模型白名单…"}
            onChange={(event) => { setModelDraft(event.target.value); setModelNotice(""); }}
            aria-describedby="portal-models-help"
          />
          <p id="portal-models-help" className="text-sm opacity-70">
            请使用已接入且支持文本调用的模型 ID；加入白名单后，还需确保上游账号支持该模型。最多 200 个模型。
          </p>
          {modelsLoaded && draftModels.length === 0 && (
            <p className="text-amber-700 dark:text-amber-400 text-sm" role="status">名单为空时，所有用户中心 Key 将无法调用模型。</p>
          )}
          {modelError && <p role="alert" className="text-red-600 dark:text-red-400 text-sm">{modelError}</p>}
          {modelNotice && <p role="status" className="text-emerald-700 dark:text-emerald-400 text-sm">{modelNotice}</p>}
          <div className="flex flex-wrap gap-2">
            <button className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 focus-visible:ring-2 focus-visible:ring-blue-500 disabled:cursor-not-allowed disabled:opacity-50" disabled={!modelsLoaded || savingModels || !modelsChanged}>
              {savingModels ? "保存中…" : "保存白名单"}
            </button>
            {modelsLoaded ? (
              <button type="button" className="rounded-lg border border-base-300 px-4 py-2 text-sm hover:bg-base-200 disabled:cursor-not-allowed disabled:opacity-50" disabled={savingModels || !modelsChanged}
                onClick={() => { setModelDraft(models.join("\n")); setModelError(""); setModelNotice(""); }}>
                撤销修改
              </button>
            ) : modelError && (
              <button type="button" className="rounded-lg border border-base-300 px-4 py-2 text-sm hover:bg-base-200" onClick={() => {
                setModelError(""); void loadModels().catch((e) => setModelError(e.message));
              }}>重新读取</button>
            )}
          </div>
        </form>
      </section>
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
