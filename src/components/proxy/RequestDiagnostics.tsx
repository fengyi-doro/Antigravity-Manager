export type Diagnostics = {
  error_class: string; total_attempts: number; truncated: boolean;
  attempts: { sequence: number; model: string; account_id: string | null; endpoint: string; status: number | null; elapsed_ms: number; error_class: string }[];
};
const labels: Record<string, string> = {
  success: "成功", account_validation: "账号需要身份验证", token_refresh: "令牌刷新失败",
  authentication: "鉴权失败", insufficient_balance: "余额不足", permission_or_region: "权限或地区限制",
  model_or_endpoint: "模型或端点不支持", timeout: "请求超时", rate_limit: "上游限流",
  client_cancelled: "客户端取消", upstream_or_gateway: "上游或网关故障", network: "网络连接失败",
};
export default function RequestDiagnostics({ requestId, diagnostics }: { requestId: string; diagnostics?: Diagnostics | null }) {
  return <details className="rounded-xl border border-gray-200 dark:border-base-300 p-3 text-xs shrink-0">
    <summary className="cursor-pointer">请求追踪 · <code className="break-all">{requestId}</code>{diagnostics && ` · ${labels[diagnostics.error_class] || diagnostics.error_class}`}</summary>
    {!diagnostics ? <p className="mt-2 opacity-60">此记录没有上游尝试明细。旧记录或尚未进入上游的请求可能没有诊断数据。</p> : <div className="overflow-auto max-h-48 mt-2">
      <table className="table table-xs"><thead><tr><th>尝试</th><th>实际模型 / 上游账号 ID</th><th>端点</th><th>结果</th><th>耗时</th></tr></thead>
        <tbody>{diagnostics.attempts.map(a => <tr key={a.sequence}><td>{a.sequence}</td><td>{a.model || "—"}<div className="opacity-60">{a.account_id || "—"}</div></td><td>{a.endpoint}</td><td>{a.status ?? "无 HTTP 响应"} · {labels[a.error_class] || a.error_class}</td><td>{a.elapsed_ms} ms</td></tr>)}</tbody></table>
      {!diagnostics.attempts.length && <p className="opacity-60">此请求未记录到上游尝试，先检查鉴权、余额、模型与账号选择阶段。</p>}
      {diagnostics.truncated && <p>共 {diagnostics.total_attempts} 次尝试，仅保留前 32 次。</p>}
    </div>}
  </details>;
}
