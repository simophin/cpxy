import { Button, Dialog, DialogActions, DialogContent, DialogTitle, Stack, TextField } from "@mui/material";
import { BASE_URL } from "./config";
import useHttp from "./useHttp";
import { useEffect, useState } from "react";
import { useEditState } from "./useEditState";
import useSnackbar from "./useSnackbar";

type Props = {
    onClose: () => unknown,
}

export default function RawSettingsEdit({ onClose }: Props) {
    const fetchConfig = useHttp<string>(
        `${BASE_URL}/api/config`,
        {
            headers: { "Accept": "text/yaml" },
        }
    );

    const updateConfig = useHttp<string>(
        `${BASE_URL}/api/config?replace=all`,
        {
            headers: { "Content-Type": "text/yaml" },
        }
    );

    const rawSettings = useEditState('');
    const [snackbar, showSnackbar] = useSnackbar();

    useEffect(() => {
        (async () => {
            rawSettings.setValue(await fetchConfig.execute('get'));
        })();
    }, []);

    const handleSave = async () => {
        try {
            await updateConfig.execute('post', rawSettings.value);
            onClose();
            window.location.reload();
        } catch (e) {
            showSnackbar(`Error saving raw config: ${e}`)
        }
    };

    return <Dialog open={true} onClose={onClose} fullWidth>
        <DialogTitle>Raw configuration</DialogTitle>
        <DialogContent>
            {fetchConfig.loading && <div>Loading...</div>}
            {fetchConfig.data && <Stack>
                <TextField
                    multiline
                    margin='dense'
                    fullWidth
                    value={rawSettings.value}
                    onChange={(e) => rawSettings.setValue(e.target.value)}
                    minRows={5}
                    style={{ fontFamily: "monospace", }}
                />
            </Stack>}

            {fetchConfig.data && <DialogActions>
                <Button onClick={onClose}>Cancel</Button>
                <Button onClick={handleSave} variant='contained' disabled={fetchConfig.loading}>Save</Button>
            </DialogActions>}
        </DialogContent>
        {snackbar}
    </Dialog>
}