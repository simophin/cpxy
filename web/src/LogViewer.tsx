import { Dialog, DialogContent, DialogTitle } from "@mui/material";
import { useEffect, useState } from "react";
import { BASE_URL } from "./config";
import useHttp from "./useHttp";


type Props = {
    onClose: () => unknown,
}

type LogEntry = {
    msg: string,
}

export default function LogViewer({ onClose }: Props) {
    // const [earliest, setEarliest] = useState();
    const logsRequest = useHttp<LogEntry[]>(`${BASE_URL}/api/logs`);

    useEffect(() => {
        logsRequest.execute();
    }, []);

    return <Dialog open={true} onClose={onClose}>
        <DialogTitle>Logs</DialogTitle>
        <DialogContent>

        </DialogContent>
    </Dialog>
}