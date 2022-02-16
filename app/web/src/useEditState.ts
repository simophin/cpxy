import { useEffect, useState } from "react";

export type FindError = (value: string) => string | undefined;

export function mandatory(fieldName: string, next?: FindError): FindError {
    return (value: string) => {
        const new_value = value.trim();
        if (new_value.length === 0) {
            return `${fieldName} can not be empty`;
        }

        if (next) {
            return next(new_value);
        }
    }
}

export function optional(next?: FindError): FindError {
    return (value: string) => {
        const new_value = value.trim();
        if (new_value.length === 0) {
            return undefined;
        }

        if (next) {
            return next(new_value);
        }
    }
}

export const validAddress: FindError = (value) => {
    const [host, port] = value.trim().split(':');
    const portNum = parseInt(port);
    if (portNum === NaN || portNum <= 0 || portNum >= 65536) {
        return `Port ${port} is invalid`;
    }

    if (host.length === 0) {
        return 'Host is empty';
    }
}

export function useEditState<T = string>(initial: string, findError?: FindError, transform?: (value: string) => T) {
    const [text, setText] = useState(initial);
    const [error, setError] = useState<string>();
    useEffect(() => {
        if (text.length > 0) setError(undefined)
    }, [text]);

    return {
        value: text,
        setValue: setText,
        error,
        validate: (): T => {
            const newError = findError ? findError(text) : undefined;
            setError(newError);
            if (newError) {
                throw newError;
            }
            try {
                return transform ? transform(text.trim()) : (text.trim() as unknown as T);
            } catch (e: any) {
                setError(e?.message ?? 'Invalid value');
                throw e;
            }
        }
    }
}