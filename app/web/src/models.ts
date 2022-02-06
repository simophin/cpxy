
export type UpstreamConfig = {
    address: string,
    accept: string[],
    reject: string[],
    priority: number,
}

export type ClientConfig = {
    socks5_address?: string,
    socks5_udp_host?: string,
    upstreams: { [name: string]: UpstreamConfig },
}

export type UpstreamStatistics = {
    tx: number,
    rx: number,
    last_activity: number,
    last_latency: number,
}

export type ClientStatistics = {
    upstreams: { [name: string]: UpstreamStatistics },
}

export type UpstreamUpdate = {
    old_name?: string;
    name: string;
    config: UpstreamConfig;
}