
export type UpstreamConfig = {
    address: string,
    accept: string[],
    reject: string[],
    priority: number,
}

type ClientConfig = {
    socks5_address: string,
    socks5_udp_host: string,
    upstreams: { [name: string]: UpstreamConfig },
}

export type UpstreamStatistics = {
    tx: number,
    rx: number,
    last_activity: number,
}

type ClientStatistics = {
    upstreams: { [name: string]: UpstreamStatistics },
}

export type ClientConfigWithStats = {
    config: ClientConfig,
    stats: ClientStatistics,
}