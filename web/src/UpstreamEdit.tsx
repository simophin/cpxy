import { Button, Dialog, DialogActions, DialogContent, DialogTitle, FormControl, FormControlLabel, FormGroup, InputLabel, MenuItem, Select, Stack, Switch, TextField } from "@mui/material"
import { BASE_URL } from "./config";
import { ClientConfig, DirectConfig, ProtocolConfig, TcpManConfig, UdpManConfig, UpstreamConfig, UpstreamUpdate } from "./models"
import _ from 'lodash';
import useHttp from "./useHttp";
import { FindError, mandatory, useEditState, validAddress } from "./useEditState";
import { transformRule } from "./trafficRules";
import { useEffect, useState } from "react";

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

function UdpManConfigEdit({ initial, onChanged }: {
    initial?: UdpManConfig,
    onChanged: (c: UdpManConfig) => unknown
}) {
    const address = useEditState(initial?.address ?? '', mandatory('Address', validAddress));

    const addressValue = address.value;
    useEffect(() => {
        if (address.validate()) {
            onChanged({
                type: 'udpman',
                address: addressValue,
            })
        }
    }, [addressValue]);

    return <TextField
        value={address.value}
        label='Address'
        margin='dense'
        error={!!address.error}
        helperText={address.error}
        onChange={v => address.setValue(v.currentTarget.value)}
    />;
}

function TcpManConfigEdit({ initial, onChanged }: {
    initial?: TcpManConfig,
    onChanged: (c: TcpManConfig) => unknown,
}) {
    const address = useEditState(initial?.address ?? '', mandatory('Address', validAddress));
    const [tls, setTls] = useState(initial?.tls === true);
    const [allowsUdp, setAllowsUdp] = useState(initial?.allows_udp === true);

    const addressValue = address.value;
    useEffect(() => {
        if (address.validate()) {
            onChanged({
                type: 'tcpman',
                address: addressValue,
                tls,
                allows_udp: allowsUdp,
            })
        }
    }, [addressValue, tls, allowsUdp]);

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
                        checked={tls}
                        onChange={v => setTls(v.currentTarget.checked)}
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
}

export default function UpstreamEdit({ onChanged, onCancelled, editing, current_config }: Props) {
    const existing = editing ? current_config.upstreams[editing] : undefined;
    const name = useEditState(editing ?? '', mandatory('Name', uniqueUpstreamName(editing, current_config)));

    const accept = useEditState(existing?.accept?.join('\n') ?? '', undefined, transformRule);
    const reject = useEditState(existing?.reject?.join('\n') ?? '', undefined, transformRule);
    const priority = useEditState(existing?.priority?.toString() ?? '0', mandatory('Priority'));
    const request = useHttp(`${BASE_URL}/api/upstream`, { headers: { "Content-Type": "application/json" } });

    const [protoType, setProtoType] = useState<ProtocolConfig['type']>();
    const [protoConfig, setProtoConfig] = useState<ProtocolConfig>();

    const handleSave = async () => {
        if (!protoConfig) {
            return;
        }

        try {
            const update: UpstreamUpdate = {
                old_name: editing,
                name: name.validate(),
                config: {
                    enabled: existing?.enabled ?? true,
                    accept: accept.validate(),
                    reject: reject.validate(),
                    priority: parseInt(priority.validate()),
                    protocol: protoConfig,
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

    useEffect(() => {
        setProtoConfig(undefined);
    }, [protoType]);

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

                <FormControl fullWidth>
                    <InputLabel>Protocol</InputLabel>
                    <Select onChange={e => setProtoType(e.target.value as any)}
                        value={protoConfig?.type}>
                        <MenuItem value={'tcpman'}>TCPMan</MenuItem>
                        <MenuItem value={'udpman'}>UDPMan</MenuItem>
                        <MenuItem value={'direct'}>Direct</MenuItem>
                    </Select>
                </FormControl>

                {protoType === 'tcpman' && <TcpManConfigEdit onChanged={setProtoConfig}
                    initial={protoConfig?.type === 'tcpman' ? protoConfig : undefined} />}

                {protoType === 'udpman' && <UdpManConfigEdit onChanged={setProtoConfig}
                    initial={protoConfig?.type === 'udpman' ? protoConfig : undefined} />}

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
            <Button onClick={handleSave} variant='contained' disabled={request.loading || !protoConfig}>
                {request.loading ? 'Saving' : 'Save'}
            </Button>
        </DialogActions>
    </Dialog>
}