"""Post-build public CLI checks, outside the frozen nine-file verifier source."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from common import require


def check(program, fixtures, output=None):
    public = json.loads(Path(fixtures).read_text())['public']
    expected = {'user_pubkey': '030a752219f1b94bbdf8994a0a980cdda08c2ad094cb29dd834878db6dee1612ee',
                'server_pubkey': '0365a81233741893bbe2461b8d479dadc5880594fe6f7479180d5843820af72b62',
                'amount_sats': 10000, 'expiry_height': 101010, 'exit_delta': 2016, 'max_exit_depth': 100}
    base = {'schema': 'spc.lab.ap5.vtxo-proof-request/v1', 'raw_vtxo_hex': public['board_vtxo'],
            'anchor_tx_hex': public['anchor_tx'], 'expected': expected}
    rows = []
    for name in ('signed-board-positive', 'foreign-server-negative'):
        request = json.loads(json.dumps(base))
        if name.endswith('negative'):
            request['expected']['server_pubkey'] = expected['user_pubkey']
        result = subprocess.run([str(program)], input=json.dumps(request).encode(), capture_output=True, timeout=30)
        if name.endswith('positive'):
            value = json.loads(result.stdout)
            require(result.returncode == 0 and value['verified'] is True and value['policy_kind'] == 'pubkey', 'public signed-board CLI check failed')
            require(value['amount_sats'] == 10000 and value['user_pubkey'] == expected['user_pubkey'], 'public signed-board CLI owner/amount mismatch')
        else:
            require(result.returncode != 0 and b'provider mismatch' in result.stderr, 'public foreign-server CLI negative failed')
        rows.append({'name': name, 'exit': result.returncode, 'expected_outcome_passed': True,
                     'stdout_hex': result.stdout.hex(), 'stderr_hex': result.stderr.hex()})
    receipt = {'schema': 'spc.lab.ap5.installed-verifier-checks/v1',
               'binary_sha256': hashlib.sha256(Path(program).read_bytes()).hexdigest(), 'cases': rows}
    if output is not None:
        Path(output).write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    check(*sys.argv[1:])
