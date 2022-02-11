import { Button, Dialog, DialogActions, DialogContent, DialogTitle, MenuItem, Select, Stack, TextField } from "@mui/material";
import { BASE_URL } from "./config";
import { ClientConfig } from "./models";
import { transformRule } from "./trafficRules";
import { mandatory, useEditState, validAddress } from "./useEditState";
import useHttp from "./useHttp";
import useSnackbar from "./useSnackbar";

type Props = {
    current_config: ClientConfig,
    onSaved: () => unknown,
    onCancelled: () => unknown
};

type RuleUpdateResult = {
    num_updated: number,
}

export default function BasicSettingsEdit({ onSaved, onCancelled, current_config }: Props) {
    const address = useEditState(current_config.socks5_address ?? '', mandatory('Address', validAddress))
    const udpHost = useEditState(current_config.socks5_udp_host ?? '', mandatory('UDP host'));
    const accept = useEditState(current_config.direct_accept?.join('\n') ?? '', undefined, transformRule);
    const reject = useEditState(current_config.direct_reject?.join('\n') ?? '', undefined, transformRule);
    const request = useHttp(`${BASE_URL}/api/config`, { headers: { "Content-Type": "application/json" } });
    const [snackbar, showSnackbar] = useSnackbar();
    const updateGfw = useHttp<RuleUpdateResult>(`${BASE_URL}/api/gfwlist`, { timeoutMills: 40000 });
    const updateAbp = useHttp<RuleUpdateResult>(`${BASE_URL}/api/abplist`, { timeoutMills: 40000 });

    const handleUpdateGfw = async () => {
        try {
            const { num_updated } = await updateGfw.execute('post');
            showSnackbar(`Updated ${num_updated} items`)
        } catch (e: any) {
            showSnackbar(`Error updating GFW List: ${e.message}`)
        }
    };

    const handleUpdateAbp = async () => {
        try {
            const { num_updated } = await updateAbp.execute('post');
            showSnackbar(`Updated ${num_updated} items`)
        } catch (e: any) {
            showSnackbar(`Error updating GFW List: ${e.message}`)
        }
    };

    const handleSave = async () => {
        try {
            let config: ClientConfig = {
                ...current_config,
                direct_accept: accept.validate(),
                direct_reject: reject.validate(),
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
                    value={accept.value}
                    label='Direct accept rules'
                    margin='dense'
                    multiline
                    error={!!accept.error}
                    helperText={accept.error}
                    onChange={v => accept.setValue(v.currentTarget.value)} />

                <TextField
                    value={reject.value}
                    label='Direct reject rules'
                    margin='dense'
                    multiline
                    error={!!reject.error}
                    helperText={reject.error}
                    onChange={v => reject.setValue(v.currentTarget.value)} />
                <TextField
                    label='SOCKS5 UDP Host'
                    helperText={udpHost.error}
                    error={!!udpHost.error}
                    value={udpHost.value}
                    margin='dense'
                    fullWidth
                    onChange={(e) => udpHost.setValue(e.target.value)}
                    variant='outlined' />
                <div>
                    <Button onClick={handleUpdateGfw}
                        variant='contained'
                        disabled={updateGfw.loading}>
                        {updateGfw.loading ? 'Updating' : 'Update GFW List'}
                    </Button>
                    &nbsp;
                    <Button onClick={handleUpdateAbp}
                        variant='contained'
                        disabled={updateAbp.loading}>
                        {updateAbp.loading ? 'Updating' : 'Update ABP List'}
                    </Button>
                </div>

            </Stack>
        </DialogContent>
        <DialogActions>
            <Button onClick={onCancelled} disabled={request.loading}>Cancel</Button>

            <Button onClick={handleSave} variant='contained' disabled={request.loading}>Save</Button>
        </DialogActions>
        {snackbar}
    </Dialog>;
}