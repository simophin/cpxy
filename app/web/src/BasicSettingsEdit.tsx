import { Button, Dialog, DialogActions, DialogContent, DialogTitle, MenuItem, Select, Stack, TextField } from "@mui/material";
import { useState } from "react";
import { BASE_URL } from "./config";
import { ClientConfig } from "./models";
import { mandatory, useEditState, validAddress } from "./useEditState";
import useHttp from "./useHttp";

type Props = {
    current_config: ClientConfig,
    onSaved: () => unknown,
    onCancelled: () => unknown
};

export default function BasicSettingsEdit({ onSaved, onCancelled, current_config }: Props) {
    const address = useEditState(current_config.socks5_address ?? '', mandatory('Address', validAddress))
    const udpHost = useEditState(current_config.socks5_udp_host ?? '', mandatory('UDP host'));
    const request = useHttp(`${BASE_URL}/api/config`, { headers: { "Content-Type": "application/json" } });

    const handleSave = async () => {
        try {
            let config: ClientConfig = {
                ...current_config,
                socks5_address: address.validate(),
                socks5_udp_host: udpHost.validate(),
            };

            await request.execute('post', config);
            onSaved();
        } catch (e) {
        }
    };

    return <Dialog open={true} fullWidth disableEscapeKeyDown>
        <DialogTitle>Basic settings</DialogTitle>
        <DialogContent>
            <Stack>
                <TextField
                    value={address.value}
                    helperText={address.error}
                    error={!!address.error}
                    onChange={(e) => address.setValue(e.target.value)}
                    margin='dense'
                    label='SOCKS5 listen address'
                    fullWidth
                    variant='outlined'
                />
                <TextField
                    label='SOCKS5 UDP Host'
                    helperText={udpHost.error}
                    error={!!udpHost.error}
                    value={udpHost.value}
                    margin='dense'
                    fullWidth
                    onChange={(e) => udpHost.setValue(e.target.value)}
                    variant='outlined' />
            </Stack>
        </DialogContent>
        <DialogActions>
            <Button onClick={onCancelled} disabled={request.loading}>Cancel</Button>
            <Button onClick={handleSave} variant='contained' disabled={request.loading}>Save</Button>
        </DialogActions>
    </Dialog>;
}