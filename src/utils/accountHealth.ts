import type { Account } from '../types/account';
export function accountHealth(account: Account, now = Date.now() / 1000) {
  const reason = [account.validation_blocked_reason, account.disabled_reason, account.quota?.forbidden_reason, account.proxy_disabled_reason].filter(Boolean).join(' ').toLowerCase();
  if (account.validation_blocked || reason.includes('validation_required') || reason.includes('verify your account')) return { state: 'validation', title: '需要身份验证', action: '使用此 Google 账号打开最新验证链接，完成验证后刷新额度，再做一次实际生成复测。' };
  if (/invalid_grant|token refresh|refresh.*token|刷新.*令牌|令牌.*刷新/.test(reason)) return { state: 'refresh', title: '令牌刷新失败', action: '检查出站连接；授权已失效时重新登录该账号，然后复测。' };
  if (account.disabled || account.proxy_disabled) return { state: 'disabled', title: '账号已停用', action: '先核对停用原因，确认恢复条件后再启用。' };
  if (Object.values(account.live_limited_models || {}).some(m => m.until > now)) return { state: 'limited', title: '部分模型暂时限流', action: '查看受限模型与恢复时间，等待恢复或选择其他可用模型。' };
  if (account.quota?.is_forbidden) return { state: 'forbidden', title: '权限或地区限制', action: '核对上游错误、所选模型和出站地区，不能只依据节点名称判断。' };
  return { state: 'available', title: '未发现已记录的阻断', action: '额度刷新只验证查询链路，实际生成仍需单独复测。' };
}
