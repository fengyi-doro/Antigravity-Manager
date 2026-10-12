import { useState } from 'react';
import { api } from './Portal';
export default function AccountRecovery({ enabled, email, csrf, verified, signedIn, reload }: { enabled: boolean; email: string; csrf?: string; verified?: boolean; signedIn: boolean; reload: () => Promise<unknown> }) {
  const [open, setOpen] = useState(false), [token, setToken] = useState(''), [password, setPassword] = useState(''), [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false), [message, setMessage] = useState(''), [error, setError] = useState('');
  async function act(action: () => Promise<void>) { setBusy(true); setError(''); setMessage(''); try { await action(); } catch (e) { setError(e instanceof Error ? e.message : '操作失败'); } finally { setBusy(false); } }
  if (signedIn && verified) return <p className="portal-muted">邮箱已验证</p>;
  return <section className={signedIn ? 'portal-section portal-recovery' : 'portal-recovery'}>
    {signedIn ? <h2>验证邮箱</h2> : <button type="button" className="portal-text-button" onClick={() => setOpen(!open)}>忘记密码？</button>}
    {(signedIn || open) && <>
      {!enabled ? <p className="portal-muted">邮箱服务尚未配置，请联系管理员处理。</p> : <>
        <p className="portal-muted">{signedIn ? `验证 ${email} 后即可完成账户验证。` : '先在上方填写注册邮箱，再请求发送重置邮件。验证码仅可使用一次，20 分钟有效。'}</p>
        <button type="button" className="portal-secondary" disabled={busy || !email} onClick={() => void act(async () => { const result = await api<{ message: string }>(signedIn ? '/auth/verification' : '/auth/forgot', 'POST', signedIn ? {} : { email }, csrf); setMessage(result.message); })}>{busy ? '请稍候…' : signedIn ? '重新发送验证邮件' : '发送重置邮件'}</button>
        <form onSubmit={e => { e.preventDefault(); void act(async () => {
          if (!signedIn && password !== confirmation) throw new Error('两次密码不一致');
          await api(signedIn ? '/auth/verify' : '/auth/reset', 'POST', { token: token.trim(), ...(!signedIn ? { password } : {}) }, csrf);
          setToken(''); setPassword(''); setConfirmation('');
          if (signedIn) await reload(); else setMessage('密码已更新，旧登录会话已退出，请使用新密码登录。');
        }); }}>
          <label>邮件验证码<input value={token} onChange={e => setToken(e.target.value)} required maxLength={69} autoComplete="one-time-code" /></label>
          {!signedIn && <><label>新密码<input type="password" required minLength={6} maxLength={128} autoComplete="new-password" value={password} onChange={e => setPassword(e.target.value)} /></label><label>确认新密码<input type="password" required minLength={6} maxLength={128} autoComplete="new-password" value={confirmation} onChange={e => setConfirmation(e.target.value)} /></label></>}
          <button className="portal-secondary" disabled={busy}>{signedIn ? '验证邮箱' : '更新密码'}</button>
        </form>
      </>}
      {message && <p role="status" className="portal-muted">{message}</p>}{error && <p role="alert" className="portal-error">{error}</p>}
    </>}
  </section>;
}
