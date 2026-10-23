"""Summarize interpreter fallbacks and locate captured RAM bytes in the ROM."""
import argparse
import collections
import csv
import json
from pathlib import Path


def summarize(rows, rom):
    reasons = collections.Counter()
    regions = collections.Counter()
    sites = {}
    for row in rows:
        key = int(row['key'], 16)
        count = int(row['steps'])
        reasons[row['reason']] += count
        region = {2: 'EWRAM', 3: 'IWRAM'}.get(key >> 24, 'ROM' if 8 <= key >> 24 <= 13 else 'other')
        regions[region] += count
        site = sites.setdefault(key, {'key': f'{key:08x}', 'region': region, 'steps': 0, 'variants': {}})
        site['steps'] += count
        site['variants'][row['bytes32']] = site['variants'].get(row['bytes32'], 0) + count
    result = []
    for site in sorted(sites.values(), key=lambda s: (-s['steps'], s['key'])):
        variants = []
        for hex_bytes, count in sorted(site['variants'].items(), key=lambda v: -v[1]):
            data = bytes.fromhex(hex_bytes)
            # Short/zero windows have too many accidental matches to infer a source.
            offset = rom.find(data) if len(data) == 32 and any(data) else -1
            variants.append({'bytes32': hex_bytes, 'steps': count,
                             'rom_candidate': f'0x{0x08000000 + offset:08x}' if offset >= 0 else None})
        site['variants'] = variants
        result.append(site)
    return {'interpreted_instructions': sum(reasons.values()), 'by_reason': dict(reasons),
            'by_region': dict(regions), 'sites': result}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audit', type=Path, required=True)
    parser.add_argument('--rom', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with args.audit.open(newline='') as source:
        report = summarize(csv.DictReader(source), args.rom.read_bytes())
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(report, output, indent=2)
    print(f"Interpreted instructions: {report['interpreted_instructions']}")
    print('Reasons:', report['by_reason'])
    print('Regions:', report['by_region'])
    for site in report['sites'][:20]:
        print(site['key'], site['region'], site['steps'], 'variants=', len(site['variants']),
              'ROM candidate=', site['variants'][0]['rom_candidate'])


if __name__ == '__main__':
    main()
