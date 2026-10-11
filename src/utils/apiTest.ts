export type TestProtocol = 'chat' | 'responses' | 'claude' | 'gemini';
export type TestAuth = 'bearer' | 'x-api-key' | 'x-goog-api-key';

export interface TestResult {
    status: number;
    elapsedMs: number;
    url: string;
    model: string;
    mappedModel: string;
    account: string;
    keySuffix: string;
    body: string;
    output: string;
    error: string;
    truncated: boolean;
    time: string;
}

type Json = Record<string, unknown>;
const object = (value: unknown): Json => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Json : {};
const array = (value: unknown): unknown[] => Array.isArray(value) ? value : [];

export function gatewayRoot(base: string): string {
    const url = new URL(base.trim());
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
        throw new Error('invalid_url');
    }
    return (url.origin + url.pathname).replace(/\/+$/, '').replace(/\/v1(?:beta)?$/, '');
}

export function testPayload(protocol: TestProtocol, model: string, prompt: string, maxTokens: number, stream: boolean): Json {
    if (protocol === 'responses') return { model, input: prompt, max_output_tokens: maxTokens, stream };
    if (protocol === 'gemini') return { contents: [{ role: 'user', parts: [{ text: prompt }] }], generationConfig: { maxOutputTokens: maxTokens } };
    return { model, messages: [{ role: 'user', content: prompt }], max_tokens: maxTokens, stream };
}

export function testEndpoint(root: string, protocol: TestProtocol, model: string, stream: boolean): string {
    if (protocol === 'gemini') return `${root}/v1beta/models/${encodeURIComponent(model)}:${stream ? 'streamGenerateContent?alt=sse' : 'generateContent'}`;
    return `${root}/v1/${{ chat: 'chat/completions', responses: 'responses', claude: 'messages' }[protocol]}`;
}

function outputText(value: unknown): string {
    const data = object(value);
    if (data.type === 'response.output_text.delta') return typeof data.delta === 'string' ? data.delta : '';
    if (data.type === 'content_block_delta') return String(object(data.delta).text ?? '');
    const chat = array(data.choices).map(choice => {
        const item = object(choice);
        const message = object(item.delta ?? item.message);
        return typeof message.content === 'string' ? message.content : '';
    }).join('');
    if (chat) return chat;
    const parts = [...array(data.content), ...array(data.output).flatMap(item => array(object(item).content))];
    const text = parts.map(part => object(part).text).filter(value => typeof value === 'string').join('');
    if (text) return text;
    return array(data.candidates).flatMap(candidate => array(object(object(candidate).content).parts))
        .map(part => object(part).text).filter(value => typeof value === 'string').join('');
}

export function inspectTestResponse(raw: string): { output: string; error: string; body: string } {
    const values: unknown[] = [];
    const sse = /^(?:data|event):/m.test(raw);
    if (sse) {
        for (const event of raw.split(/\r?\n\r?\n/)) {
            const data = event.split(/\r?\n/).filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n');
            if (!data || data === '[DONE]') continue;
            try { values.push(JSON.parse(data)); } catch { /* Retain non-JSON events in the raw response. */ }
        }
    } else {
        try { values.push(JSON.parse(raw)); } catch { /* HTML/text errors remain visible. */ }
    }
    const errors = values.flatMap(value => {
        const data = object(value);
        const failure = data.error ?? object(data.response).error ?? (data.type === 'error' ? data : null);
        if (!failure) return [];
        return [typeof failure === 'string' ? failure : String(object(failure).message ?? JSON.stringify(failure))];
    });
    // Responses sends the complete response again at the end of an SSE stream.
    const hasDeltas = values.some(value => object(value).type === 'response.output_text.delta');
    const output = values.filter(value => !hasDeltas || object(value).type === 'response.output_text.delta').map(outputText).join('');
    return { output, error: errors.join('\n'), body: !sse && values.length === 1 ? JSON.stringify(values[0], null, 2) : raw };
}

export async function apiTestRequest(options: {
    url: string; key: string; auth: TestAuth; signal: AbortSignal; model?: string; payload?: Json; rawBody?: string;
}): Promise<TestResult> {
    const { url, key, auth, signal, payload, model = '' } = options;
    const headers: Record<string, string> = auth === 'bearer' ? { Authorization: `Bearer ${key}` } : { [auth]: key };
    if (payload) headers['Content-Type'] = 'application/json';
    if (url.endsWith('/v1/messages')) headers['anthropic-version'] = '2023-06-01';
    const started = performance.now();
    // Use only the entered API Key. Management credentials/cookies are unrelated to this request.
    const response = await fetch(url, { method: payload ? 'POST' : 'GET', headers, body: payload ? options.rawBody ?? JSON.stringify(payload) : undefined, signal, credentials: 'omit', cache: 'no-store', redirect: 'error' });
    const reader = response.body?.getReader();
    const decoder = new TextDecoder();
    let body = '', bytes = 0, truncated = false;
    const limit = 1024 * 1024;
    if (reader) {
        while (true) {
            const chunk = await reader.read();
            if (chunk.done) break;
            const remaining = limit - bytes;
            body += decoder.decode(chunk.value.subarray(0, remaining), { stream: true });
            bytes += chunk.value.length;
            if (bytes > limit) { truncated = true; await reader.cancel(); break; }
        }
        body += decoder.decode();
    }
    const inspected = inspectTestResponse(body.split(key).join('[redacted]'));
    return {
        ...inspected, status: response.status, elapsedMs: Math.round(performance.now() - started), url, model,
        mappedModel: response.headers.get('x-mapped-model') ?? '', account: response.headers.get('x-account-email') ?? '',
        keySuffix: key.slice(-4), truncated, time: new Date().toLocaleTimeString(),
    };
}

export function modelsFromResult(result: TestResult): string[] {
    if (result.status < 200 || result.status >= 300 || result.error || result.truncated) return [];
    try {
        const data = object(JSON.parse(result.body));
        return [...new Set(array(data.data).map(item => object(item).id).filter((id): id is string => typeof id === 'string'))].sort();
    } catch { return []; }
}
