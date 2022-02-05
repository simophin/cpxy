import { useEffect, useMemo, useState } from "react";
import useFetch from "use-http";
import { ClientConfigWithStats, UpstreamStatistics } from "./models";
import { BASE_URL } from './config';
import _ from 'lodash';
import { Button, Checkbox, Chip, Fab, List, ListItem, ListItemText, Typography } from "@mui/material";
import { Add, ArrowDownward, ArrowUpward } from "@mui/icons-material";

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

function formatStatistics({ tx, rx }: UpstreamStatistics) {
    return <>
        <Chip icon={<ArrowUpward />} style={{ marginRight: 4 }}
            color='info' label={formatBytes(tx)} size='small' />
        <Chip icon={<ArrowDownward />}
            color='success' label={formatBytes(rx)} size='small' />
    </>
}

export default function UpstreamList({ reloadList }: Props) {
    const [reload, setReload] = useState(0);
    const { loading, error, data } = useFetch<ClientConfigWithStats>(`${BASE_URL}/api/config`, {},
        [BASE_URL, reload, reloadList]);

    const [selectMode, setSelectMode] = useState(false);
    const [selected, setSelected] = useState<string[]>([]);

    useEffect(() => {
        const handle = setInterval(() => {
            setReload(reload + 1);
        }, 1000);

        return () => clearInterval(handle);
    }, [setReload]);

    const items = useMemo(() => {
        return _.map(data?.config?.upstreams, (value, name) => {
            const stats = data?.stats?.upstreams?.[name];
            return <ListItem
                onClick={() => {
                    if (selectMode) {
                        if (selected.indexOf(name) >= 0) {
                            setSelected(selected.filter(n => n != name));
                        } else {
                            setSelected([...selected, name]);
                        }
                    } else {
                        //TODO: edit upstream
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
    }, [data, selectMode, selected]);

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
                    onClick={() => setReload(reload + 1)}>
                    Reload
                </Button>
            </p>
        </div>}

        {data && _.isEmpty(data.config.upstreams) &&
            <Typography style={{ padding: 16 }}>No upstream configs</Typography>
        }

        {items.length > 0 &&
            <List>{items}</List>
        }

        <Fab color='primary' style={{ position: 'absolute', right: 24, bottom: 24 }}>
            <Add />
        </Fab>
    </>;
}