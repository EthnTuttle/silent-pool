{ pkgs, src, lab, node, pool, miner, scanner, agent, txd, captaind, probe }:
pkgs.writeShellApplication {
  name = "spc-lab-self-hosted-ark";
  text = ''
    # Default block cadence for the 8 H/s wrapper scenario: a 1-in-320 target,
    # about 40 s per block (docs/LAB-RUNTIME.md section 4). `spc-lab runtime`
    # rejects a repeated --pow-bits, so the default is dropped when the caller
    # passes their own.
    pow_bits=(--pow-bits 0x2000cccc)
    for arg in "$@"; do
      case "$arg" in
        --pow-bits | --pow-bits=*) pow_bits=() ;;
      esac
    done
    exec ${lab}/bin/spc-lab runtime \
      --scenario ${src}/crates/spc-lab/scenarios/lab-ark-wallets.json \
      --node ${node}/bin/rbitcoin-node \
      --pool ${pool}/bin/spc-pool \
      --miner ${miner}/bin/spc-cpu-miner \
      --scanner ${scanner}/bin/spc-scan \
      --wallet-agent-bin ${agent}/bin/spc-agent \
      --txd ${txd}/bin/spc-lab-txd \
      --traffic-probe ${pkgs.python3}/bin/python3 \
      --traffic-probe-arg ${probe}/share/ark-rpc/probe.py \
      --captaind ${captaind}/bin/captaind \
      --grpcurl ${pkgs.grpcurl}/bin/grpcurl \
      --postgres-bin-dir ${pkgs.postgresql_17}/bin \
      --redis-server ${pkgs.redis}/bin/redis-server \
      --redis-cli ${pkgs.redis}/bin/redis-cli \
      "''${pow_bits[@]}" \
      "$@"
  '';
}
