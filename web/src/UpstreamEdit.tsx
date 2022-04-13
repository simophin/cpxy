import { Button, Dialog, DialogActions, DialogContent, DialogTitle, FormControl, FormControlLabel, FormGroup, Stack, Switch, TextField } from "@mui/material"
import { BASE_URL } from "./config";
import { ClientConfig, UpstreamUpdate } from "./models"
import _ from 'lodash';
import useHttp from "./useHttp";
import { FindError, mandatory, useEditState, validAddress } from "./useEditState";
import { transformRule } from "./trafficRules";
import { useState } from "react";

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

export default function UpstreamEdit({ onChanged, onCancelled, editing, current_config }: Props) {
    const existing = editing ? current_config.upstreams[editing] : undefined;
    const name = useEditState(editing ?? '', mandatory('Name', uniqueUpstreamName(editing, current_config)));
    const address = useEditState(existing?.address ?? '', mandatory('Address', validAddress));
    const [tls, setTls] = useState(existing?.tls === true);
    const accept = useEditState(existing?.accept?.join('\n') ?? '', undefined, transformRule);
    const reject = useEditState(existing?.reject?.join('\n') ?? '', undefined, transformRule);
    const priority = useEditState(existing?.priority?.toString() ?? '0', mandatory('Priority'));
    const request = useHttp(`${BASE_URL}/api/upstream`, { headers: { "Content-Type": "application/json" } });
    const handleSave = async () => {
        try {
            const update: UpstreamUpdate = {
                old_name: editing,
                name: name.validate(),
                config: {
                    enabled: existing?.enabled ?? true,
                    tls,
                    address: address.validate(),
                    accept: accept.validate(),
                    reject: reject.validate(),
                    priority: parseInt(priority.validate()),
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