import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CheckCircle2, Copy, Eye, EyeOff, FlaskConical, Loader2, Play, Square, XCircle } from 'lucide-react';
import { useConfigStore } from '../stores/useConfigStore';
import { isTauri } from '../utils/env';
import { copyToClipboard } from '../utils/clipboard';
import { apiTestRequest, gatewayRoot, modelsFromResult, testEndpoint, testPayload, type TestAuth, type TestProtocol, type TestResult } from '../utils/apiTest';

export default function ApiTest() {
    const { t } = useTranslation();
    const { config } = useConfigStore();
    const defaultBase = isTauri() ? `http://127.0.0.1:${config?.proxy.port ?? 8045}/v1` : `${window.location.origin}/v1`;
    const [base, setBase] = useState<string | null>(null);
    const [key, setKey] = useState('');
    const [showKey, setShowKey] = useState(false);
    const [auth, setAuth] = useState<TestAuth>('bearer');
    const [protocol, setProtocol] = useState<TestProtocol>('chat');
    const [model, setModel] = useState('');
    const [models, setModels] = useState<string[]>([]);
    const [prompt, setPrompt] = useState(t('api_test.default_prompt'));
    const [maxTokens, setMaxTokens] = useState(1024);
    const [stream, setStream] = useState(false);
    const [custom, setCustom] = useState(false);
    const [json, setJson] = useState('');
    const [busy, setBusy] = useState<'models' | 'request' | null>(null);
    const [error, setError] = useState('');
    const [result, setResult] = useState<TestResult | null>(null);
    const [history, setHistory] = useState<Omit<TestResult, 'body' | 'output' | 'error'>[]>([]);
    const [copied, setCopied] = useState(false);
    const controller = useRef<AbortController | null>(null);
    const cancelReason = useRef<'cancelled' | 'timeout'>('cancelled');
    const effectiveBase = base ?? defaultBase;

    useEffect(() => { setModels([]); }, [key, effectiveBase, auth]);
    useEffect(() => () => controller.current?.abort(), []);

    async function run(mode: 'models' | 'request') {
        if (!key.trim()) { setError(t('api_test.key_required')); return; }
        let root: string;
        let payload: Record<string, unknown> | undefined;
        let requestedModel = model.trim();
        try {
            root = gatewayRoot(effectiveBase);
            if (mode === 'request') {
                payload = custom ? JSON.parse(json) : testPayload(protocol, requestedModel, prompt, maxTokens, stream);
                if (!payload || typeof payload !== 'object' || Array.isArray(payload)) throw new Error('invalid_json');
                if (protocol !== 'gemini') requestedModel = typeof payload.model === 'string' ? payload.model.trim() : '';
                if (!requestedModel) { setError(t('api_test.model_required')); return; }
            }
        } catch (failure) {
            setError(t(failure instanceof SyntaxError || (failure instanceof Error && failure.message === 'invalid_json') ? 'api_test.invalid_json' : 'api_test.invalid_url'));
            return;
        }
        const abort = new AbortController();
        controller.current = abort;
        cancelReason.current = 'cancelled';
        const timeout = window.setTimeout(() => { cancelReason.current = 'timeout'; abort.abort(); }, 120000);
        setBusy(mode); setError(''); setCopied(false); setResult(null);
        try {
            const next = await apiTestRequest({ url: mode === 'models' ? `${root}/v1/models` : testEndpoint(root, protocol, requestedModel, stream), key: key.trim(), auth, signal: abort.signal, model: mode === 'request' ? requestedModel : '', payload, rawBody: mode === 'request' && custom ? json : undefined });
            if (abort.signal.aborted) return;
            setResult(next);
            if (mode === 'models') {
                const ids = modelsFromResult(next);
                setModels(ids);
                if (ids.length && !model.trim()) setModel(ids[0]);
                if (next.status >= 200 && next.status < 300 && !ids.length) setError(t('api_test.no_models'));
            } else {
                const { body: _body, output: _output, error: _error, ...summary } = next;
                setHistory(previous => [summary, ...previous].slice(0, 5));
            }
        } catch {
            setError(t(abort.signal.aborted ? `api_test.${cancelReason.current}` : 'api_test.network_error'));
        } finally {
            window.clearTimeout(timeout);
            if (controller.current === abort) { controller.current = null; setBusy(null); }
        }
    }

    const success = result && result.status >= 200 && result.status < 300 && !result.error && !result.truncated;
    const input = 'w-full min-w-0 rounded-lg border border-gray-200 dark:border-base-300 bg-white dark:bg-base-100 px-3 py-2.5 text-sm text-gray-900 dark:text-base-content focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-60';
    const label = 'block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2';
    const card = 'min-w-0 rounded-2xl border border-gray-200 dark:border-base-200 bg-white dark:bg-base-200 p-5 sm:p-6';

    return <div className="h-full overflow-y-auto">
        <div className="max-w-7xl mx-auto p-4 sm:p-8 space-y-6">
            <header>
                <h1 className="text-2xl font-bold text-gray-900 dark:text-base-content flex items-center gap-3"><FlaskConical className="w-6 h-6 text-blue-500" />{t('api_test.title')}</h1>
                <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">{t('api_test.description')}</p>
            </header>
            <div className="grid lg:grid-cols-2 gap-6 items-start">
                <form className={card} onSubmit={event => { event.preventDefault(); void run('request'); }}>
                    <fieldset disabled={!!busy} className="space-y-5 min-w-0">
                        <div><label className={label} htmlFor="test-base">{t('api_test.base_url')}</label><input id="test-base" className={input} type="url" value={effectiveBase} onChange={event => setBase(event.target.value)} required autoComplete="off" /></div>
                        <div>
                            <label className={label} htmlFor="test-key">API Key</label>
                            <div className="flex gap-2"><input id="test-key" type={showKey ? 'text' : 'password'} className={`${input} font-mono`} value={key} onChange={event => setKey(event.target.value)} placeholder="sk-…" autoComplete="off" spellCheck={false} required />
                                <button type="button" className="p-2.5 rounded-lg border border-gray-200 dark:border-base-300" aria-label={t(showKey ? 'api_test.hide_key' : 'api_test.show_key')} onClick={() => setShowKey(!showKey)}>{showKey ? <EyeOff size={18} /> : <Eye size={18} />}</button>
                                <button type="button" className="text-sm text-gray-500 whitespace-nowrap px-2" onClick={() => { setKey(''); setShowKey(false); }}>{t('api_test.clear_key')}</button>
                            </div>
                            <p className="mt-2 text-xs text-gray-500">{t('api_test.key_hint')}</p>
                        </div>
                        <div className="grid sm:grid-cols-2 gap-4">
                            <div><label className={label} htmlFor="test-protocol">{t('api_test.protocol')}</label><select id="test-protocol" className={input} value={protocol} onChange={event => { setProtocol(event.target.value as TestProtocol); setCustom(false); }}>
                                <option value="chat">OpenAI Chat</option><option value="responses">OpenAI Responses</option><option value="claude">Anthropic Claude</option><option value="gemini">Google Gemini</option>
                            </select></div>
                            <div><label className={label} htmlFor="test-auth">{t('api_test.auth_header')}</label><select id="test-auth" className={input} value={auth} onChange={event => setAuth(event.target.value as TestAuth)}><option value="bearer">Authorization: Bearer</option><option value="x-api-key">x-api-key</option><option value="x-goog-api-key">x-goog-api-key</option></select></div>
                        </div>
                        <div>
                            <label className={label} htmlFor="test-model">{t('api_test.model')}</label>
                            <div className="flex gap-2"><input id="test-model" className={`${input} font-mono`} list="test-models" value={model} onChange={event => setModel(event.target.value)} placeholder={t('api_test.model_placeholder')} />
                                <button type="button" className="shrink-0 px-3 rounded-lg border border-gray-200 dark:border-base-300 text-sm hover:bg-gray-50 dark:hover:bg-base-300 disabled:opacity-60" onClick={() => void run('models')}>{busy === 'models' ? <Loader2 className="animate-spin" size={18} /> : t('api_test.load_models')}</button>
                            </div>
                            <datalist id="test-models">{models.map(id => <option key={id} value={id} />)}</datalist>
                            {!!models.length && <p className="mt-2 text-xs text-gray-500">{t('api_test.model_count', { count: models.length })}</p>}
                        </div>
                        <label className="flex gap-2 items-center text-sm cursor-pointer"><input type="checkbox" checked={custom} onChange={event => { setCustom(event.target.checked); if (event.target.checked) setJson(JSON.stringify(testPayload(protocol, model, prompt, maxTokens, stream), null, 2)); }} />{t('api_test.custom_json')}</label>
                        {custom ? <div><label className={label} htmlFor="test-json">{t('api_test.request_body')}</label><textarea id="test-json" className={`${input} font-mono resize-y`} rows={12} value={json} onChange={event => setJson(event.target.value)} spellCheck={false} /><p className="mt-2 text-xs text-gray-500">{t('api_test.json_hint')}</p></div> : <>
                            <div><label className={label} htmlFor="test-prompt">{t('api_test.prompt')}</label><textarea id="test-prompt" className={`${input} resize-y`} rows={4} value={prompt} onChange={event => setPrompt(event.target.value)} required /></div>
                            <div><label className={label} htmlFor="test-max-tokens">{t('api_test.max_tokens')}</label><input id="test-max-tokens" className={input} type="number" min={1} max={131072} value={maxTokens} onChange={event => setMaxTokens(Number(event.target.value))} required /></div>
                        </>}
                        {(!custom || protocol === 'gemini') && <label className="flex gap-2 items-center text-sm cursor-pointer"><input type="checkbox" checked={stream} onChange={event => setStream(event.target.checked)} />{t('api_test.stream')}</label>}
                        <button type="submit" className="w-full flex justify-center items-center gap-2 rounded-lg bg-blue-600 hover:bg-blue-700 text-white px-4 py-3 font-medium disabled:opacity-60">{busy === 'request' ? <Loader2 size={18} className="animate-spin" /> : <Play size={18} />}{t(busy === 'request' ? 'api_test.running' : 'api_test.send')}</button>
                    </fieldset>
                    {!!busy && <button type="button" className="mt-3 w-full flex justify-center items-center gap-2 text-sm text-gray-500 py-2" onClick={() => { cancelReason.current = 'cancelled'; controller.current?.abort(); }}><Square size={14} />{t('api_test.cancel')}</button>}
                    {!!error && <p role="alert" className="mt-4 rounded-lg bg-red-50 dark:bg-red-950/30 text-red-600 dark:text-red-400 p-3 text-sm break-words">{error}</p>}
                </form>
                <section className={`${card} space-y-5`} aria-live="polite" aria-busy={!!busy}>
                    <div className="flex items-center justify-between gap-3"><h2 className="font-semibold text-gray-900 dark:text-base-content">{t('api_test.result')}</h2>{result && <button type="button" className="text-sm text-gray-500 flex items-center gap-1.5" onClick={async () => { setCopied(await copyToClipboard(JSON.stringify(result, null, 2))); }}><Copy size={15} />{t(copied ? 'api_test.copied' : 'api_test.copy_result')}</button>}</div>
                    {!result ? <div className="py-16 text-center text-gray-400">{busy ? <Loader2 className="w-8 h-8 animate-spin mx-auto mb-3 text-blue-500" /> : <FlaskConical className="w-10 h-10 mx-auto mb-3 opacity-40" />}<p className="text-sm">{t(busy ? 'api_test.waiting' : 'api_test.empty')}</p></div> : <>
                        <div className={`flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm ${success ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/30 dark:text-emerald-400' : 'bg-red-50 text-red-600 dark:bg-red-950/30 dark:text-red-400'}`}>{success ? <CheckCircle2 size={18} /> : <XCircle size={18} />}<strong>HTTP {result.status}</strong><span className="ml-auto">{result.elapsedMs} ms</span></div>
                        <dl className="grid grid-cols-2 gap-4 text-sm">
                            {[[t('api_test.request_model'), result.model], [t('api_test.mapped_model'), result.mappedModel], [t('api_test.account'), result.account], ['API Key', `••••${result.keySuffix}`]].map(([title, value]) => <div key={title} className="min-w-0"><dt className="text-xs text-gray-500 mb-1">{title}</dt><dd className="font-mono break-all text-gray-800 dark:text-gray-200">{value || t('api_test.not_provided')}</dd></div>)}
                            <div className="col-span-2 min-w-0"><dt className="text-xs text-gray-500 mb-1">{t('api_test.endpoint')}</dt><dd className="font-mono text-xs break-all text-gray-500">{result.url}</dd></div>
                        </dl>
                        {!!result.error && <p role="alert" className="text-sm text-red-600 dark:text-red-400 whitespace-pre-wrap break-words">{result.error}</p>}
                        {result.truncated && <p className="text-sm text-amber-600">{t('api_test.truncated')}</p>}
                        {!!result.output && <div><h3 className="text-sm font-medium mb-2">{t('api_test.output')}</h3><pre className="text-sm whitespace-pre-wrap break-words rounded-lg bg-gray-50 dark:bg-base-300 p-3 max-h-64 overflow-auto">{result.output}</pre></div>}
                        <details open={!result.output}><summary className="text-sm cursor-pointer text-gray-600 dark:text-gray-400">{t('api_test.response_body')}</summary><pre className="mt-3 rounded-lg bg-gray-50 dark:bg-base-300 p-3 text-xs font-mono whitespace-pre-wrap break-all max-h-96 overflow-auto">{result.body || t('api_test.empty_body')}</pre></details>
                    </>}
                    {!!history.length && <div className="border-t border-gray-100 dark:border-base-300 pt-4"><h3 className="text-xs font-medium text-gray-500 mb-2">{t('api_test.recent')}</h3><ul className="space-y-2">{history.map((item, index) => <li key={`${item.time}-${index}`} className="flex flex-wrap gap-x-3 gap-y-1 text-xs font-mono text-gray-500"><span>{item.time}</span><span>••••{item.keySuffix}</span><span>HTTP {item.status}</span><span className="break-all">{item.mappedModel || item.model}</span><span>{item.elapsedMs} ms</span></li>)}</ul></div>}
                </section>
            </div>
        </div>
    </div>;
}
