"""Restrict all DEB822 apt sources to amd64 only, so arm64 packages are
fetched exclusively from ports.ubuntu.com (added separately)."""
import os

for fname in os.listdir('/etc/apt/sources.list.d/'):
    if not fname.endswith('.sources'):
        continue
    path = '/etc/apt/sources.list.d/' + fname
    with open(path) as f:
        content = f.read()
    blocks = content.split('\n\n')
    out = []
    for block in blocks:
        if 'Types: deb' in block and 'Architectures:' not in block:
            lines = block.strip().splitlines()
            idx = next((i for i, l in enumerate(lines) if l.startswith('Types:')), len(lines))
            lines.insert(idx + 1, 'Architectures: amd64')
            block = '\n'.join(lines)
        out.append(block)
    with open(path, 'w') as f:
        f.write('\n\n'.join(out))
