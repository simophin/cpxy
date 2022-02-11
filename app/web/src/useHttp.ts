import { useState } from "react"

type Method = 'get' | 'post' | 'put' | 'delete';

export default function useHttp<T = any>(
    url: string,
    options?: {
        headers?: Record<string, string>,
        defaultMethod?: Method,
        timeoutMills?: number,
    }) {
    const [data, setData] = useState<T>();
    const [error, setError] = useState<Error>();
    const [loading, setLoading] = useState(false);

    return {
        data, error, loading,
        execute: async (method?: Method, body?: any): Promise<T> => {
            setLoading(true);
            setError(undefined);
            const timeout = options?.timeoutMills ?? 2000;
            const controller = new AbortController();
            const id = setTimeout(() => controller.abort(), timeout);
            try {
                const res = await fetch(url, {
                    headers: options?.headers,
                    method: method ?? options?.defaultMethod ?? 'get',
                    body: typeof body === 'object' ? JSON.stringify(body) : body,
                    signal: controller.signal
                });

                if (res.status >= 200 && res.status < 300) {
                    const contentType = res.headers.get('content-type')?.toLowerCase() ?? '';
                    let data;
                    if (contentType.startsWith('application/json')) {
                        data = await res.json();
                    } else if (contentType.startsWith("text/")) {
                        data = await res.text() as unknown as T;
                    } else {
                        data = res as unknown as T;
                    }
                    setData(data);
                    return data;
                } else {
                    throw new Error(await res.text());
                }
            } catch (e: any) {
                setError(e?.message ?? 'Unknown error');
                throw e;
            } finally {
                setLoading(false);
                clearTimeout(id);
            }
        }
    }
}