import { useEffect, useMemo, useState } from "react";
import useFetch from "use-http";
import { ClientConfig, ClientStatistics, UpstreamStatistics } from "./models";
import { BASE_URL } from './config';
import _ from 'lodash';
import { Button, Checkbox, Chip, Fab, List, ListItem, ListItemText, Typography } from "@mui/material";
import { Add, ArrowDownward, ArrowUpward } from "@mui/icons-material";
import UpstreamEdit from "./UpstreamEdit";

type Props = {
    reloadList?: any,
}

const ONE_KB = 1024;
const ONE_MB = ONE_KB * 1024;
const ONE_GB = ONE_MB * 1024;
const ONE_TB = ONE_GB * 1024;

function formatBytes(v: number) {
    if (v < ONE_MB) {
        return `${(v / ONE_KB).toFixed(0)}KB`;
    }

    if (v < ONE_GB) {
        return `${(v / ONE_MB).toFixed(0)}MB`;
    }

    if (v < ONE_TB) {
        return `${(v / ONE_GB).toFixed(2)}GB`;
    }

    return `${(v / ONE_TB).toFixed(4)}TB`;
}

function formatStatistics({ tx, rx, last_latency }: UpstreamStatistics) {
    return <>
        <Chip style={{ marginRight: 4 }}
            color='primary' label={`${last_latency}ms`} size='small' />
        <Chip icon={<ArrowUpward />} style={{ marginRight: 4 }}
            color='info' label={formatBytes(tx)} size='small' />
        <Chip icon={<ArrowDownward />}
            color='success' label={formatBytes(rx)} size='small' />
    </>
}

type EditState<T> = {
    state: 'editing',
    value: T
} | {
    state: 'adding'
} | {
    state: 'idle'
};

export default function UpstreamList({ reloadList }: Props) {
    const [reload, setReload] = useState(Date.now());
    const [reloadStats, setReloadStats] = useState(Date.now());
    const { loading, error, data } = useFetch<ClientConfig>(`${BASE_URL}/api/config?t=${reload}`, { timeout: 1000, }, [reload]);
    const { data: clientStats, error: clientStatsError } = useFetch<ClientStatistics>(`${BASE_URL}/api/stats?t=${reloadStats}`, {
        timeout: 1000,
    }, [reloadStats]);

    const [selectMode, setSelectMode] = useState(false);
    const [selected, setSelected] = useState<string[]>([]);
    const [editing, setEditing] = useState<EditState<string>>({ state: 'idle' });

    useEffect(() => {
        const handle = setTimeout(() => {
            setReloadStats(Date.now());
        }, clientStatsError ? 2000 : 1000);

        return () => clearTimeout(handle);
    }, [setReloadStats, clientStats, clientStatsError]);

    const items = useMemo(() => {
        return _.map(data?.upstreams, (value, name) => {
            const stats = clientStats?.upstreams?.[name];
            return <ListItem
                onClick={() => {
                    if (selectMode) {
                        if (selected.indexOf(name) >= 0) {
                            setSelected(selected.filter(n => n != name));
                        } else {
                            setSelected([...selected, name]);
                        }
                    } else {
                        setEditing({ state: 'editing', value: name });
                    }
                }}
                secondaryAction={
                    <>
                        {stats && formatStatistics(stats)}
                        {selectMode && <Checkbox checked={selected.indexOf(name) >= 0} />}
                    </>
                }>
                <ListItemText primary={<>
                    {name}

                </>} secondary={value.address}>
                </ListItemText>

            </ListItem >;
        });
    }, [data, selectMode, selected, clientStats]);

    return <>
        {loading && !data && <Typography style={{ padding: 16 }}>
            Loading...
        </Typography>}

        {error && <div style={{ padding: 16 }}>
            <p>Error loading configurations: </p>
            <p>{error.message}</p>
            <p>
                <Button
                    variant='contained'
                    onClick={() => setReload(Date.now())}>
                    Reload
                </Button>
            </p>
        </div>}

        {data && _.isEmpty(data.upstreams) &&
            <Typography style={{ padding: 16 }}>No upstream configs</Typography>
        }

        {items.length > 0 &&
            <List>{items}</List>
        }

        <Fab
            color='primary'
            style={{ position: 'absolute', right: 24, bottom: 24 }}
            onClick={() => setEditing({ state: 'adding' })}>
            <Add />
        </Fab>

        {editing.state !== 'idle' && data && <UpstreamEdit
            editing={editing.state === 'editing' ? editing.value : undefined}
            current_config={data}
            onCancelled={() => setEditing({ state: 'idle' })}
            onChanged={() => {
                setReload(Date.now());
                setEditing({ state: 'idle' });
            }} />
        }
    </>;
}