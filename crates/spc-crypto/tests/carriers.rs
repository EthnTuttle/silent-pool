//! Carrier round-trip tests (SPEC §2.4, §8.1).

use spc_crypto::carriers::{
    emit_op_return, emit_script_sig, height_push_len, parse_carrier, parse_op_return,
    parse_script_sig, script_sig_budget, TxOut,
};

// ---------------------------------------------------------------------------
// scriptSig carrier
// ---------------------------------------------------------------------------

#[test]
fn scriptsig_roundtrip_800000() {
    let a_x = [0x42u8; 32];
    let script = emit_script_sig(800_000, &a_x);
    let parsed = parse_script_sig(&script).unwrap().unwrap();
    assert_eq!(parsed, a_x);
}

#[test]
fn scriptsig_roundtrip_16777216() {
    let a_x = [0x11u8; 32];
    let script = emit_script_sig(16_777_216, &a_x);
    let parsed = parse_script_sig(&script).unwrap().unwrap();
    assert_eq!(parsed, a_x);
}

#[test]
fn scriptsig_byte_exact_800000() {
    // height 800000 = 0x000C3500 in LE = [0x00, 0x35, 0x0c]
    // push = 0x03 0x00 0x35 0x0c  (4 bytes)
    // then OP_PUSHBYTES_32 + 32 bytes = 33 bytes
    // total prefix = 37 bytes
    let a_x = [0xabu8; 32];
    let script = emit_script_sig(800_000, &a_x);
    assert_eq!(
        script.len(),
        4 + 33,
        "800000 height uses 4-byte push + push32=37"
    );
    assert_eq!(script[0], 0x03); // OP_PUSHBYTES_3
    assert_eq!(&script[1..4], &[0x00, 0x35, 0x0c]);
    assert_eq!(script[4], 0x20); // OP_PUSHBYTES_32
    assert_eq!(&script[5..37], &a_x);
}

#[test]
fn scriptsig_byte_exact_16777216() {
    // height 16777216 = 0x01_00_00_00 in LE = [0x00, 0x00, 0x00, 0x01]
    // push = 0x04 0x00 0x00 0x00 0x01  (5 bytes)
    // then OP_PUSHBYTES_32 + 32 bytes
    // total prefix = 38 bytes
    let a_x = [0xbbu8; 32];
    let script = emit_script_sig(16_777_216, &a_x);
    assert_eq!(
        script.len(),
        5 + 33,
        "16777216 height uses 5-byte push + push32=38"
    );
    assert_eq!(script[0], 0x04); // OP_PUSHBYTES_4
    assert_eq!(&script[1..5], &[0x00, 0x00, 0x00, 0x01]);
    assert_eq!(script[5], 0x20); // OP_PUSHBYTES_32
    assert_eq!(&script[6..38], &a_x);
}

#[test]
fn scriptsig_height_push_lengths() {
    // Heights <= 16,777,215 should use a 4-byte push (or less for small values).
    // Height push len for 800000 = 4.
    assert_eq!(height_push_len(800_000), 4);
    // Height push len for 16777216 = 5.
    assert_eq!(height_push_len(16_777_216), 5);
    // Height push len for 0 = 1.
    assert_eq!(height_push_len(0), 1);
    // Height push len for 1..16 = 1 (single opcode).
    for h in 1u32..=16 {
        assert_eq!(height_push_len(h), 1, "height {h}");
    }
}

// ---------------------------------------------------------------------------
// OP_RETURN carrier
// ---------------------------------------------------------------------------

#[test]
fn op_return_roundtrip() {
    let a_x = [0xddu8; 32];
    let script = emit_op_return(&a_x);
    let bytes = script.as_bytes();
    // Must be exactly 37 bytes.
    assert_eq!(bytes.len(), 37);
    assert_eq!(bytes[0], 0x6a); // OP_RETURN
    assert_eq!(bytes[1], 0x23); // OP_PUSHBYTES_35
    assert_eq!(&bytes[2..5], &[0x53, 0x50, 0x63]); // "SPc"
    assert_eq!(&bytes[5..], &a_x);

    let vouts = vec![TxOut {
        value: 0,
        script_pubkey: bytes,
    }];
    let parsed = parse_op_return(&vouts).unwrap();
    assert_eq!(parsed, a_x);
}

#[test]
fn op_return_wrong_magic_rejected() {
    let a_x = [0x42u8; 32];
    let script = emit_op_return(&a_x);
    let mut bad = script.as_bytes().to_vec();
    bad[3] = 0xff; // corrupt "SPc" magic
    let vouts = vec![TxOut {
        value: 0,
        script_pubkey: &bad,
    }];
    assert_eq!(parse_op_return(&vouts), None);
}

#[test]
fn op_return_wrong_length_rejected() {
    // Too short.
    let short = [0x6au8, 0x23, 0x53, 0x50, 0x63];
    let vouts = vec![TxOut {
        value: 0,
        script_pubkey: &short,
    }];
    assert_eq!(parse_op_return(&vouts), None);
}

#[test]
fn op_return_nonzero_value_rejected() {
    // SPEC §2.4.2: the carrier output's value MUST be 0. A script-shaped
    // output carrying value is not an SPc carrier.
    let a_x = [0x99u8; 32];
    let script = emit_op_return(&a_x);
    let vouts = vec![TxOut {
        value: 1_000,
        script_pubkey: script.as_bytes(),
    }];
    assert_eq!(parse_op_return(&vouts), None);
}

// ---------------------------------------------------------------------------
// Carrier precedence (SPEC §2.5)
// ---------------------------------------------------------------------------

#[test]
fn scriptsig_takes_precedence_over_op_return() {
    let a_x_sig = [0x01u8; 32];
    let a_x_ret = [0x02u8; 32];
    let script = emit_script_sig(800_000, &a_x_sig);
    let or_script = emit_op_return(&a_x_ret);
    let vouts = vec![TxOut {
        value: 0,
        script_pubkey: or_script.as_bytes(),
    }];
    let parsed = parse_carrier(&script, &vouts).unwrap();
    assert_eq!(parsed, a_x_sig, "scriptSig must win");
}

#[test]
fn fallback_to_op_return_when_no_scriptsig() {
    let a_x = [0x42u8; 32];
    let or_script = emit_op_return(&a_x);
    let vouts = vec![TxOut {
        value: 0,
        script_pubkey: or_script.as_bytes(),
    }];
    let parsed = parse_carrier(&[], &vouts).unwrap();
    assert_eq!(parsed, a_x);
}

#[test]
fn neither_present_returns_none() {
    assert_eq!(parse_carrier(&[], &[]), None);
}

// ---------------------------------------------------------------------------
// Budget (SPEC §2.4.1)
// ---------------------------------------------------------------------------

#[test]
fn budget_800000_default_extranonce() {
    // 4 (height) + 33 (push32) + 16 (extranonce) = 53 <= 100
    assert!(script_sig_budget(800_000, 16));
}

#[test]
fn budget_16777216_default_extranonce() {
    // 5 (height) + 33 + 16 = 54 <= 100
    assert!(script_sig_budget(16_777_216, 16));
}

#[test]
fn budget_800000_boundary() {
    // 4 + 33 + 63 = 100 exactly.
    assert!(script_sig_budget(800_000, 63));
    assert!(!script_sig_budget(800_000, 64));
}

#[test]
fn budget_16777216_boundary() {
    // 5 + 33 + 62 = 100 exactly.
    assert!(script_sig_budget(16_777_216, 62));
    assert!(!script_sig_budget(16_777_216, 63));
}

#[test]
fn budget_zero_extranonce() {
    // 4 + 33 + 0 = 37 <= 100
    assert!(script_sig_budget(800_000, 0));
}
