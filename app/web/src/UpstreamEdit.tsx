import { Button, Dialog, DialogActions, DialogContent, DialogContentText, DialogTitle, Stack, TextField } from "@mui/material"
import { useState } from "react";
import useFetch from "use-http";
import { BASE_URL } from "./config";
import { ClientConfig } from "./models"
import _ from 'lodash';

type Props = {
    editing: string | undefined,
    current_config: ClientConfig,
    onChanged: () => unknown,
    onCancelled: () => unknown,
}

type SaveState = { state: 'loading' } | { state: 'error', msg: string } | { state: 'idle' };

export default function UpstreamEdit({ onChanged, onCancelled, editing, current_config }: Props) {
    const existing = editing ? current_config.upstreams[editing] : undefined;
    const [name, setName] = useState(editing ?? '');
    const [address, setAddress] = useState(existing?.address ?? '');
    const [accept, setAccept] = useState(existing?.accept ?? []);
    const [reject, setReject] = useState(existing?.reject ?? []);
    const [priority, setPriority] = useState(existing?.priority ?? 1);
    const { post } = useFetch(`${BASE_URL}/api/config`);
    const [saveState, setSaveState] = useState<SaveState>({ state: 'idle' });
    const handleSave = () => {
        if (name.trim().length === 0) {
            setSaveState({ state: 'error', msg: "Name can't be empty" });
            return;
        }

        let upstreams = current_config.upstreams;
        if (editing) {
            upstreams = _.omit(upstreams, [editing]);
        }
        console.log('upstreams=', upstreams, 'editing=', editing);

        if (upstreams[name]) {
            setSaveState({ state: 'error', msg: `Configuration ${name} exists` });
            return;
        }
        upstreams[name] = {
            address, accept, reject, priority
        };

        let config: ClientConfig = {
            ...current_config,
            upstreams,
        };
        setSaveState({ state: 'loading' });
        post(config).then((res) => {
            setSaveState({ state: 'idle' });
            onChanged();
        }).catch((err) => {
            setSaveState({ state: 'error', msg: err?.toString() ?? 'Unknown error saving config' })
        });
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
                    value={name}
                    label='Name'
                    margin='dense'
                    onChange={v => setName(v.currentTarget.value)}
                />

                <TextField
                    value={address}
                    label='Address'
                    margin='dense'
                    onChange={v => setAddress(v.currentTarget.value)}
                />

                {saveState.state === 'error' && saveState.msg}
            </Stack>
        </DialogContent>
        <DialogActions>
            <Button onClick={onCancelled} disabled={saveState.state === 'loading'}>Cancel</Button>
            <Button onClick={handleSave} variant='contained' disabled={saveState.state === 'loading'}>
                {saveState.state === 'loading' ? 'Saving' : 'Save'}
            </Button>
        </DialogActions>
    </Dialog>
}