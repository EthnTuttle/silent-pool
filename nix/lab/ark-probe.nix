# Unmodified canonical r4 parser/normalizer, plus its already-reviewed owned
# fixture. The fixture's source path and descriptive node label alone change.
{ pkgs, probeSource ? "/tmp/opencode/self-hosted-ark-probe-src" }:
let
  source = builtins.fetchTree {
    type = "path";
    path = probeSource;
    narHash = "sha256-K1yy58S8HPIaxb/xmENcv26pVHsBCdWGn9fCJDItCao=";
  };
in pkgs.runCommand "spc-ark-canonical-probe-r4" { } ''
  mkdir -p $out/share/ark-rpc $out/bin
  cp -r ${source}/. $out/share/ark-rpc/
  chmod u+w $out/share/ark-rpc/lifecycle_fixture.py
  substituteInPlace $out/share/ark-rpc/lifecycle_fixture.py \
    --replace-fail '/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-probe-profile/testing/ark-rpc' "$out/share/ark-rpc" \
    --replace-fail 'node-profile-facts-r2-36e2387a' 'rbitcoin-node-ark-lab-release-r2'
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
''
