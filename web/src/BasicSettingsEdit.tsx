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
    const udpHost = useEditState(current_config.socks5_udp_host ?? '', mandatory('UDP host'));
    const fwmark = useEditState<number>(current_config.fwmark?.toString() ?? '', optional(isValidFwmark), transformFwmark);
    const udpTProxyAddress = useEditState(current_config.udp_tproxy_address ?? '', optional(validAddress));
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

    const handleSave = async () => {
        try {
            let config: ClientConfig = {
                ...current_config,
                socks5_address: address.validate(),
                socks5_udp_host: udpHost.validate(),
                fwmark: fwmark.validate(),
                udp_tproxy_address: udpTProxyAddress.validate(),
                set_router_rules: routerRules,
                traffic_rules: trafficRules.validate(),
            };

            if (config.udp_tproxy_address?.length === 0) {
                delete config.udp_tproxy_address;
            }

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
                    value={udpTProxyAddress.value}
                    helperText={udpTProxyAddress.error}
                    error={!!udpTProxyAddress.error}
                    onChange={(e) => udpTProxyAddress.setValue(e.target.value)}
                    margin='dense'
                    label='UDP TProxy listen address'
                    fullWidth
                    variant='outlined'
                />
                <TextField
                    value={fwmark.value}
                    helperText={fwmark.error}
                    error={!!fwmark.error}
                    onChange={(e) => fwmark.setValue(e.target.value)}
                    margin='dense'
                    label='TCP fwmark'
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
            <Button onClick={onCancelled} disabled={request.loading}>Cancel</Button>

            <Button onClick={handleSave} variant='contained' disabled={request.loading}>Save</Button>
        </DialogActions>
        {snackbar}
    </Dialog>;
}