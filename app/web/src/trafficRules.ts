import _ from "lodash";

const GEOIP_PAT = /^geoip:[a-z]{2}$/i;
const GFWLIST_PAT = /^gfwlist$/i;
const NETWORK_PAT = /^network:.+?$/i;
const DOMAIN_PAT = /^domain:.+?$/i;

export function transformRule(value: string): string[] {
    const separator = value.indexOf("\r\n") >= 0 ? "\r\n" : "\n";
    return _.map(value.split(separator), (item) => {
        const trimmed = item.trim();
        if (trimmed.length === 0) {
            return undefined;
        }
        if (trimmed.match(GEOIP_PAT) || trimmed.match(GFWLIST_PAT) || trimmed.match(NETWORK_PAT) || trimmed.match(DOMAIN_PAT)) {
            return trimmed;
        } else {
            throw new Error(`Rule ${trimmed} is invalid`);
        }
    }).filter((s) => !!s).map((s) => s ?? '');
}