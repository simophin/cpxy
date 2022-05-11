

export interface TcpManConfig {
    type: "tcpman";
    address: string;
    ssl: boolean;
    allows_udp: boolean;
}

export interface UdpManConfig {
    type: "udpman";
    addr: string;
}

export interface DirectConfig {
    type: "direct";
}

export interface Socks5Config {
    type: "socks5";
    address: string;
    supports_udp: boolean;
}

export interface HttpProxyConfig {
    type: "http";
    address: string;
    ssl: boolean;
}

export type ProtocolConfig = TcpManConfig | UdpManConfig | DirectConfig | Socks5Config | HttpProxyConfig;

export type UpstreamConfig = {
    protocol: ProtocolConfig,
    accept: string[];
    reject: string[];
    priority: number;
    enabled: boolean;
}

export type ClientConfig = {
    socks5_address?: string,
    socks5_udp_host?: string,
    fwmark?: number,
    udp_tproxy_address?: string,
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