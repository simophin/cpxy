import { Button, Dialog, DialogActions, DialogContent, DialogTitle, Stack, TextField, FormControl, Switch, FormControlLabel } from "@mui/material";
import { useEffect, useState } from "react";
import { BASE_URL } from "./config";
import { ClientConfig } from "./models";
import { mandatory, optional, useEditState, validAddress } from "./useEditState";
import useHttp from "./useHttp";
import useSnackbar from "./useSnackbar";

type Props = {
    current_config: ClientConfig,
    onSaved: () => unknown,
    onCancelled: () => unknown
};

type RuleResult = {
    num_rules?: number,
    last_updated: string,
}

function formatDate(str: string | undefined) {
    return str ? new Date(str).toLocaleString() : "None"
}

function isValidFwmark(value: string): string | undefined {
    parseInt(value);
    return undefined;
}

function transformFwmark(text: string): number {
    return parseInt(text);
}

export default function BasicSettingsEdit({ onSaved, onCancelled, current_config }: Props) {
    const address = useEditState(current_config.socks5_address ?? '', mandatory('Address', validAddress))
    const fwmark = useEditState<number>(current_config.fwmark?.toString() ?? '', optional(isValidFwmark), transformFwmark);
    const [routerRules, setRouterRules] = useState<boolean>(current_config.set_router_rules === true);
    const request = useHttp(`${BASE_URL}/api/config`, { headers: { "Content-Type": "application/json" } });
    const [snackbar, showSnackbar] = useSnackbar();
    const trafficRules = useEditState(current_config.traffic_rules ?? '');
    const gfwListRequest = useHttp<RuleResult>(`${BASE_URL}/api/gfwlist`, { timeoutMills: 40000 });
    const adBlockListRequest = useHttp<RuleResult>(`${BASE_URL}/api/adblocklist`, { timeoutMills: 40000 });

    useEffect(() => {
        gfwListRequest.execute('get');
        adBlockListRequest.execute('get');
    }, []);

    const handleUpdateGfw = async () => {
        try {
            const { num_rules } = await gfwListRequest.execute('post');
            showSnackbar(`Updated ${num_rules} items`)
        } catch (e: any) {
            showSnackbar(`Error updating GFW List: ${e.message}`)
        }
    };

    const handleUpdateAbp = async () => {
        try {
            const { num_rules } = await adBlockListRequest.execute('post');
            showSnackbar(`Updated ${num_rules} items`)
        } catch (e: any) {
            showSnackbar(`Error updating GFW List: ${e.message}`)
        }
    };

    const onExportClicked = () => {
        window.location.href = `${BASE_URL}/export`;
    }

    const handleSave = async () => {
        try {
            let config: ClientConfig = {
                ...current_config,
                socks5_address: address.validate(),
                fwmark: fwmark.validate(),
                set_router_rules: routerRules,
                traffic_rules: trafficRules.validate(),
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
                    label='Traffic rules'
                    helperText={trafficRules.error}
                    error={!!trafficRules.error}
                    value={trafficRules.value}
                    multiline
                    margin='dense'
                    fullWidth
                    minRows={5}
                    style={{ fontFamily: "monospace", }}
                    onChange={(e) => trafficRules.setValue(e.target.value)}
                    variant='outlined' />
                <div>
                    <FormControl>
                        <FormControlLabel
                            control={<Switch
                                checked={routerRules}
                                onChange={v => setRouterRules(v.currentTarget.checked)}
                            />}
                            labelPlacement='start'
                            label="Set Router Rules (Linux only)" />
                    </FormControl>
                </div>

                <div style={{ marginTop: 8 }}>
                    <b>GFW List: </b>{gfwListRequest.data ? formatDate(gfwListRequest.data.last_updated)
                        : (gfwListRequest.error ? 'Error' : 'Loading')} &nbsp;
                    <a href="#" onClick={handleUpdateGfw}>Update</a>
                </div>

                <div style={{ marginTop: 8 }}>
                    <b>Adblock List: </b>{adBlockListRequest.data ? formatDate(adBlockListRequest.data.last_updated)
                        : (adBlockListRequest.error ? 'Error' : 'Loading')} &nbsp;
                    <a onClick={handleUpdateAbp} href="#">Update</a>
                </div>

                {request.error && `Error: ${request.error}`}
            </Stack>
        </DialogContent>
        <DialogActions>
            <Button onClick={onExportClicked} disabled={request.loading}>Export</Button>
            <Button onClick={onCancelled} disabled={request.loading}>Cancel</Button>
            <Button onClick={handleSave} variant='contained' disabled={request.loading}>Save</Button>
        </DialogActions>
        {snackbar}
    </Dialog>;
}