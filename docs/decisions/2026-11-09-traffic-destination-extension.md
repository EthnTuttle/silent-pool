# 2026-11-09 - Bounded traffic destination outputs (T1-DEST)

## Scope

Approved traffic-runtime plan §6, SHA256
`44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`.
The tested T1/J1 baseline is full
`e53118dd3869db3722b4a28bf0028103604dcb7d162320c07fcf9607579dfaaa`.
Stock Bark's published onchain funding addresses require P2TR/P2WPKH
destination admission in the existing traffic signer/manual-payment path.

## Decisions

1. The lab controller, checked-address API, payment-request validator, builder
   defense-in-depth guard and declared-raw-transaction admission MUST admit
   exactly standard P2PKH, P2WPKH and P2TR destination output templates.
   Address-bearing APIs MUST parse with `bitcoin::Address` and
   `require_network(Network::Regtest)`. Mainnet and `tb` witness addresses,
   malformed addresses, invalid witness-program lengths, future witness
   versions, P2WSH, P2SH, nonstandard scripts and OP_RETURN MUST reject.
   Regtest/testnet base58 P2PKH version bytes are shared; the persisted
   regtest run binding remains authoritative for those addresses.
2. This is OUTPUT admission only. Every selected prevout, owned funding
   observation and receive/change derivation remains P2PKH. Witness input
   data and witness prevouts remain refused under T4. A witness destination
   MUST NOT carry a traffic-wallet owner tag; ownership follows exact owned
   P2PKH derivation, not admission of an external script template.
3. Invalid address/script/owner requests MUST reject before payment command
   begin, reservation or signature. The actor MUST preflight output addresses
   before its chain observation can append; accepted intent still requires
   the complete rescan and typed/request checks. Declared raw transactions still verify
   their original legacy SIGHASH_ALL signatures before output admission and
   before any journal mutation. All outputs retain exact declared script
   bytes and amount. No donation or implicit destination fallback exists.
4. Selection MUST price actual output serialization, including the nine
   additional bytes of P2TR over P2PKH. Final measured weight/vbytes and exact
   input-minus-output fee arithmetic remain authoritative. Dust and fee caps
   remain the explicitly declared traffic policy. This does not set Ark
   server fees: the AP-5 consumer discovers real stock-server policy/fees;
   the pool absorbs boarding/refresh costs per SPEC §11.3, and miners' SPc
   tier amounts are not reduced by this lab output-policy extension.
5. Existing immutable payload/replay conflicts, single signer/journal,
   unknown-outcome retention, reorg/maturity checks and budget accounting
   remain in force. Control v1, history 3 and derivation v2 are unchanged.
   T2 bootstrap distribution still targets owned P2PKH scripts. AP-5 may use
   the existing manual/send interface after real native funding/maturity.
   Admission alone is not funded Bitcoin-node or Bark send/receive/refresh/
   offboard evidence. No Ark-server modification is required or authorized.

## Sections edited

- SPEC frontmatter: this lab-tool policy fragment (no payout derivation change).
- `crates/spc-lab/src/traffic/commands.rs`: regtest destination admission.
- `crates/spc-lab/src/traffic/spec.rs`: clarify input-only spend gate comment.
- `crates/spc-lab-txd/src/{wallet,builder,admit,error,adapter}.rs`: all core
  output checks, exact-template shared predicate and accurate diagnostics.
- `crates/spc-lab-txd/src/actor.rs`: output-address preflight before rescan.
- Scoped controller/core/actor conformance tests and address corpus; existing
  cryptographic golden vectors remain byte-identical.

## Consequences

The existing legacy-input transaction path can construct and journal payments
to real stock Bark onchain addresses. The external recipient handles later
witness spends. Pool attribution and Ark order/receipt boundaries (SPEC §11.3)
are unaffected. The review report records offline tests separately from the
pending native-funded node and unmodified-server proof.
