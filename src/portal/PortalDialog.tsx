import { useEffect, useId, useRef, type ReactNode } from 'react';
import ModalDialog from '../components/common/ModalDialog';
export default function PortalDialog({ title, children, busy, confirmText, confirm, cancel, admin = false }: {
  title: string; children: ReactNode; busy: boolean; confirmText: string;
  confirm: () => void; cancel: () => void; admin?: boolean;
}) {
  const id = useId();
  const previousFocus = useRef<HTMLElement | null>(null);
  useEffect(() => { previousFocus.current = document.activeElement as HTMLElement; return () => previousFocus.current?.focus(); }, []);
  if (admin) return <ModalDialog isOpen title={title} isLoading={busy} confirmText={confirmText} cancelText="取消" onConfirm={confirm} onCancel={busy ? undefined : cancel}>{children}</ModalDialog>;
  return <div className="portal-overlay" role="presentation" onClick={e => { if (e.target === e.currentTarget && !busy) cancel(); }}>
    <div className="portal-dialog" role="dialog" aria-modal="true" aria-labelledby={id} onKeyDown={e => { if (e.key === 'Escape' && !busy) cancel(); }}>
      <h2 id={id}>{title}</h2>{children}
      <div className="portal-dialog-actions"><button type="button" className="portal-secondary" disabled={busy} onClick={cancel}>取消</button><button type="button" className="portal-primary" disabled={busy} onClick={confirm}>{busy ? '请稍候…' : confirmText}</button></div>
    </div>
  </div>;
}
