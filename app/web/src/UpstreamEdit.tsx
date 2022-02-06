import { Button, Dialog, DialogActions, DialogContent, DialogContentText, DialogTitle, Stack, TextField } from "@mui/material"
import { useEffect, useState } from "react";
import useFetch from "use-http";
import { BASE_URL } from "./config";
import { ClientConfig, UpstreamUpdate } from "./models"
import _ from 'lodash';

type Props = {
    editing: string | undefined,
    current_config: ClientConfig,
    onChanged: () => unknown,
    onCancelled: () => unknown,
}

type FindError = (value: string) => string | undefined;

function trimming(value: string) {
    return value.trim();
}

function mandatory(fieldName: string, next?: FindError): FindError {
    return (value: string) => {
        const new_value = value.trim();
        if (new_value.length === 0) {
            return `${fieldName} can not be empty`;
        }

        if (next) {
            return next(new_value);
        }
    }
}

function useEditState<T = string>(initial: string, findError?: FindError, transform?: (value: string) => T) {
    const [text, setText] = useState(initial);
    const [error, setError] = useState<string>();
    useEffect(() => {
        if (text.length > 0) setError(undefined)
    }, [text]);

    return {
        value: text,
        setValue: setText,
        error,
        validate: (): T => {
            const newError = findError ? findError(text) : undefined;
            setError(newError);
            if (newError) {
                throw newError;
            }
            try {
                return transform ? transform(text.trim()) : (text.trim() as unknown as T);
            } catch (e: any) {
                setError(e?.message ?? 'Invalid value');
                throw e;
            }
        }
    }
}

function uniqueUpstreamName(editing: string | undefined, config: ClientConfig): FindError {
    return (value: string) => {
        if (value !== editing && config.upstreams[value]) {
            return `Name "${value}" exists for other upstream configuration`;
        }
    }
}

const GEOIP_PAT = /^geoip:[a-z]{2}$/i;
const GFWLIST_PAT = /^gfwlist$/i;
const NETWORK_PAT = /^network:.+?$/i;

const transformRule = (value: string): string[] => {
    const separator = value.indexOf("\r\n") >= 0 ? "\r\n" : "\n";
    return _.map(value.split(separator), (item) => {
        const trimmed = item.trim();
        if (trimmed.length === 0) {
            return undefined;
        }
        if (trimmed.match(GEOIP_PAT) || trimmed.match(GFWLIST_PAT) || trimmed.match(NETWORK_PAT)) {
            return trimmed;
        } else {
            throw new Error(`Rule ${trimmed} is invalid`);
        }
    }).filter((s) => !!s).map((s) => s ?? '');
}


export default function UpstreamEdit({ onChanged, onCancelled, editing, current_config }: Props) {
    const existing = editing ? current_config.upstreams[editing] : undefined;
    const name = useEditState(editing ?? '', mandatory('Name', uniqueUpstreamName(editing, current_config)));
    const address = useEditState(existing?.address ?? '', mandatory('Address'));
    const accept = useEditState(existing?.accept?.join('\n') ?? '', undefined, transformRule);
    const reject = useEditState(existing?.reject?.join('\n') ?? '', undefined, transformRule);
    const priority = useEditState(existing?.priority?.toString() ?? '0', mandatory('Priority'));
    const request = useFetch(`${BASE_URL}/api/upstream`);
    const [error, setError] = useState<string>();
    const handleSave = async () => {
        setError(undefined);
        try {
            const update: UpstreamUpdate = {
                old_name: editing,
                name: name.validate(),
                config: {
                    address: address.validate(),
                    accept: accept.validate(),
                    reject: reject.validate(),
                    priority: parseInt(priority.validate()),
                }
            };

            await request.post([update]);
            if (request.response.status != 200) {
                setError(await request.response.text());
            } else {
                setError(undefined);
                onChanged();
            }
        } catch (e) {
            // Do nothing
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

                {(error || request.error) && <>
                    <p>Error saving configuration: <br />
                        {error || request.error?.message || 'Unknown error'}</p>
                </>}
            </Stack>
        </DialogContent>
        <DialogActions>
            <Button onClick={onCancelled} disabled={request.loading}>Cancel</Button>
            <Button onClick={handleSave} variant='contained' disabled={request.loading}>
                {request.loading ? 'Saving' : 'Save'}
            </Button>
        </DialogActions>
    </Dialog>
}