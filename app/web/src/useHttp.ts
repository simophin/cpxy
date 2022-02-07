import { useState } from "react"

type Method = 'get' | 'post' | 'put' | 'delete';

export default function useHttp<T = any>(
    url: string,
    options?: {
        headers?: Record<string, string>,
        defaultMethod?: Method,
    }) {
    const [data, setData] = useState<T>();
    const [error, setError] = useState<Error>();
    const [loading, setLoading] = useState(false);

    return {
        data, error, loading,
        execute: async (method?: Method, body?: any) => {
            setLoading(false);
            setError(undefined);
            try {
                const res = await fetch(url, {
                    headers: options?.headers,
                    method: method ?? options?.defaultMethod ?? 'get',
                    body: typeof body === 'object' ? JSON.stringify(body) : body,
                });

                if (res.status >= 200 && res.status < 300) {
                    const contentType = res.headers.get('content-type')?.toLowerCase() ?? '';
                    if (contentType.startsWith('application/json')) {
                        setData(await res.json());
                    } else if (contentType.startsWith("text/")) {
                        setData(await res.text() as unknown as T);
                    } else {
                        setData(res as unknown as T);
                    }
                } else {
                    throw new Error(await res.text());
                }
            } catch (e: any) {
                setError(e?.message ?? 'Unknown error');
                throw e;
            } finally {
                setLoading(false);
            }
        }
    }
}