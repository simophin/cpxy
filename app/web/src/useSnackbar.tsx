import { Snackbar } from "@mui/material";
import { ReactElement, useEffect, useMemo, useState } from "react";

export default function useSnackbar(): [ReactElement, (v: string) => unknown] {
    const [text, setText] = useState('');
    useEffect(() => {
        if (text.length > 0) {
            const handle = setTimeout(() => {
                setText('');
            }, 1000);
            return () => clearTimeout(handle);
        }
    }, [text]);

    return [
        <Snackbar
            open={text.length > 0}
            message={text}
            onClose={() => setText('')} />,
        setText
    ];
}