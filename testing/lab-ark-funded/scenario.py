"""AP5's declared default. No implicit small fixture or secondary hash source."""
USERS = ("alice", "bob", "carol", "dave")
POOLS = ("pool-a", "pool-b")
FLOAT_TARGET = 115_000_000
PAID_WINDOW = 110


def scenario(run_id, seed):
    value = {
        "scenario_version": 1, "expanded_access": True,
        "run": {"id": run_id, "seed": seed,
                "time_policy": {"chain_time": "node_header", "miner_ntime": "hold", "mock_node_time": None},
                "warmup": {"blocks": 0, "window_fills": 0, "exclude_from_statistics": True}},
        "network": {"id": "net-regtest", "consensus_profile": {
            "required_capability": "spc.node.regtest.pow_baseline",
            "requirements": {"pow_limit_bits": "207fffff", "initial_target_bits": "200fffff",
                             "no_pow_retargeting": True, "coinbase_maturity_blocks": 100}}},
        "nodes": [], "pools": [], "fronts": [], "identities": [], "miners": [], "wallet_agents": [],
        "resources": {"redis_instances": [], "postgres_databases": []}, "links": [],
        "ark_backends": [{"id": "local-ark", "implementation": "captaind", "listen": "127.0.0.1:0",
            "local": {"deployment": "self_hosted", "node_id": "node-a", "vtxo_lifetime": 432,
                      "max_vtxo_exit_depth": 100, "required_board_confirmations": 3, "min_board_amount_sats": 20000,
                      "round_interval_secs": 10, "round_submit_secs": 2, "round_sign_secs": 2}}],
        "traffic": {"version": 1, "required_capability": "spc.node.regtest.mempool_rpc.v1",
            "protocol": {"control": "spc-lab-tx-control/v1", "history_version": 3},
            "bootstrap": {"enabled": True, "mining_mode": "native", "funding_blocks": 101,
                          "distribution_outputs_per_wallet": 8, "excluded_from_statistics": True},
            "driver": {"actors": [
                {"id": "txd-0", "network_wallets": 4, "user_payment_wallets": 2, "hosts_bootstrap_funding": True},
                {"id": "txd-1", "network_wallets": 4, "user_payment_wallets": 2, "hosts_bootstrap_funding": False}],
                "user_payment_wallets": 4, "history": "traffic/",
                "bootstrap": {"hashes_per_second": 40, "total_attempts": 4194304,
                              "deadline_secs": 3600, "cancel_grace_secs": 30}},
            "rate": {"ordinary_tx_per_sec": 0.2, "pacing": "fixed_jitter"},
            "fees": {"rate_sats_vb": {"min": 1, "max": 20}, "max_fee_sats_per_tx": 100000},
            "sizes": {"classes_vb": [{"vb": 220, "mix": 1.0}], "max_vb": 1000},
            "spend_types": ["p2pkh"],
            "ark": {"enabled": True, "registration_before_mining": True, "chain_effect_budget_tx_per_sec": 0.1},
            "caps": {"max_inflight_broadcasts": 4, "max_unknown_retries": 3,
                     "max_mempool_backlog_tx": 1000, "reorg_resubmit_max": 3},
            "pause_semantics": "no_broadcast_after_ack",
            # Declared in the runtime-normalized form: the QA controller's
            # config_binding case requires the rendered private scenario to
            # equal this traffic block exactly, and the runtime serde-defaults
            # lab_acceptance. True is the funded-campaign opt-in (expanded
            # operator access is declared above); it arms the reviewed
            # acceptance adapters the funded cases require.
            "lab_acceptance": True},
        # The engine requires this declaration to select the bootstrap route.
        # Runtime execution must wait on this same asynchronous operation id.
        "phases": [{"id": "native-bootstrap", "commands": [{"command_id": "ap5-native-bootstrap",
            "kind": {"type": "bootstrap_fund", "driver_id": "txd-0", "node_id": "node-a"}}]}],
        "assertions": [], "statistics": [],
    }
    def link(name, a, b):
        value["links"].append({"id": name, "from": {"kind": a[0], "id": a[1]},
                               "to": {"kind": b[0], "id": b[1]}, "policy": "normal"})
    for suffix in ("a", "b"):
        node, pool, front = "node-" + suffix, "pool-" + suffix, "front-" + suffix + "1"
        value["nodes"].append({"id": node, "network_id": "net-regtest", "implementation": "rbitcoin-node",
            "p2p": {"listen": "127.0.0.1:0"}, "rpc": {"listen": "127.0.0.1:0"},
            "tdp": {"listen": "127.0.0.1:0", "session_cap": 8}})
        value["pools"].append({"id": pool, "network_id": "net-regtest", "node_ids": [node],
            "implementation": "spc-pool", "ark_backend_id": "local-ark",
            "work_profile": {"work_unit_version": 2,
                "reference_target": "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "share_work_multiplier": 2, "max_work_multiplier": 4294967296,
                "max_window_target_units": "18446744073709551616", "window_rule": "network_block_work",
                "window_n": 8, "max_slice_shares": 4, "legacy_share_difficulty": None},
            "storage": {"redis_instance_id": "redis-" + pool, "postgres_database_id": "pg-" + pool}})
        value["fronts"].append({"id": front, "pool_id": pool, "roles": ["front", "payout", "api"],
            "listen": {"sv2": "127.0.0.1:0", "sv1": None, "jd": "127.0.0.1:0", "api": "127.0.0.1:0"}})
        value["resources"]["redis_instances"].append({"id": "redis-" + pool, "owner_pool_id": pool,
            "port": 0, "dir": "redis/" + pool})
        value["resources"]["postgres_databases"].append({"id": "pg-" + pool, "owner_pool_id": pool,
            "database": "spc_lab_" + suffix, "role": "spc_lab_" + suffix, "port": 0})
        link(pool + "-node", ("pool", pool), ("node", node))
        link(pool + "-ark", ("pool", pool), ("ark", "local-ark"))
    link("node-peering", ("node", "node-a"), ("node", "node-b"))
    link("ark-node", ("ark", "local-ark"), ("node", "node-a"))
    for i, user in enumerate(USERS):
        front = "front-a1" if i < 2 else "front-b1"
        value["identities"].append({"id": user, "address_source": {"fixture": "cold-wallet-" + user}})
        value["wallet_agents"].append({"identity_id": user, "front_id": front})
        value["miners"].append({"id": "miner-" + user, "implementation": "spc-cpu-miner",
            "connections": [{"id": "conn-" + user, "front_id": front, "identity_id": user}],
            "workers": [{"id": "worker-" + user, "identity_id": user, "connection_id": "conn-" + user,
                         # total_hashes is declared because the runtime renders it
                         # (serde default null) and config_binding compares the
                         # budget dict for exact equality.
                         "budget": {"hashes_per_second": "10", "total_hashes": None}}]})
        link(user + "-front", ("miner", "miner-" + user), ("front", front))
    return value
