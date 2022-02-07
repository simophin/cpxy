import { useEffect, useMemo, useState } from "react";
import { ClientConfig, ClientStatistics, UpstreamConfig, UpstreamStatistics, UpstreamUpdate } from "./models";
import { BASE_URL } from './config';
import _ from 'lodash';
import { Button, Chip, Fab, List, ListItem, ListItemIcon, ListItemText, Switch, Typography } from "@mui/material";
import { Add, ArrowDownward, ArrowUpward } from "@mui/icons-material";
import UpstreamEdit from "./UpstreamEdit";
import useSnackbar from "./useSnackbar";
import useHttp from "./useHttp";

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
        {last_latency > 0 && <Chip style={{ marginRight: 4 }}
            color='primary' label={`${last_latency}ms`} size='small' />}
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

export default function UpstreamList({ }: Props) {
    const configRequest = useHttp<ClientConfig>(`${BASE_URL}/api/config`);
    const statsRequest = useHttp<ClientStatistics>(`${BASE_URL}/api/stats`);
    const upstreamRequest = useHttp(`${BASE_URL}/api/upstream`, { headers: { 'content-type': 'application/json' } });

    const [editing, setEditing] = useState<EditState<string>>({ state: 'idle' });
    const [snackbar, showSnackbar] = useSnackbar();

    const { data: statsData, error: statsError } = statsRequest;
    const { data: configData, error: configError } = configRequest;

    // Start querying right away
    useEffect(() => {
        statsRequest.execute();
        configRequest.execute();
    }, []);

    // Repeat stats query
    useEffect(() => {
        const handle = setTimeout(() => {
            statsRequest.execute();
        }, 1000);

        return () => clearTimeout(handle);
    }, [statsData, statsError]);

    // Repeat error config
    useEffect(() => {
        if (configError) {
            const handle = setTimeout(() => {
                configRequest.execute();
            }, 1000);

            return () => clearTimeout(handle);
        }
    }, [configError]);

    const toggleUpstream = async (name: string, config: UpstreamConfig) => {
        try {
            await upstreamRequest.execute('post', [{
                old_name: name,
                name,
                config: {
                    ...config,
                    enabled: !config.enabled,
                }
            } as UpstreamUpdate]);
            configRequest.execute();
        } catch (e: any) {
            showSnackbar(`Error: ${e.message}`);
        }
    };

    const items = useMemo(() => {
        return _.sortBy(_.map(configData?.upstreams, (value, name) => ({ value, name })), ({ value, name }) => name)
            .map(({ value, name }) => {
                const stats = statsData?.upstreams?.[name];
                return <ListItem
                    key={name}
                    secondaryAction={
                        <>
                            {stats && formatStatistics(stats)}
                        </>
                    }>
                    <ListItemIcon>
                        <Switch checked={value.enabled}
                            onChange={() => toggleUpstream(name, value)} />
                    </ListItemIcon>
                    <ListItemText
                        role='link'
                        onClick={() => setEditing({ state: 'editing', value: name })}
                        primary={name} secondary={value.address} />

                </ListItem >;
            });
    }, [configData, statsData]);

    return <>
        {configRequest.loading && !configRequest.data && <Typography style={{ padding: 16 }}>
            Loading...
        </Typography>}

        {configRequest.error && !configRequest.data && <div style={{ padding: 16 }}>
            <p>Error loading configurations: </p>
            <p>{configRequest.error.message}</p>
            <p>
                <Button
                    variant='contained'
                    onClick={() => configRequest.execute()}>
                    Reload
                </Button>
            </p>
        </div>}

        {configData && _.isEmpty(configData.upstreams) &&
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

        {editing.state !== 'idle' && configData && <UpstreamEdit
            editing={editing.state === 'editing' ? editing.value : undefined}
            current_config={configData}
            onCancelled={() => setEditing({ state: 'idle' })}
            onChanged={(name, action) => {
                configRequest.execute();
                setEditing({ state: 'idle' });
                showSnackbar(`Upstream ${name} ${action}`);
            }} />
        }

        {snackbar}
    </>;
}