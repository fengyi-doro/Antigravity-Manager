import { useState } from 'react';
import { money } from './billing';
import PortalDialog from './PortalDialog';
import { input, table, tableButton } from './styles';
export type WalletHistoryData = { credits: { reference: string; amount_nano: number; refundable_nano: number; created_at: number }[]; refunds: { id: string; credit_reference: string; amount_nano: number; state: string; reason: string; payment_reference: string | null; updated_at: number }[] };
const labels: Record<string, string> = { requested: '待审核', approved: '待线下打款', paid: '已退款', cancelled: '已取消' };
const badges: Record<string, string> = { requested: 'bg-amber-50 text-amber-700 dark:bg-amber-900/20 dark:text-amber-400', approved: 'bg-blue-50 text-blue-600 dark:bg-blue-900/20 dark:text-blue-400', paid: 'bg-green-50 text-green-600 dark:bg-green-900/20 dark:text-green-400', cancelled: 'bg-gray-100 text-gray-500 dark:bg-base-200 dark:text-gray-400' };
type Dialog = { kind: 'request'; credit: WalletHistoryData['credits'][number] } | { kind: 'approve' | 'pay' | 'cancel'; refund: WalletHistoryData['refunds'][number] };
export default function WalletHistory({ history, busy, request, action, admin = false }: { history: WalletHistoryData; busy: boolean; admin?: boolean; request?: (credit: string, cents: number, reason: string) => Promise<void>; action?: (id: string, action: string, reference?: string) => Promise<void> }) {
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [amount, setAmount] = useState(''), [reason, setReason] = useState(''), [reference, setReference] = useState('');
  const [working, setWorking] = useState(false), [error, setError] = useState('');
  function open(value: Dialog) { setDialog(value); setAmount(''); setReason(''); setReference(''); setError(''); }
  async function submit() {
    if (!dialog || working || busy) return;
    setError('');
    if (dialog.kind === 'request' && (!/^\d{1,7}(\.\d{1,2})?$/.test(amount) || Number(amount) <= 0 || Number(amount) > 1000000 || Math.round(Number(amount) * 100) * 10000000 > dialog.credit.refundable_nano || !reason.trim() || reason.length > 500)) { setError('请填写有效金额和退款原因，金额不得超过该笔充值剩余可退金额。'); return; }
    if (dialog.kind === 'pay' && (!reference.trim() || reference.trim().length > 128)) { setError('请填写实际退款流水号，最多 128 字。'); return; }
    setWorking(true);
    try { if (dialog.kind === 'request') await request?.(dialog.credit.reference, Math.round(Number(amount) * 100), reason.trim()); else await action?.(dialog.refund.id, dialog.kind, dialog.kind === 'pay' ? reference.trim() : undefined); setDialog(null); }
    catch (e) { setError(e instanceof Error ? e.message : '操作失败'); } finally { setWorking(false); }
  }
  const button = admin ? tableButton : 'portal-secondary';
  const field = admin ? input : 'portal-input';
  const frame = admin ? 'overflow-x-auto rounded-xl border border-gray-100 dark:border-base-200' : 'portal-table-scroll';
  const head = admin ? 'bg-gray-50/80 dark:bg-base-200/50' : '';
  const body = admin ? 'divide-y divide-gray-100 dark:divide-base-200' : '';
  return <section className={admin ? 'space-y-5' : 'portal-section portal-history'}>
    <div><h2 className={admin ? 'text-base font-semibold text-gray-900 dark:text-base-content' : ''}>充值与退款记录</h2><p className={admin ? 'text-xs text-gray-500 dark:text-gray-400 mt-1' : 'portal-muted'}>最近 100 条记录。退款审核后扣除余额，管理员线下打款后登记完成。</p></div>
    <div className={frame}><table className={admin ? table : 'portal-history-table'}><thead className={head}><tr><th>充值流水号</th><th>时间（北京时间）</th><th>充值金额</th><th>剩余可申请退款</th>{request && <th>操作</th>}</tr></thead><tbody className={body}>
      {history.credits.map(c => <tr key={c.reference}><td className="break-all font-mono text-xs">{c.reference}</td><td className="whitespace-nowrap text-xs">{new Date(c.created_at * 1000).toLocaleString('zh-CN', { timeZone: 'Asia/Shanghai' })}</td><td className="whitespace-nowrap tabular-nums">{money(c.amount_nano)}</td><td className="whitespace-nowrap tabular-nums">{money(c.refundable_nano)}</td>{request && <td><button className={button} disabled={busy || working || c.refundable_nano < 10000000} onClick={() => open({ kind: 'request', credit: c })}>申请退款</button></td>}</tr>)}
      {!history.credits.length && <tr><td colSpan={request ? 5 : 4} className="text-center py-8 text-gray-400">暂无人工充值</td></tr>}
    </tbody></table></div>
    <div className={frame}><table className={admin ? table : 'portal-history-table'}><thead className={head}><tr><th>退款 / 原充值</th><th>金额</th><th>状态 / 原因</th><th>线下退款流水</th>{action && <th>操作</th>}</tr></thead><tbody className={body}>
      {history.refunds.map(r => <tr key={r.id}><td className="break-all text-xs"><code>{r.id}</code><div className="mt-1 opacity-60">{r.credit_reference}</div></td><td className="whitespace-nowrap tabular-nums">{money(r.amount_nano)}</td><td><span className={admin ? `inline-flex px-2 py-0.5 text-xs font-medium rounded-full ${badges[r.state] || badges.cancelled}` : 'portal-state'}>{labels[r.state] || r.state}</span><div className={admin ? 'text-xs text-gray-500 mt-2 break-words' : 'portal-muted'}>{r.reason}</div></td><td className="break-all text-xs font-mono">{r.payment_reference || '—'}</td>{action && <td><div className="flex flex-wrap gap-2">{r.state === 'requested' && <button className={button} disabled={busy || working} onClick={() => open({ kind: 'approve', refund: r })}>审核通过</button>}{r.state === 'approved' && <button className={button} disabled={busy || working} onClick={() => open({ kind: 'pay', refund: r })}>登记已打款</button>}{['requested', 'approved'].includes(r.state) && <button className={button} disabled={busy || working} onClick={() => open({ kind: 'cancel', refund: r })}>取消退款</button>}</div></td>}</tr>)}
      {!history.refunds.length && <tr><td colSpan={action ? 5 : 4} className="text-center py-8 text-gray-400">暂无退款申请</td></tr>}
    </tbody></table></div>
    {dialog && <PortalDialog admin={admin} title={{ request: '申请退款', approve: '审核退款', pay: '登记线下退款', cancel: '取消退款' }[dialog.kind]} busy={busy || working} confirmText={{ request: '提交申请', approve: '通过并扣除余额', pay: '确认已完成打款', cancel: '确认取消退款' }[dialog.kind]} cancel={() => setDialog(null)} confirm={() => void submit()}>
      <div className="space-y-4 text-sm">
        {dialog.kind === 'request' ? <><p className="portal-muted">该笔充值可申请 {money(dialog.credit.refundable_nano)}，最终退款还需核对账户可用余额。</p><label className="block space-y-2"><span>退款金额（元）</span><input autoFocus className={field} inputMode="decimal" value={amount} onChange={e => setAmount(e.target.value)} /></label><label className="block space-y-2"><span>退款原因</span><textarea className={field} rows={3} maxLength={500} placeholder="请勿填写银行卡、密码等敏感信息" value={reason} onChange={e => setReason(e.target.value)} /></label></> : <><p>退款金额 <strong>{money(dialog.refund.amount_nano)}</strong></p>{dialog.kind === 'pay' ? <><p className="text-gray-500">请在线下打款完成后登记，本操作不转账。</p><label className="block space-y-2"><span>实际退款流水号</span><input autoFocus className={field} maxLength={128} value={reference} onChange={e => setReference(e.target.value)} /></label></> : <p className="text-gray-500">{dialog.kind === 'approve' ? '通过后将从账户余额中扣除该金额，随后安排线下退款。' : '取消已审核但尚未打款的退款，将恢复此前扣除的余额。'}</p>}</>}
        {error && <p role="alert" className={admin ? 'text-red-500 text-sm' : 'portal-error'}>{error}</p>}
      </div>
    </PortalDialog>}
  </section>;
}
