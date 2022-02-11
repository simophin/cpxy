import _ from "lodash";



const PATTERNS = [
    /^geoip:[a-z]{2}$/i,
    /^gfwlist$/i,
    /^network:.+?$/i,
    /^domain:.+?$/i,
    /^adblocklist$/i,
]

export function transformRule(value: string): string[] {
    const separator = value.indexOf("\r\n") >= 0 ? "\r\n" : "\n";
    return _.map(value.split(separator), (item) => {
        const trimmed = item.trim();
        if (trimmed.length === 0) {
            return undefined;
        }

        for (const pat of PATTERNS) {
            if (trimmed.match(pat)) {
                return trimmed;
            }
        }

        throw new Error(`Rule ${trimmed} is invalid`);
    }).filter((s) => !!s).map((s) => s ?? '');
}