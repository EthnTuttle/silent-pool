{ pkgs, src, lab, node, pool, miner, scanner, agent, txd, captaind, probe }:
pkgs.writeShellApplication {
  name = "spc-lab-self-hosted-ark";
  text = ''
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
      "$@"
  '';
}
