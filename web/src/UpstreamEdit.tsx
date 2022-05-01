import { Button, Dialog, DialogActions, DialogContent, DialogTitle, FormControl, FormControlLabel, MenuItem, Stack, Switch, TextField } from "@mui/material";
import { forwardRef, useImperativeHandle, useRef, useState } from "react";
import { BASE_URL } from "./config";
import { ClientConfig, ProtocolConfig, TcpManConfig, UdpManConfig, UpstreamUpdate } from "./models";
import { transformRule } from "./trafficRules";
import { FindError, mandatory, useEditState, validAddress } from "./useEditState";
import useHttp from "./useHttp";

type Props = {
    editing: string | undefined,
    current_config: ClientConfig,
    onChanged: (name: string, action: 'saved' | 'deleted') => unknown,
    onCancelled: () => unknown,
}


function uniqueUpstreamName(editing: string | undefined, config: ClientConfig): FindError {
    return (value: string) => {
        if (value !== editing && config.upstreams[value]) {
            return `Name "${value}" exists for other upstream configuration`;
        }
    }
}

interface Validator {
    validate(): ProtocolConfig;
}

const UdpManConfigEdit = forwardRef(({ initial }: {
    initial?: UdpManConfig,
}, ref) => {
    const address = useEditState(initial?.address ?? '', mandatory('Address', validAddress));

    useImperativeHandle(ref, () => ({
        validate(): UdpManConfig {
            return {
                type: 'udpman',
                address: address.validate(),
            }
        }
    }), [address]);

    return <TextField
        value={address.value}
        label='Address'
        margin='dense'
        error={!!address.error}
        helperText={address.error}
        onChange={v => address.setValue(v.currentTarget.value)}
    />;
});

const TcpManConfigEdit = forwardRef(({ initial }: { initial?: TcpManConfig }, ref) => {
    const address = useEditState(initial?.address ?? '', mandatory('Address', validAddress));
    const [ssl, setSsl] = useState(initial?.ssl === true);
    const [allowsUdp, setAllowsUdp] = useState(initial?.allows_udp === true);

    useImperativeHandle(ref, () => ({
        validate(): TcpManConfig {
            return {
                type: 'tcpman',
                address: address.validate(),
                ssl,
                allows_udp: allowsUdp,
            };
        }
    }), [address, allowsUdp]);

    return <>
        <TextField
            value={address.value}
            label='Address'
            margin='dense'
            error={!!address.error}
            helperText={address.error}
            onChange={v => address.setValue(v.currentTarget.value)}
        />

        <div>
            <FormControl>
                <FormControlLabel
                    control={<Switch
                        checked={ssl}
                        onChange={v => setSsl(v.currentTarget.checked)}
                    />}
                    labelPlacement='start'
                    label="TLS" />
            </FormControl>
        </div>

        <div>
            <FormControl>
                <FormControlLabel
                    control={<Switch
                        checked={allowsUdp}
                        onChange={v => setAllowsUdp(v.currentTarget.checked)}
                    />}
                    labelPlacement='start'
                    label="Allows UDP" />
            </FormControl>
        </div>
    </>;
});


export default function UpstreamEdit({ onChanged, onCancelled, editing, current_config }: Props) {
    const existing = editing ? current_config.upstreams[editing] : undefined;
    const name = useEditState(editing ?? '', mandatory('Name', uniqueUpstreamName(editing, current_config)));

    const accept = useEditState(existing?.accept?.join('\n') ?? '', undefined, transformRule);
    const reject = useEditState(existing?.reject?.join('\n') ?? '', undefined, transformRule);
    const priority = useEditState(existing?.priority?.toString() ?? '0', mandatory('Priority'));
    const request = useHttp(`${BASE_URL}/api/upstream`, { headers: { "Content-Type": "application/json" } });

    const [protoType, setProtoType] = useState<ProtocolConfig['type'] | undefined>(existing?.protocol?.type);

    const validatorRef = useRef<Validator>();

    const handleSave = async () => {
        try {
            let protocol: ProtocolConfig | undefined;
            if (protoType !== 'direct') {
                protocol = validatorRef.current?.validate();
                if (!protocol) {
                    return;
                }
            } else {
                protocol = { 'type': 'direct' };
            }

            const update: UpstreamUpdate = {
                old_name: editing,
                name: name.validate(),
                config: {
                    enabled: existing?.enabled ?? true,
                    accept: accept.validate(),
                    reject: reject.validate(),
                    priority: parseInt(priority.validate()),
                    protocol,
                }
            };

            await request.execute('post', [update]);
            onChanged(update.name, 'saved');
        } catch (e) {
            // Do nothing
        }
    };

    const handleDelete = async () => {
        if (editing) {
            try {
                await request.execute('delete', [editing]);
                onChanged(editing, 'deleted');
            } catch (e) { }
        }
    };

    return <Dialog open={true} onClose={onCancelled} fullWidth disableEscapeKeyDown>
        <DialogTitle>
            {editing && `Edit "${editing}"`}
            {!editing && 'New upstream'}
        </DialogTitle>

        <DialogContent>
            <Stack>
                <TextField
                    autoFocus={editing === undefined}
                    value={name.value}
                    label='Name'
                    margin='dense'
                    error={!!name.error}
                    helperText={name.error}
                    onChange={v => name.setValue(v.currentTarget.value)}
                />
                <TextField
                    value={protoType}
                    select
                    label='Protocol'
                    onChange={v => setProtoType(v.target.value as any)}
                    margin='dense'>
                    <MenuItem value={'tcpman'}>TCPMan</MenuItem>
                    <MenuItem value={'udpman'}>UDPMan</MenuItem>
                    <MenuItem value={'direct'}>Direct</MenuItem>
                </TextField>

                {protoType === 'tcpman' && <TcpManConfigEdit
                    ref={validatorRef}
                    initial={existing?.protocol?.type === 'tcpman' ? existing?.protocol : undefined} />}

                {protoType === 'udpman' && <UdpManConfigEdit
                    ref={validatorRef}
                    initial={existing?.protocol?.type === 'udpman' ? existing?.protocol : undefined} />}

                <TextField
                    value={accept.value}
                    label='Accept rules'
                    margin='dense'
                    multiline
                    error={!!accept.error}
                    helperText={accept.error}
                    onChange={v => accept.setValue(v.currentTarget.value)} />

                <TextField
                    value={reject.value}
                    label='Reject rules'
                    margin='dense'
                    multiline
                    error={!!reject.error}
                    helperText={reject.error}
                    onChange={v => reject.setValue(v.currentTarget.value)} />

                <TextField
                    value={priority.value}
                    label='Priority'
                    margin='dense'
                    type='number'
                    error={!!priority.error}
                    helperText={priority.error}
                    onChange={v => priority.setValue(v.currentTarget.value)} />

                {(request.error) && <>
                    <p>Error saving configuration: <br />
                        {request.error ?? 'Unknown error'}</p>
                </>}
            </Stack>
        </DialogContent>
        <DialogActions>
            <Button onClick={onCancelled} disabled={request.loading}>Cancel</Button>
            {editing && <Button onClick={handleDelete} disabled={request.loading} color='error' variant='contained'>Delete</Button>}
            <Button onClick={handleSave} variant='contained' disabled={request.loading}>
                {request.loading ? 'Saving' : 'Save'}
            </Button>
        </DialogActions>
    </Dialog>
}