# Project-owned canonical r4 sources. Deployment identity is generated from
# actual output bytes; the RPC contract and normalizer are unchanged.
{ pkgs, probeSource ? ../../testing/ark-rpc }:
let
  canonicalFiles = [
    "arkrpc/__init__.py" "arkrpc/cli.py" "arkrpc/client.py"
    "arkrpc/contracts.py" "arkrpc/launchprofile.py" "arkrpc/nodeproc.py"
    "arkrpc/probe.py" "arkrpc/reporting.py" "arkrpc/schemadef.py"
    "arkrpc/selftest.py" "arkrpc/trafficfacts.py"
    "lifecycle_fixture.py" "probe.py" "report.schema.json"
  ];
  fixtureFiles = [
    "fixtures/base_responses.json"
    "fixtures/launch-profile-goldens.json"
    "fixtures/launch-profile-rpc-policy-goldens.json"
  ] ++ map (name: "fixtures/scenarios/${name}.json") [
    "anchor_verbose_tx" "auth_fail" "baseline_190000" "gettxout_anchor_rejected"
    "id_echo_bool" "id_echo_missing" "id_echo_null" "id_echo_wrong_int"
    "id_string_echo" "id_string_echo_errorpath" "method_not_found" "no_txindex"
    "protocol_error_parse" "shape_mismatch" "traffic_mempoolinfo_ok"
    "traffic_selected_consumer" "version_310000_only" "version_missing"
    "version_wrong_type" "wrong_genesis" "wrong_network"
  ];
  source = pkgs.lib.cleanSourceWith {
    src = probeSource;
    name = "spc-ark-canonical-probe-source";
    filter = path: type:
      let rel = pkgs.lib.removePrefix (toString probeSource + "/") (toString path);
      in (type == "directory" && (toString path == toString probeSource
            || builtins.elem rel [ "arkrpc" "fixtures" "fixtures/scenarios" ]))
        || (type == "regular" && builtins.elem rel (canonicalFiles ++ fixtureFiles));
  };
in pkgs.runCommand "spc-ark-canonical-probe-r4" {
  passthru = { inherit canonicalFiles fixtureFiles source; };
} ''
  mkdir -p $out/share/ark-rpc $out/bin
  cp -r ${source}/. $out/share/ark-rpc/
  chmod u+w $out/share/ark-rpc/lifecycle_fixture.py
  substituteInPlace $out/share/ark-rpc/lifecycle_fixture.py \
    --replace-fail 'sealed-L2-eb257af7' 'rbitcoin-node-ark-lab-release-r2'
  cat > $out/bin/ark-lab-profile-check <<EOF
  #!${pkgs.runtimeShell}
  exec ${pkgs.python3}/bin/python3 $out/share/ark-rpc/lifecycle_fixture.py "\$@"
  EOF
  cat > $out/bin/ark-rpc-probe <<EOF
  #!${pkgs.runtimeShell}
  exec ${pkgs.python3}/bin/python3 $out/share/ark-rpc/probe.py "\$@"
  EOF
  chmod +x $out/bin/*
  PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$out/share/ark-rpc ${pkgs.python3}/bin/python3 -c \
    'from arkrpc.contracts import contract_hash; assert contract_hash() == "754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9"'
  PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$out/share/ark-rpc ${pkgs.python3}/bin/python3 - "$out" <<'PY'
  import hashlib, json, sys
  from pathlib import Path
  from arkrpc.contracts import contract_hash
  out = Path(sys.argv[1])
  root = out / 'share/ark-rpc'
  canonical = json.loads('${builtins.toJSON canonicalFiles}')
  fixtures = json.loads('${builtins.toJSON fixtureFiles}')
  def sha(path):
      return hashlib.sha256(path.read_bytes()).hexdigest()
  def write(name, value):
      path = out / 'share' / name
      path.write_text(json.dumps(value, sort_keys=True, indent=2) + '\n')
      return path
  source_manifest = write('ark-rpc-source-manifest.json', {
      'schema': 'spc.lab.ap5.probe-source/v1', 'root': str(root),
      'files': {name: sha(root / name) for name in sorted(canonical)}})
  fixture_manifest = write('ark-rpc-fixtures-manifest.json', {
      'schema': 'spc.lab.probe-fixtures/v1', 'root': str(root),
      'files': {name: sha(root / name) for name in sorted(fixtures)}})
  python = Path(sys.executable)
  with python.open('rb') as stream:
      assert stream.read(4) == b'\x7fELF', 'native ELF Python required'
  def binding(path):
      return {'path': str(path), 'sha256': sha(path)}
  write('ark-rpc-program-binding.json', {
      'schema': 'spc.lab.probe-program/v1', 'contract_sha256': contract_hash(),
      'native_python': binding(python), 'native_python_resolved': str(python.resolve()),
      'probe_script': binding(root / 'probe.py'),
      'wrappers': {name: binding(out / 'bin' / name)
                   for name in ('ark-rpc-probe', 'ark-lab-profile-check')},
      'source_manifest': binding(source_manifest),
      'fixture_manifest': binding(fixture_manifest)})
  PY
''
