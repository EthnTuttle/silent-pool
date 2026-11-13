"""Decode the exact queried Nix derivation, including Nix 2.34's v4 envelope."""
import copy
from pathlib import Path
import re

from common import require
from pool_control import strict_json


def derivation_record(raw, drv):
    value = strict_json(raw)
    require(type(drv) is str and Path(drv).parent == Path('/nix/store') and drv.endswith('.drv'),
            'queried Nix derivation must be an absolute store path')
    require(type(value) is dict, 'actual Nix derivation object required')
    if set(value) == {'derivations', 'version'}:
        require(type(value['version']) is int and value['version'] == 4
                and type(value['derivations']) is dict and set(value['derivations']) == {Path(drv).name},
                'actual Nix v4 derivation census/version')
        record = copy.deepcopy(value['derivations'][Path(drv).name])
        require(type(record) is dict and type(record.get('version')) is int and record['version'] == 4,
                'actual Nix v4 record version')
        require(type(record.get('outputs')) is dict and record['outputs'], 'actual Nix outputs required')
        for output in record['outputs'].values():
            path = output.get('path')
            require(type(path) is str and re.fullmatch(r'[0-9a-z]{32}-[^/]+', path)
                    and path not in {'.', '..'}, 'actual Nix v4 output must be a store basename')
            output['path'] = '/nix/store/' + path
    else:
        require(set(value) == {drv}, 'actual Nix derivation census')
        record = value[drv]
    require(type(record) is dict and type(record.get('env')) is dict, 'actual Nix derivation environment')
    return record
