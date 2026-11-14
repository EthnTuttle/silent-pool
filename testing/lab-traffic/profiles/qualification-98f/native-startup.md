Running phase: unpackPhase
@nix { "action": "setPhase", "phase": "unpackPhase" }
unpacking source archive /nix/store/6hwxw1jm24l2is2cd9mnnkxg0i0dsqf7-source
source root is source
Executing cargoSetupPostUnpackHook
Finished cargoSetupPostUnpackHook
Running phase: patchPhase
@nix { "action": "setPhase", "phase": "patchPhase" }
Executing cargoSetupPostPatchHook
Validating consistency between /build/source/Cargo.lock and /build/cargo-deps-vendor/Cargo.lock
Finished cargoSetupPostPatchHook
Running phase: updateAutotoolsGnuConfigScriptsPhase
@nix { "action": "setPhase", "phase": "updateAutotoolsGnuConfigScriptsPhase" }
Running phase: configurePhase
@nix { "action": "setPhase", "phase": "configurePhase" }
Running phase: buildPhase
@nix { "action": "setPhase", "phase": "buildPhase" }
[1m[92m   Compiling[0m proc-macro2 v1.0.107
[1m[92m   Compiling[0m quote v1.0.47
[1m[92m   Compiling[0m unicode-ident v1.0.27
[1m[92m   Compiling[0m libc v0.2.191
[1m[92m    Checking[0m cfg-if v1.0.5
[1m[92m   Compiling[0m serde v1.0.229
[1m[92m   Compiling[0m version_check v0.9.5
[1m[92m   Compiling[0m serde_core v1.0.229
[1m[92m   Compiling[0m zerocopy v0.8.63
[1m[92m   Compiling[0m shlex v2.0.1
[1m[92m   Compiling[0m find-msvc-tools v0.1.15
[1m[92m    Checking[0m typenum v1.20.1
[1m[92m   Compiling[0m memchr v2.8.3
[1m[92m    Checking[0m once_cell v1.21.4
[1m[92m    Checking[0m pin-project-lite v0.2.17
[1m[92m    Checking[0m itoa v1.0.18
[1m[92m   Compiling[0m autocfg v1.5.1
[1m[92m   Compiling[0m parking_lot_core v0.9.12
[1m[92m   Compiling[0m zmij v1.0.23
[1m[92m    Checking[0m subtle v2.6.1
[1m[92m    Checking[0m arrayvec v0.7.8
[1m[92m    Checking[0m cpufeatures v0.2.17
[1m[92m   Compiling[0m bitcoin-io v0.1.101
[1m[92m    Checking[0m futures-core v0.3.34
[1m[92m    Checking[0m scopeguard v1.2.0
[1m[92m   Compiling[0m icu_normalizer_data v2.4.0
[1m[92m   Compiling[0m cc v1.11.0
[1m[92m   Compiling[0m icu_properties_data v2.4.0
[1m[92m   Compiling[0m object v0.39.1
[1m[92m   Compiling[0m hex_lit v0.1.1
[1m[92m    Checking[0m bytes v1.12.1
[1m[92m   Compiling[0m stable_deref_trait v1.2.1
[1m[92m    Checking[0m hex-conservative v0.2.3
[1m[92m   Compiling[0m serde_json v1.0.154
[1m[92m    Checking[0m tracing-core v0.1.36
[1m[92m   Compiling[0m generic-array v0.14.7
[1m[92m   Compiling[0m ahash v0.8.12
[1m[92m    Checking[0m lock_api v0.4.14
[1m[92m   Compiling[0m bitcoin v0.32.102
[1m[92m   Compiling[0m derive_codec_sv2 v2.0.0
[1m[92m    Checking[0m log v0.4.35
[1m[92m    Checking[0m bech32 v0.11.1
[1m[92m    Checking[0m opaque-debug v0.3.1
[1m[92m    Checking[0m futures-sink v0.3.34
[1m[92m    Checking[0m allocator-api2 v0.2.21
[1m[92m   Compiling[0m num-traits v0.2.19
[1m[92m    Checking[0m byteorder v1.5.0
[1m[92m   Compiling[0m litemap v0.8.3
[1m[92m   Compiling[0m utf_types v1.0.0
[1m[92m   Compiling[0m writeable v0.6.4
[1m[92m    Checking[0m futures-channel v0.3.34
[1m[92m   Compiling[0m rustversion v1.0.23
[1m[92m   Compiling[0m pest_stack_guard v1.0.1
[1m[92m    Checking[0m futures-task v0.3.34
[1m[92m   Compiling[0m utf8_iter v1.1.0
[1m[92m    Checking[0m slab v0.4.12
[1m[92m    Checking[0m hex v0.4.3
[1m[92m   Compiling[0m getrandom v0.3.4
[1m[92m    Checking[0m futures-io v0.3.34
[1m[92m   Compiling[0m crossbeam-utils v0.8.23
[1m[92m   Compiling[0m crunchy v0.2.4
[1m[92m   Compiling[0m syn v3.0.9
[1m[92m   Compiling[0m syn v2.0.119
[1m[92m   Compiling[0m getrandom v0.4.3
[1m[92m    Checking[0m binary_sv2 v7.0.0
[1m[92m   Compiling[0m thiserror v1.0.69
[1m[92m    Checking[0m tinyvec v1.13.3
[1m[92m    Checking[0m iana-time-zone v0.1.65
[1m[92m    Checking[0m lazy_static v1.5.0
[1m[92m   Compiling[0m paste v1.0.15
[1m[92m   Compiling[0m tiny-keccak v2.0.2
[1m[92m    Checking[0m crypto-common v0.1.7
[1m[92m    Checking[0m inout v0.1.4
[1m[92m    Checking[0m block-buffer v0.10.4
[1m[92m    Checking[0m hashbrown v0.17.1
[1m[92m    Checking[0m hex-conservative v0.1.2
[1m[92m   Compiling[0m libm v0.2.16
[1m[92m    Checking[0m universal-hash v0.5.1
[1m[92m    Checking[0m aead v0.5.2
[1m[92m    Checking[0m digest v0.10.7
[1m[92m    Checking[0m unicode-normalization v0.1.26
[1m[92m   Compiling[0m thiserror v2.0.21
[1m[92m    Checking[0m polyval v0.6.2
[1m[92m    Checking[0m static_assertions v1.1.0
[1m[92m    Checking[0m equivalent v1.0.3
[1m[92m    Checking[0m poly1305 v0.8.0
[1m[92m    Checking[0m bitcoin_hashes v0.13.1
[1m[92m    Checking[0m sha2 v0.10.9
[1m[92m    Checking[0m mining_sv2 v12.0.0
[1m[92m    Checking[0m template_distribution_sv2 v7.0.0
[1m[92m    Checking[0m ghash v0.5.1
[1m[92m    Checking[0m framing_sv2 v8.0.0
[1m[92m    Checking[0m getrandom v0.2.17
[1m[92m    Checking[0m errno v0.3.14
[1m[92m    Checking[0m mio v1.2.4
[1m[92m    Checking[0m socket2 v0.6.5
[1m[92m   Compiling[0m secp256k1-sys v0.10.1
[1m[92m   Compiling[0m stacker v0.1.25
[1m[92m    Checking[0m signal-hook-registry v1.4.8
[1m[92m   Compiling[0m secp256k1-sys v0.9.2
[1m[92m   Compiling[0m secp256k1-sys v0.14.1
[1m[92m    Checking[0m rand_core v0.6.4
[1m[92m    Checking[0m rand_core v0.9.5
[1m[92m    Checking[0m rustc-hex v2.1.0
[1m[92m    Checking[0m indexmap v2.14.2
[1m[92m    Checking[0m minimal-lexical v0.2.1
[1m[92m    Checking[0m percent-encoding v2.3.2
[1m[92m    Checking[0m possiblyrandom v0.2.1
[1m[92m    Checking[0m uint v0.10.2
[1m[92m    Checking[0m form_urlencoded v1.2.2
[1m[92m    Checking[0m extensions_sv2 v0.3.0
[1m[92m    Checking[0m nom v7.1.3
[1m[92m    Checking[0m common_messages_sv2 v9.0.0
[1m[92m    Checking[0m job_declaration_sv2 v10.0.0
[1m[92m   Compiling[0m ucd-trie v0.1.7
[1m[92m    Checking[0m dnssec-prover v0.6.10
[1m[92m    Checking[0m hashbrown v0.13.2
[1m[92m    Checking[0m parsers_sv2 v0.6.0
[1m[92m   Compiling[0m const-random-macro v0.1.16
[1m[92m    Checking[0m hex-conservative v0.3.2
[1m[92m    Checking[0m bitflags v2.13.2
[1m[92m   Compiling[0m tokio v1.54.1
[1m[92m   Compiling[0m unicode_categories v0.1.1
[1m[92m   Compiling[0m heck v0.5.0
[1m[92m   Compiling[0m rustix v1.1.6
[1m[92m   Compiling[0m crc-catalog v2.5.0
[1m[92m   Compiling[0m parking v2.2.1
[1m[92m   Compiling[0m futures-util v0.3.34
[1m[92m   Compiling[0m event-listener v5.4.2
[1m[92m   Compiling[0m crossbeam-queue v0.3.14
[1m[92m   Compiling[0m crc v3.4.0
[1m[92m    Checking[0m const-random v0.1.18
[1m[92m   Compiling[0m chrono v0.4.45
[1m[92m   Compiling[0m atoi v2.0.0
[1m[92m   Compiling[0m hmac v0.12.1
[1m[92m   Compiling[0m encoding_rs v0.8.44
[1m[92m    Checking[0m http v1.5.0
[1m[92m   Compiling[0m linux-raw-sys v0.12.1
[1m[92m   Compiling[0m uuid v1.29.0
[1m[92m   Compiling[0m unicode-bidi v0.3.18
[1m[92m   Compiling[0m unicode-properties v0.1.5
[1m[92m   Compiling[0m hkdf v0.12.4
[1m[92m    Checking[0m dlv-list v0.5.2
[1m[92m   Compiling[0m md-5 v0.10.6
[1m[92m    Checking[0m utf8parse v0.2.2
[1m[92m   Compiling[0m httparse v1.10.1
[1m[92m    Checking[0m option-ext v0.2.0
[1m[92m   Compiling[0m stringprep v0.1.5
[1m[92m   Compiling[0m home v0.5.12
[1m[92m    Checking[0m toml_write v0.1.2
[1m[92m    Checking[0m simdutf8 v0.1.5
[1m[92m    Checking[0m http-body v1.1.0
[1m[92m    Checking[0m core_detect v1.0.0
[1m[92m   Compiling[0m multiversion_no_op v1.0.0
[1m[92m   Compiling[0m base64 v0.22.1
[1m[92m    Checking[0m regex-syntax v0.8.11
[1m[92m   Compiling[0m fastrand v2.5.0
[1m[92m   Compiling[0m whoami v1.6.1
[1m[92m    Checking[0m winnow v0.7.15
[1m[92m    Checking[0m ppv-lite86 v0.2.21
[1m[92m   Compiling[0m dotenvy v0.15.7
[1m[92m    Checking[0m hashbrown v0.14.5
[1m[92m    Checking[0m sqlformat v0.2.6
[1m[92m    Checking[0m dirs-sys v0.5.0
[1m[92m    Checking[0m rand_chacha v0.3.1
[1m[92m    Checking[0m rand_chacha v0.9.0
[1m[92m    Checking[0m anstyle-parse v1.0.0
[1m[92m    Checking[0m rand v0.9.5
[1m[92m    Checking[0m rand v0.8.8
[1m[92m    Checking[0m block-buffer v0.9.0
[1m[92m    Checking[0m digest v0.9.0
[1m[92m    Checking[0m is_terminal_polyfill v1.70.2
[1m[92m    Checking[0m anstyle v1.0.14
[1m[92m    Checking[0m arraydeque v0.5.1
[1m[92m   Compiling[0m ar_archive_writer v0.5.3
[1m[92m    Checking[0m base64 v0.21.7
[1m[92m    Checking[0m unicode-segmentation v1.13.3
[1m[92m    Checking[0m hashlink v0.8.4
[1m[92m    Checking[0m hashlink v0.9.1
[1m[92m    Checking[0m regex-automata v0.4.18
[1m[92m    Checking[0m ordered-multimap v0.7.3
[1m[92m    Checking[0m colorchoice v1.0.5
[1m[92m    Checking[0m tower-service v0.3.3
[1m[92m    Checking[0m anstyle-query v1.1.5
[1m[92m   Compiling[0m anyhow v1.0.104
[1m[92m    Checking[0m ryu v1.0.23
[1m[92m    Checking[0m convert_case v0.6.0
[1m[92m    Checking[0m sha2 v0.9.9
[1m[92m    Checking[0m anstream v1.0.0
[1m[92m    Checking[0m fixed-hash v0.8.0
[1m[92m    Checking[0m primitive-types v0.13.1
[1m[92m    Checking[0m rust-ini v0.20.0
[1m[92m    Checking[0m dirs v6.0.0
[1m[92m    Checking[0m yaml-rust2 v0.8.1
[1m[92m    Checking[0m concurrent-queue v2.5.0
[1m[92m   Compiling[0m synstructure v0.14.0
[1m[92m    Checking[0m num-integer v0.1.47
[1m[92m    Checking[0m sharded-slab v0.1.7
[1m[92m   Compiling[0m tempfile v3.27.0
[1m[92m    Checking[0m tracing-log v0.2.0
[1m[92m    Checking[0m hex-conservative v1.3.0
[1m[92m    Checking[0m thread_local v1.1.10
[1m[92m   Compiling[0m psm v0.1.32
[1m[92m    Checking[0m tower-layer v0.3.3
[1m[92m    Checking[0m httpdate v1.0.3
[1m[92m    Checking[0m clap_lex v1.1.1
[1m[92m    Checking[0m atomic-waker v1.1.2
[1m[92m    Checking[0m pathdiff v0.2.3
[1m[92m    Checking[0m event-listener v2.5.3
[1m[92m    Checking[0m sync_wrapper v1.0.2
[1m[92m    Checking[0m strsim v0.11.1
[1m[92m    Checking[0m nu-ansi-term v0.50.3
[1m[92m    Checking[0m num-bigint v0.4.8
[1m[92m    Checking[0m bs58 v0.4.0
[1m[92m    Checking[0m shellexpand v3.1.2
[1m[92m    Checking[0m async-channel v1.9.0
[1m[92m    Checking[0m http-body-util v0.1.5
[1m[92m    Checking[0m clap_builder v4.6.7
[1m[92m    Checking[0m arc-swap v1.9.3
[1m[92m    Checking[0m socket2 v0.5.10
[1m[92m    Checking[0m sha1_smol v1.0.1
[1m[92m    Checking[0m mime v0.3.17
[1m[92m    Checking[0m serde_path_to_error v0.1.20
[1m[92m    Checking[0m matchit v0.8.4
[1m[92m   Compiling[0m tracing-attributes v0.1.31
[1m[92m   Compiling[0m zeroize_derive v1.5.0
[1m[92m   Compiling[0m thiserror-impl v1.0.69
[1m[92m   Compiling[0m lightning-macros v0.2.1
[1m[92m   Compiling[0m trait-variant v0.1.3
[1m[92m   Compiling[0m serde_derive v1.0.229
[1m[92m   Compiling[0m zerofrom-derive v0.1.8
[1m[92m   Compiling[0m yoke-derive v0.8.4
[1m[92m   Compiling[0m zerovec-derive v0.11.7
[1m[92m   Compiling[0m displaydoc v0.2.7
[1m[92m   Compiling[0m tokio-macros v2.7.2
[1m[92m   Compiling[0m futures-macro v0.3.34
[1m[92m   Compiling[0m async-trait v0.1.92
[1m[92m   Compiling[0m thiserror-impl v2.0.21
[1m[92m    Checking[0m matchers v0.2.0
[1m[92m   Compiling[0m clap_derive v4.6.7
[1m[92m    Checking[0m secp256k1 v0.28.2
[1m[92m    Checking[0m handlers_sv2 v0.6.0
[1m[92m    Checking[0m zeroize v1.9.1
[1m[92m    Checking[0m cipher v0.4.4
[1m[92m    Checking[0m chacha20 v0.9.1
[1m[92m    Checking[0m ctr v0.9.2
[1m[92m    Checking[0m aes v0.8.4
[1m[92m   Compiling[0m tokio-stream v0.1.19
[1m[92m    Checking[0m tracing v0.1.44
[1m[92m    Checking[0m chacha20poly1305 v0.10.1
[1m[92m   Compiling[0m zerofrom v0.1.8
[1m[92m   Compiling[0m pest v2.9.5
[1m[92m    Checking[0m noise_sv2 v2.0.0
[1m[92m   Compiling[0m yoke v0.8.4
[1m[92m    Checking[0m axum-core v0.5.6
[1m[92m   Compiling[0m zerovec v0.11.9
[1m[92m   Compiling[0m zerotrie v0.2.5
[1m[92m    Checking[0m aes-gcm v0.10.3
[1m[92m    Checking[0m buffer_sv2 v3.0.1
[1m[92m    Checking[0m codec_sv2 v7.0.0
[1m[92m    Checking[0m tinystr v0.8.5
[1m[92m    Checking[0m potential_utf v0.1.6
[1m[92m    Checking[0m clap v4.6.7
[1m[92m    Checking[0m icu_collections v2.4.0
[1m[92m    Checking[0m icu_locale_core v2.4.0
[1m[92m   Compiling[0m pest_meta v2.9.5
[1m[92m    Checking[0m icu_provider v2.4.0
[1m[92m    Checking[0m icu_properties v2.4.0
[1m[92m    Checking[0m smallvec v1.16.4
[1m[92m    Checking[0m bitcoin_hashes v0.14.101
[1m[92m    Checking[0m bitcoin-units v0.1.101
[1m[92m    Checking[0m secp256k1 v0.33.1
[1m[92m    Checking[0m either v1.19.0
[1m[92m    Checking[0m toml_datetime v0.6.11
[1m[92m    Checking[0m serde_spanned v0.6.9
[1m[92m    Checking[0m ron v0.8.1
[1m[92m    Checking[0m tracing-serde v0.2.0
[1m[92m    Checking[0m serde_urlencoded v0.7.1
[1m[92m   Compiling[0m pest_generator v2.9.5
[1m[92m    Checking[0m itertools v0.13.0
[1m[92m    Checking[0m toml_edit v0.22.27
[1m[92m   Compiling[0m icu_normalizer v2.4.0
[1m[92m    Checking[0m tracing-subscriber v0.3.23
[1m[92m    Checking[0m parking_lot v0.12.5
[1m[92m    Checking[0m dashmap v6.2.1
[1m[92m    Checking[0m futures-executor v0.3.34
[1m[92m    Checking[0m futures-intrusive v0.5.0
[1m[92m    Checking[0m futures v0.3.34
[1m[92m    Checking[0m idna_adapter v1.2.2
[1m[92m    Checking[0m secp256k1 v0.29.1
[1m[92m    Checking[0m base58ck v0.1.101
[1m[92m    Checking[0m bip39 v2.2.2
[1m[92m    Checking[0m idna v1.1.0
[1m[92m   Compiling[0m pest_derive v2.9.5
[1m[92m    Checking[0m url v2.5.8
[1m[92m    Checking[0m json5 v0.4.1
[1m[92m    Checking[0m toml v0.8.23
[1m[92m    Checking[0m config v0.14.1
[1m[92m   Compiling[0m sqlx-core v0.8.0
[1m[92m    Checking[0m tokio-util v0.7.20
[1m[92m    Checking[0m hyper v1.12.0
[1m[92m    Checking[0m tower v0.5.3
[1m[92m   Compiling[0m sqlx-postgres v0.8.0
[1m[92m    Checking[0m combine v4.6.9
[1m[92m    Checking[0m hyper-util v0.1.21
[1m[92m    Checking[0m axum v0.8.9
[1m[92m    Checking[0m lightning-types v0.3.5
[1m[92m    Checking[0m bark-bitcoin-ext v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m    Checking[0m sv1_api v6.0.0
[1m[92m    Checking[0m channels_sv2 v8.0.0
[1m[92m    Checking[0m miniscript v13.2.0
[1m[92m    Checking[0m lightning-invoice v0.34.1
[1m[92m    Checking[0m stratum-core v0.6.0
[1m[92m    Checking[0m lightning v0.2.7
[1m[92m    Checking[0m stratum-apps v0.8.0
[1m[92m   Compiling[0m sqlx-macros-core v0.8.0
[1m[92m    Checking[0m redis v0.27.6
[1m[92m   Compiling[0m sqlx-macros v0.8.0
[1m[92m    Checking[0m sqlx v0.8.0
[1m[92m    Checking[0m ark-lib v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m    Checking[0m spc-crypto v0.1.0 (/build/source/crates/spc-crypto)
[1m[92m    Checking[0m spc-accounting v0.1.0 (/build/source/crates/spc-accounting)
[1m[92m    Checking[0m spc-ext v0.1.0 (/build/source/crates/spc-ext)
[1m[92m    Checking[0m spc-share-ext v0.1.0 (/build/source/crates/spc-share-ext)
[1m[92m    Checking[0m spc-store v0.1.0 (/build/source/crates/spc-store)
[1m[92m    Checking[0m spc-scan v0.1.0 (/build/source/crates/spc-scan)
[1m[92m    Checking[0m spc-pool v0.1.0 (/build/source/bins/spc-pool)
[1m[92m    Checking[0m spc-agent v0.1.0 (/build/source/bins/spc-agent)
[1m[92m    Finished[0m `dev` profile [unoptimized + debuginfo] target(s) in 29.49s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m   Compiling[0m unicode-ident v1.0.27
[1m[92m   Compiling[0m cfg-if v1.0.5
[1m[92m   Compiling[0m once_cell v1.21.4
[1m[92m   Compiling[0m memchr v2.8.3
[1m[92m   Compiling[0m pin-project-lite v0.2.17
[1m[92m   Compiling[0m bytes v1.12.1
[1m[92m   Compiling[0m scopeguard v1.2.0
[1m[92m   Compiling[0m itoa v1.0.18
[1m[92m   Compiling[0m typeid v1.0.3
[1m[92m   Compiling[0m erased-serde v0.4.11
[1m[92m   Compiling[0m typenum v1.20.1
[1m[92m   Compiling[0m futures-core v0.3.34
[1m[92m   Compiling[0m subtle v2.6.1
[1m[92m   Compiling[0m arrayvec v0.7.8
[1m[92m   Compiling[0m libc v0.2.191
[1m[92m   Compiling[0m serde_core v1.0.229
[1m[92m   Compiling[0m zerocopy v0.8.63
[1m[92m   Compiling[0m zmij v1.0.23
[1m[92m   Compiling[0m futures-io v0.3.34
[1m[92m   Compiling[0m equivalent v1.0.3
[1m[92m   Compiling[0m lock_api v0.4.14
[1m[92m   Compiling[0m bitcoin-io v0.1.101
[1m[92m   Compiling[0m hashbrown v0.17.1
[1m[92m   Compiling[0m proc-macro2 v1.0.107
[1m[92m   Compiling[0m base64 v0.21.7
[1m[92m   Compiling[0m allocator-api2 v0.2.21
[1m[92m   Compiling[0m futures-sink v0.3.34
[1m[92m   Compiling[0m bitcoin v0.32.102
[1m[92m   Compiling[0m hex_lit v0.1.1
[1m[92m   Compiling[0m bech32 v0.11.1
[1m[92m   Compiling[0m stable_deref_trait v1.2.1
[1m[92m   Compiling[0m tracing-core v0.1.36
[1m[92m   Compiling[0m futures-task v0.3.34
[1m[92m   Compiling[0m hex-conservative v0.2.3
[1m[92m   Compiling[0m cpufeatures v0.2.17
[1m[92m   Compiling[0m percent-encoding v2.3.2
[1m[92m   Compiling[0m slab v0.4.12
[1m[92m   Compiling[0m futures-channel v0.3.34
[1m[92m   Compiling[0m num-traits v0.2.19
[1m[92m   Compiling[0m utf_types v1.0.0
[1m[92m   Compiling[0m litemap v0.8.3
[1m[92m   Compiling[0m writeable v0.6.4
[1m[92m   Compiling[0m pkg-config v0.3.34
[1m[92m   Compiling[0m untrusted v0.9.0
[1m[92m   Compiling[0m icu_normalizer_data v2.4.0
[1m[92m   Compiling[0m rustls v0.23.46
[1m[92m   Compiling[0m iana-time-zone v0.1.65
[1m[92m   Compiling[0m icu_properties_data v2.4.0
[1m[92m   Compiling[0m base64 v0.23.1
[1m[92m   Compiling[0m binary_sv2 v7.0.0
[1m[92m   Compiling[0m vcpkg v0.2.15
[1m[92m   Compiling[0m opaque-debug v0.3.1
[1m[92m   Compiling[0m hex v0.4.3
[1m[92m   Compiling[0m tower-service v0.3.3
[1m[92m   Compiling[0m tinyvec v1.13.3
[1m[92m   Compiling[0m quote v1.0.47
[1m[92m   Compiling[0m utf8_iter v1.1.0
[1m[92m   Compiling[0m object v0.39.1
[1m[92m   Compiling[0m form_urlencoded v1.2.2
[1m[92m   Compiling[0m try-lock v0.2.5
[1m[92m   Compiling[0m atomic-waker v1.1.2
[1m[92m   Compiling[0m http v1.5.0
[1m[92m   Compiling[0m byteorder v1.5.0
[1m[92m   Compiling[0m fallible-streaming-iterator v0.1.9
[1m[92m   Compiling[0m base64 v0.13.1
[1m[92m   Compiling[0m indexmap v2.14.2
[1m[92m   Compiling[0m fallible-iterator v0.3.0
[1m[92m   Compiling[0m lazy_static v1.5.0
[1m[92m   Compiling[0m syn v3.0.9
[1m[92m   Compiling[0m syn v2.0.119
[1m[92m   Compiling[0m fnv v1.0.7
[1m[92m   Compiling[0m want v0.3.2
[1m[92m   Compiling[0m httparse v1.10.1
[1m[92m   Compiling[0m minimal-lexical v0.2.1
[1m[92m   Compiling[0m generic-array v0.14.7
[1m[92m   Compiling[0m httpdate v1.0.3
[1m[92m   Compiling[0m getrandom v0.4.3
[1m[92m   Compiling[0m crossbeam-utils v0.8.23
[1m[92m   Compiling[0m unicode-normalization v0.1.26
[1m[92m   Compiling[0m sync_wrapper v1.0.2
[1m[92m   Compiling[0m ipnet v2.12.2
[1m[92m   Compiling[0m fastrand v2.5.0
[1m[92m   Compiling[0m tower-layer v0.3.3
[1m[92m   Compiling[0m libm v0.2.16
[1m[92m   Compiling[0m base64 v0.22.1
[1m[92m   Compiling[0m rand_core v0.10.1
[1m[92m   Compiling[0m nom v7.1.3
[1m[92m   Compiling[0m hashbrown v0.13.2
[1m[92m   Compiling[0m dnssec-prover v0.6.10
[1m[92m   Compiling[0m anyhow v1.0.104
[1m[92m   Compiling[0m jobserver v0.1.35
[1m[92m   Compiling[0m getrandom v0.2.17
[1m[92m   Compiling[0m socket2 v0.6.5
[1m[92m   Compiling[0m mio v1.2.4
[1m[92m   Compiling[0m errno v0.3.14
[1m[92m   Compiling[0m rand_core v0.6.4
[1m[92m   Compiling[0m getrandom v0.3.4
[1m[92m   Compiling[0m signal-hook-registry v1.4.8
[1m[92m   Compiling[0m cc v1.11.0
[1m[92m   Compiling[0m possiblyrandom v0.2.1
[1m[92m   Compiling[0m crypto-common v0.1.7
[1m[92m   Compiling[0m block-padding v0.3.3
[1m[92m   Compiling[0m block-buffer v0.10.4
[1m[92m   Compiling[0m rand_core v0.9.5
[1m[92m   Compiling[0m const-random-macro v0.1.16
[1m[92m   Compiling[0m http-body v1.1.0
[1m[92m   Compiling[0m universal-hash v0.5.1
[1m[92m   Compiling[0m inout v0.1.4
[1m[92m   Compiling[0m aead v0.5.2
[1m[92m   Compiling[0m digest v0.10.7
[1m[92m   Compiling[0m tempfile v3.27.0
[1m[92m   Compiling[0m http-body-util v0.1.5
[1m[92m   Compiling[0m polyval v0.6.2
[1m[92m   Compiling[0m static_assertions v1.1.0
[1m[92m   Compiling[0m hex-conservative v0.1.2
[1m[92m   Compiling[0m poly1305 v0.8.0
[1m[92m   Compiling[0m template_distribution_sv2 v7.0.0
[1m[92m   Compiling[0m mining_sv2 v12.0.0
[1m[92m   Compiling[0m sha2 v0.10.9
[1m[92m   Compiling[0m ghash v0.5.1
[1m[92m   Compiling[0m framing_sv2 v8.0.0
[1m[92m   Compiling[0m crunchy v0.2.4
[1m[92m   Compiling[0m const-random v0.1.18
[1m[92m   Compiling[0m hex-conservative v0.3.2
[1m[92m   Compiling[0m bitcoin_hashes v0.13.1
[1m[92m   Compiling[0m rustc-hex v2.1.0
[1m[92m   Compiling[0m ryu v1.0.23
[1m[92m   Compiling[0m crc-catalog v2.5.0
[1m[92m   Compiling[0m parking v2.2.1
[1m[92m   Compiling[0m unicode_categories v0.1.1
[1m[92m   Compiling[0m crc v3.4.0
[1m[92m   Compiling[0m dlv-list v0.5.2
[1m[92m   Compiling[0m uint v0.10.2
[1m[92m   Compiling[0m event-listener v5.4.2
[1m[92m   Compiling[0m common_messages_sv2 v9.0.0
[1m[92m   Compiling[0m job_declaration_sv2 v10.0.0
[1m[92m   Compiling[0m extensions_sv2 v0.3.0
[1m[92m   Compiling[0m simdutf8 v0.1.5
[1m[92m   Compiling[0m core_detect v1.0.0
[1m[92m   Compiling[0m prettyplease v0.2.37
[1m[92m   Compiling[0m regex-syntax v0.8.11
[1m[92m   Compiling[0m pulldown-cmark v0.13.6
[1m[92m   Compiling[0m ucd-trie v0.1.7
[1m[92m   Compiling[0m hmac v0.12.1
[1m[92m   Compiling[0m tokio v1.54.1
[1m[92m   Compiling[0m encoding_rs v0.8.44
[1m[92m   Compiling[0m foldhash v0.1.5
[1m[92m   Compiling[0m unicode-bidi v0.3.18
[1m[92m   Compiling[0m parsers_sv2 v0.6.0
[1m[92m   Compiling[0m unicode-properties v0.1.5
[1m[92m   Compiling[0m arraydeque v0.5.1
[1m[92m   Compiling[0m unicase v2.10.0
[1m[92m   Compiling[0m hashbrown v0.15.5
[1m[92m   Compiling[0m futures-util v0.3.34
[1m[92m   Compiling[0m serde_fmt v1.1.0
[1m[92m   Compiling[0m serde_json v1.0.154
[1m[92m   Compiling[0m bitflags v2.13.2
[1m[92m   Compiling[0m uuid v1.29.0
[1m[92m   Compiling[0m hkdf v0.12.4
[1m[92m   Compiling[0m crossbeam-queue v0.3.14
[1m[92m   Compiling[0m sqlformat v0.2.6
[1m[92m   Compiling[0m stringprep v0.1.5
[1m[92m   Compiling[0m atoi v2.0.0
[1m[92m   Compiling[0m chrono v0.4.45
[1m[92m   Compiling[0m md-5 v0.10.6
[1m[92m   Compiling[0m value-bag-serde1 v1.14.1
[1m[92m   Compiling[0m value-bag v1.14.1
[1m[92m   Compiling[0m home v0.5.12
[1m[92m   Compiling[0m whoami v1.6.1
[1m[92m   Compiling[0m unicode-segmentation v1.13.3
[1m[92m   Compiling[0m fixedbitset v0.5.7
[1m[92m   Compiling[0m dotenvy v0.15.7
[1m[92m   Compiling[0m zstd-safe v8.2.0
[1m[92m   Compiling[0m petgraph v0.8.3
[1m[92m   Compiling[0m log v0.4.35
[1m[92m   Compiling[0m regex-automata v0.4.18
[1m[92m   Compiling[0m utf8parse v0.2.2
[1m[92m   Compiling[0m toml_write v0.1.2
[1m[92m   Compiling[0m convert_case v0.6.0
[1m[92m   Compiling[0m pathdiff v0.2.3
[1m[92m   Compiling[0m option-ext v0.2.0
[1m[92m   Compiling[0m winnow v0.7.15
[1m[92m   Compiling[0m multimap v0.10.1
[1m[92m   Compiling[0m anstyle-parse v1.0.0
[1m[92m   Compiling[0m dirs-sys v0.5.0
[1m[92m   Compiling[0m block-buffer v0.9.0
[1m[92m   Compiling[0m pulldown-cmark-to-cmark v22.0.3
[1m[92m   Compiling[0m digest v0.9.0
[1m[92m   Compiling[0m openssl-probe v0.2.2
[1m[92m   Compiling[0m is_terminal_polyfill v1.70.2
[1m[92m   Compiling[0m colorchoice v1.0.5
[1m[92m   Compiling[0m anstyle-query v1.1.5
[1m[92m   Compiling[0m secp256k1-sys v0.10.1
[1m[92m   Compiling[0m ring v0.17.14
[1m[92m   Compiling[0m libsqlite3-sys v0.28.0
[1m[92m   Compiling[0m stacker v0.1.25
[1m[92m   Compiling[0m secp256k1-sys v0.14.1
[1m[92m   Compiling[0m secp256k1-sys v0.9.2
[1m[92m   Compiling[0m zstd-sys v2.1.2+zstd.1.5.7
[1m[92m   Compiling[0m foldhash v0.2.0
[1m[92m   Compiling[0m anstyle v1.0.14
[1m[92m   Compiling[0m sha2 v0.9.9
[1m[92m   Compiling[0m ppv-lite86 v0.2.21
[1m[92m   Compiling[0m ahash v0.8.12
[1m[92m   Compiling[0m anstream v1.0.0
[1m[92m   Compiling[0m hashbrown v0.14.5
[1m[92m   Compiling[0m ar_archive_writer v0.5.3
[1m[92m   Compiling[0m rand_chacha v0.3.1
[1m[92m   Compiling[0m rand_chacha v0.9.0
[1m[92m   Compiling[0m hashbrown v0.16.1
[1m[92m   Compiling[0m dirs v6.0.0
[1m[92m   Compiling[0m tracing-log v0.2.0
[1m[92m   Compiling[0m concurrent-queue v2.5.0
[1m[92m   Compiling[0m rand v0.8.8
[1m[92m   Compiling[0m synstructure v0.14.0
[1m[92m   Compiling[0m rand v0.9.5
[1m[92m   Compiling[0m hashlink v0.9.1
[1m[92m   Compiling[0m ordered-multimap v0.7.3
[1m[92m   Compiling[0m hashlink v0.8.4
[1m[92m   Compiling[0m num-integer v0.1.47
[1m[92m   Compiling[0m sharded-slab v0.1.7
[1m[92m   Compiling[0m rust-ini v0.20.0
[1m[92m   Compiling[0m psm v0.1.32
[1m[92m   Compiling[0m yaml-rust2 v0.8.1
[1m[92m   Compiling[0m hex-conservative v1.3.0
[1m[92m   Compiling[0m fixed-hash v0.8.0
[1m[92m   Compiling[0m primitive-types v0.13.1
[1m[92m   Compiling[0m regex v1.13.1
[1m[92m   Compiling[0m thread_local v1.1.10
[1m[92m   Compiling[0m event-listener v2.5.3
[1m[92m   Compiling[0m winnow v1.0.4
[1m[92m   Compiling[0m tonic-build v0.14.9
[1m[92m   Compiling[0m nu-ansi-term v0.50.3
[1m[92m   Compiling[0m strsim v0.11.1
[1m[92m   Compiling[0m saa v5.6.2
[1m[92m   Compiling[0m clap_lex v1.1.1
[1m[92m   Compiling[0m async-channel v1.9.0
[1m[92m   Compiling[0m clap_builder v4.6.7
[1m[92m   Compiling[0m num-bigint v0.4.8
[1m[92m   Compiling[0m hashlink v0.11.1
[1m[92m   Compiling[0m shellexpand v3.1.2
[1m[92m   Compiling[0m bs58 v0.4.0
[1m[92m   Compiling[0m pest_stack_guard v1.0.1
[1m[92m   Compiling[0m matchers v0.2.0
[1m[92m   Compiling[0m sdd v4.8.11
[1m[92m   Compiling[0m pest v2.9.5
[1m[92m   Compiling[0m socket2 v0.5.10
[1m[92m   Compiling[0m arc-swap v1.9.3
[1m[92m   Compiling[0m cpufeatures v0.3.1
[1m[92m   Compiling[0m toml_datetime v1.1.2+spec-1.1.0
[1m[92m   Compiling[0m mime v0.3.17
[1m[92m   Compiling[0m tracing-attributes v0.1.31
[1m[92m   Compiling[0m serde_derive v1.0.229
[1m[92m   Compiling[0m tokio-macros v2.7.2
[1m[92m   Compiling[0m zeroize_derive v1.5.0
[1m[92m   Compiling[0m zerofrom-derive v0.1.8
[1m[92m   Compiling[0m yoke-derive v0.8.4
[1m[92m   Compiling[0m zerovec-derive v0.11.7
[1m[92m   Compiling[0m displaydoc v0.2.7
[1m[92m   Compiling[0m futures-macro v0.3.34
[1m[92m   Compiling[0m zeroize v1.9.1
[1m[92m   Compiling[0m async-trait v0.1.92
[1m[92m   Compiling[0m thiserror-impl v2.0.21
[1m[92m   Compiling[0m rustls-pki-types v1.15.1
[1m[92m   Compiling[0m cipher v0.4.4
[1m[92m   Compiling[0m webpki-roots v1.0.11
[1m[92m   Compiling[0m lightning-macros v0.2.1
[1m[92m   Compiling[0m thiserror-impl v1.0.69
[1m[92m   Compiling[0m trait-variant v0.1.3
[1m[92m   Compiling[0m pest_meta v2.9.5
[1m[92m   Compiling[0m aes v0.8.4
[1m[92m   Compiling[0m ctr v0.9.2
[1m[92m   Compiling[0m tracing v0.1.44
[1m[92m   Compiling[0m chacha20 v0.9.1
[1m[92m   Compiling[0m tokio-stream v0.1.19
[1m[92m   Compiling[0m zerofrom v0.1.8
[1m[92m   Compiling[0m yoke v0.8.4
[1m[92m   Compiling[0m chacha20poly1305 v0.10.1
[1m[92m   Compiling[0m pin-project-internal v1.1.13
[1m[92m   Compiling[0m secp256k1 v0.28.2
[1m[92m   Compiling[0m rustls-native-certs v0.8.5
[1m[92m   Compiling[0m toml_parser v1.1.5+spec-1.1.0
[1m[92m   Compiling[0m clap_derive v4.6.7
[1m[92m   Compiling[0m linux-raw-sys v0.12.1
[1m[92m   Compiling[0m serde_spanned v1.1.2
[1m[92m   Compiling[0m handlers_sv2 v0.6.0
[1m[92m   Compiling[0m zerovec v0.11.9
[1m[92m   Compiling[0m zerotrie v0.2.5
[1m[92m   Compiling[0m sha1_smol v1.0.1
[1m[92m   Compiling[0m aes-gcm v0.10.3
[1m[92m   Compiling[0m axum-core v0.5.6
[1m[92m   Compiling[0m buffer_sv2 v3.0.1
[1m[92m   Compiling[0m cbc v0.1.2
[1m[92m   Compiling[0m scc v3.8.8
[1m[92m   Compiling[0m chacha20 v0.10.2
[1m[92m   Compiling[0m yaml-rust2 v0.11.1
[1m[92m   Compiling[0m noise_sv2 v2.0.0
[1m[92m   Compiling[0m toml v1.1.8+spec-1.1.0
[1m[92m   Compiling[0m rustix v1.1.6
[1m[92m   Compiling[0m thiserror v2.0.21
[1m[92m   Compiling[0m pest_generator v2.9.5
[1m[92m   Compiling[0m rust-ini v0.21.3
[1m[92m   Compiling[0m serde-untagged v0.1.9
[1m[92m   Compiling[0m tinystr v0.8.5
[1m[92m   Compiling[0m potential_utf v0.1.6
[1m[92m   Compiling[0m icu_locale_core v2.4.0
[1m[92m   Compiling[0m rustls-webpki v0.103.16
[1m[92m   Compiling[0m icu_collections v2.4.0
[1m[92m   Compiling[0m codec_sv2 v7.0.0
[1m[92m   Compiling[0m serde_path_to_error v0.1.20
[1m[92m   Compiling[0m thiserror v1.0.69
[1m[92m   Compiling[0m rmp v0.8.15
[1m[92m   Compiling[0m matchit v0.8.4
[1m[92m   Compiling[0m pin-project v1.1.13
[1m[92m   Compiling[0m rand v0.10.3
[1m[92m   Compiling[0m pest_derive v2.9.5
[1m[92m   Compiling[0m serde v1.0.229
[1m[92m   Compiling[0m clap v4.6.7
[1m[92m   Compiling[0m icu_provider v2.4.0
[1m[92m   Compiling[0m icu_properties v2.4.0
[1m[92m   Compiling[0m smallvec v1.16.4
[1m[92m   Compiling[0m bitcoin_hashes v0.14.101
[1m[92m   Compiling[0m bitcoin-units v0.1.101
[1m[92m   Compiling[0m minreq v2.14.1
[1m[92m   Compiling[0m secp256k1 v0.33.1
[1m[92m   Compiling[0m either v1.19.0
[1m[92m   Compiling[0m serde_urlencoded v0.7.1
[1m[92m   Compiling[0m toml_datetime v0.6.11
[1m[92m   Compiling[0m serde_spanned v0.6.9
[1m[92m   Compiling[0m tracing-serde v0.2.0
[1m[92m   Compiling[0m ron v0.8.1
[1m[92m   Compiling[0m json5 v0.4.1
[1m[92m   Compiling[0m json5 v1.3.7
[1m[92m   Compiling[0m email_address v0.2.5
[1m[92m   Compiling[0m ron v0.12.3
[1m[92m   Compiling[0m parking_lot_core v0.9.12
[1m[92m   Compiling[0m icu_normalizer v2.4.0
[1m[92m   Compiling[0m toml_edit v0.22.27
[1m[92m   Compiling[0m itertools v0.14.0
[1m[92m   Compiling[0m jsonrpc v0.18.0
[1m[92m   Compiling[0m parking_lot v0.12.5
[1m[92m   Compiling[0m futures-executor v0.3.34
[1m[92m   Compiling[0m futures v0.3.34
[1m[92m   Compiling[0m futures-intrusive v0.5.0
[1m[92m   Compiling[0m dashmap v6.2.1
[1m[92m   Compiling[0m itertools v0.13.0
[1m[92m   Compiling[0m tracing-subscriber v0.3.23
[1m[92m   Compiling[0m asynchronous-codec v0.7.0
[1m[92m   Compiling[0m rmp-serde v1.3.1
[1m[92m   Compiling[0m bark-common v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m secp256k1 v0.29.1
[1m[92m   Compiling[0m base58ck v0.1.101
[1m[92m   Compiling[0m bip39 v2.2.2
[1m[92m   Compiling[0m idna_adapter v1.2.2
[1m[92m   Compiling[0m zstd v0.14.3
[1m[92m   Compiling[0m idna v1.1.0
[1m[92m   Compiling[0m config v0.15.27
[1m[92m   Compiling[0m url v2.5.8
[1m[92m   Compiling[0m prost-derive v0.14.4
[1m[92m   Compiling[0m sqlx-core v0.8.0
[1m[92m   Compiling[0m toml v0.8.23
[1m[92m   Compiling[0m config v0.14.1
[1m[92m   Compiling[0m prost v0.14.4
[1m[92m   Compiling[0m prost-types v0.14.4
[1m[92m   Compiling[0m sqlx-postgres v0.8.0
[1m[92m   Compiling[0m prost-build v0.14.4
[1m[92m   Compiling[0m rusqlite v0.31.0
[1m[92m   Compiling[0m tonic-prost-build v0.14.9
[1m[92m   Compiling[0m bark-server-rpc v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m tokio-util v0.7.20
[1m[92m   Compiling[0m tokio-rustls v0.26.6
[1m[92m   Compiling[0m bitreq v0.3.8
[1m[92m   Compiling[0m bdk_core v0.6.4
[1m[92m   Compiling[0m bitcoincore-rpc-json v0.19.0
[1m[92m   Compiling[0m miniscript v12.3.8
[1m[92m   Compiling[0m lightning-types v0.3.5
[1m[92m   Compiling[0m corepc-types v0.15.0
[1m[92m   Compiling[0m channels_sv2 v8.0.0
[1m[92m   Compiling[0m sv1_api v6.0.0
[1m[92m   Compiling[0m miniscript v13.2.0
[1m[92m   Compiling[0m h2 v0.4.20
[1m[92m   Compiling[0m tower v0.5.3
[1m[92m   Compiling[0m combine v4.6.9
[1m[92m   Compiling[0m zeromq v0.6.0
[1m[92m   Compiling[0m bark-runtime v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m lightning-invoice v0.34.1
[1m[92m   Compiling[0m stratum-core v0.6.0
[1m[92m   Compiling[0m lightning v0.2.7
[1m[92m   Compiling[0m tower-http v0.6.11
[1m[92m   Compiling[0m sqlx-macros-core v0.8.0
[1m[92m   Compiling[0m bdk_chain v0.23.4
[1m[92m   Compiling[0m sqlx-macros v0.8.0
[1m[92m   Compiling[0m stratum-apps v0.8.0
[1m[92m   Compiling[0m hyper v1.12.0
[1m[92m   Compiling[0m bdk_wallet v3.2.0
[1m[92m   Compiling[0m bitcoincore-rpc v0.19.0
[1m[92m   Compiling[0m redis v0.27.6
[1m[92m   Compiling[0m bdk_bitcoind_rpc v0.22.1
[1m[92m   Compiling[0m sqlx v0.8.0
[1m[92m   Compiling[0m hyper-util v0.1.21
[1m[92m   Compiling[0m hyper-rustls v0.27.10
[1m[92m   Compiling[0m hyper-timeout v0.5.2
[1m[92m   Compiling[0m axum v0.8.9
[1m[92m   Compiling[0m tonic v0.14.9
[1m[92m   Compiling[0m reqwest v0.12.28
[1m[92m   Compiling[0m esplora-client v0.12.3
[1m[92m   Compiling[0m lnurl-rs v0.10.0
[1m[92m   Compiling[0m tonic-prost v0.14.9
[1m[92m   Compiling[0m bdk_esplora v0.22.3
[1m[92m   Compiling[0m bitcoind-async-client v0.15.2
[1m[92m   Compiling[0m bark-bitcoin-ext v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m bip321 v0.4.0 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m ark-lib v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m spc-crypto v0.1.0 (/build/source/crates/spc-crypto)
[1m[92m   Compiling[0m spc-accounting v0.1.0 (/build/source/crates/spc-accounting)
[1m[92m   Compiling[0m spc-ext v0.1.0 (/build/source/crates/spc-ext)
[1m[92m   Compiling[0m spc-share-ext v0.1.0 (/build/source/crates/spc-share-ext)
[1m[92m   Compiling[0m spc-store v0.1.0 (/build/source/crates/spc-store)
[1m[92m   Compiling[0m spc-scan v0.1.0 (/build/source/crates/spc-scan)
[1m[92m   Compiling[0m bark-wallet v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m spc-pool v0.1.0 (/build/source/bins/spc-pool)
[1m[92m   Compiling[0m spc-agent v0.1.0 (/build/source/bins/spc-agent)
[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 1m 01s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m  Executable[0m unittests src/lib.rs (target/debug/deps/spc_agent-d8be6c9087a5cedb)
buildPhase completed in 1 minutes 30 seconds
Running phase: checkPhase
@nix { "action": "setPhase", "phase": "checkPhase" }
[32m.[0m[32m.[0m[32m.[0m[32m.[0m[32m.[0m[32m.[0m[32m.[0m
----------------------------------------------------------------------
Ran 7 tests in 0.001s

[32mOK[0m
[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 0.16s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m     Running[0m unittests src/lib.rs (target/debug/deps/spc_agent-d8be6c9087a5cedb)

running 7 tests
test wallet::native_audit::tests::joint_artifact::tests::wrong_descriptor_digest_fails ... ok
test wallet::native_audit::tests::joint_artifact::tests::missing_artifact_fails ... ok
test wallet::native_audit::tests::joint_artifact::tests::wrong_binary_path_fails ... ok
test wallet::native_audit::tests::joint_artifact::tests::wrong_derivation_fails ... ok
test wallet::native_audit::tests::joint_artifact::tests::unknown_schema_and_fields_fail ... ok
test wallet::native_audit::tests::joint_artifact::tests::wrong_binary_digest_fails ... ok
test wallet::native_audit::tests::joint_artifact::tests::wrong_source_fails ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 178 filtered out; finished in 0.33s

[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 0.16s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m     Running[0m unittests src/lib.rs (target/debug/deps/spc_agent-d8be6c9087a5cedb)

running 1 test
native joint results: /build/native-joint.XY1imw6e/native-joint-9XwMUr
test wallet::native_audit::tests::sealed_native_cpu_waits_for_actual_wallet_collector_commit_before_first_hash ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 184 filtered out; finished in 1.31s

[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 0.16s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 0.16s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m     Running[0m unittests src/lib.rs (target/debug/deps/spc_agent-d8be6c9087a5cedb)

running 6 tests
test wallet::native_audit::tests::startup_tests::foreign_network_namespace_is_rejected_before_token_authorization ... ignored, requires enabled user/network namespaces; explicit host gate
test wallet::native_audit::tests::startup_tests::native_bootstrap_probe ... ignored, supervised subprocess helper for native_run_token_gate
test wallet::native_audit::tests::startup_tests::opaque_marker_requires_exact_expected_digest_and_unique_bounded_environment ... ok
test wallet::native_audit::tests::startup_tests::unaudited_profile_has_no_native_marker_requirement ... ok
test wallet::native_audit::tests::startup_tests::native_private_config_requires_canonical_controller_digest ... ok
test wallet::native_audit::tests::startup_tests::exact_marker_does_not_bypass_foreign_process_config_or_registry ... ok

test result: ok. 4 passed; 0 failed; 2 ignored; 0 measured; 179 filtered out; finished in 1.16s

[1m[92m   Compiling[0m hex-conservative v0.2.3
[1m[92m   Compiling[0m slab v0.4.12
[1m[92m   Compiling[0m log v0.4.35
[1m[92m   Compiling[0m secp256k1-sys v0.10.1
[1m[92m   Compiling[0m num-traits v0.2.19
[1m[92m   Compiling[0m inout v0.1.4
[1m[92m   Compiling[0m serde v1.0.229
[1m[92m   Compiling[0m zeroize v1.9.1
[1m[92m   Compiling[0m futures-channel v0.3.34
[1m[92m   Compiling[0m zerofrom v0.1.8
[1m[92m   Compiling[0m psm v0.1.32
[1m[92m   Compiling[0m secp256k1-sys v0.9.2
[1m[92m   Compiling[0m thiserror v2.0.21
[1m[92m   Compiling[0m secp256k1-sys v0.14.1
[1m[92m   Compiling[0m ucd-trie v0.1.7
[1m[92m   Compiling[0m thiserror v1.0.69
[1m[92m   Compiling[0m handlers_sv2 v0.6.0
[1m[92m   Compiling[0m getrandom v0.4.3
[1m[92m   Compiling[0m regex-syntax v0.8.11
[1m[92m   Compiling[0m tracing v0.1.44
[1m[92m   Compiling[0m tokio-stream v0.1.19
[1m[92m   Compiling[0m base64 v0.22.1
[1m[92m   Compiling[0m yoke v0.8.4
[1m[92m   Compiling[0m cipher v0.4.4
[1m[92m   Compiling[0m bs58 v0.4.0
[1m[92m   Compiling[0m anyhow v1.0.104
[1m[92m   Compiling[0m secp256k1 v0.28.2
[1m[92m   Compiling[0m futures-util v0.3.34
[1m[92m   Compiling[0m tracing-log v0.2.0
[1m[92m   Compiling[0m tempfile v3.27.0
[1m[92m   Compiling[0m uuid v1.29.0
[1m[92m   Compiling[0m zerovec v0.11.9
[1m[92m   Compiling[0m zerotrie v0.2.5
[1m[92m   Compiling[0m aes v0.8.4
[1m[92m   Compiling[0m chacha20 v0.9.1
[1m[92m   Compiling[0m ctr v0.9.2
[1m[92m   Compiling[0m rustix v1.1.6
[1m[92m   Compiling[0m clap v4.6.7
[1m[92m   Compiling[0m sync_wrapper v1.0.2
[1m[92m   Compiling[0m axum-core v0.5.6
[1m[92m   Compiling[0m chacha20poly1305 v0.10.1
[1m[92m   Compiling[0m smallvec v1.16.4
[1m[92m   Compiling[0m bitcoin_hashes v0.14.101
[1m[92m   Compiling[0m bitcoin-units v0.1.101
[1m[92m   Compiling[0m secp256k1 v0.33.1
[1m[92m   Compiling[0m toml_datetime v0.6.11
[1m[92m   Compiling[0m serde_spanned v0.6.9
[1m[92m   Compiling[0m either v1.19.0
[1m[92m   Compiling[0m tracing-serde v0.2.0
[1m[92m   Compiling[0m ron v0.8.1
[1m[92m   Compiling[0m serde_urlencoded v0.7.1
[1m[92m   Compiling[0m noise_sv2 v2.0.0
[1m[92m   Compiling[0m stacker v0.1.25
[1m[92m   Compiling[0m toml_edit v0.22.27
[1m[92m   Compiling[0m parking_lot_core v0.9.12
[1m[92m   Compiling[0m chrono v0.4.45
[1m[92m   Compiling[0m aes-gcm v0.10.3
[1m[92m   Compiling[0m tinystr v0.8.5
[1m[92m   Compiling[0m potential_utf v0.1.6
[1m[92m   Compiling[0m pest_stack_guard v1.0.1
[1m[92m   Compiling[0m atoi v2.0.0
[1m[92m   Compiling[0m num-integer v0.1.47
[1m[92m   Compiling[0m pest v2.9.5
[1m[92m   Compiling[0m buffer_sv2 v3.0.1
[1m[92m   Compiling[0m itertools v0.13.0
[1m[92m   Compiling[0m icu_collections v2.4.0
[1m[92m   Compiling[0m parking_lot v0.12.5
[1m[92m   Compiling[0m dashmap v6.2.1
[1m[92m   Compiling[0m icu_locale_core v2.4.0
[1m[92m   Compiling[0m codec_sv2 v7.0.0
[1m[92m   Compiling[0m num-bigint v0.4.8
[1m[92m   Compiling[0m tokio v1.54.1
[1m[92m   Compiling[0m futures-intrusive v0.5.0
[1m[92m   Compiling[0m regex-automata v0.4.18
[1m[92m   Compiling[0m base58ck v0.1.101
[1m[92m   Compiling[0m secp256k1 v0.29.1
[1m[92m   Compiling[0m bip39 v2.2.2
[1m[92m   Compiling[0m icu_provider v2.4.0
[1m[92m   Compiling[0m icu_properties v2.4.0
[1m[92m   Compiling[0m icu_normalizer v2.4.0
[1m[92m   Compiling[0m pest_meta v2.9.5
[1m[92m   Compiling[0m bitcoin v0.32.102
[1m[92m   Compiling[0m toml v0.8.23
[1m[92m   Compiling[0m pest_generator v2.9.5
[1m[92m   Compiling[0m futures-executor v0.3.34
[1m[92m   Compiling[0m idna_adapter v1.2.2
[1m[92m   Compiling[0m futures v0.3.34
[1m[92m   Compiling[0m idna v1.1.0
[1m[92m   Compiling[0m url v2.5.8
[1m[92m   Compiling[0m pest_derive v2.9.5
[1m[92m   Compiling[0m json5 v0.4.1
[1m[92m   Compiling[0m sqlx-core v0.8.0
[1m[92m   Compiling[0m matchers v0.2.0
[1m[92m   Compiling[0m tracing-subscriber v0.3.23
[1m[92m   Compiling[0m config v0.14.1
[1m[92m   Compiling[0m tokio-util v0.7.20
[1m[92m   Compiling[0m hyper v1.12.0
[1m[92m   Compiling[0m tower v0.5.3
[1m[92m   Compiling[0m sqlx-postgres v0.8.0
[1m[92m   Compiling[0m combine v4.6.9
[1m[92m   Compiling[0m hyper-util v0.1.21
[1m[92m   Compiling[0m axum v0.8.9
[1m[92m   Compiling[0m lightning-types v0.3.5
[1m[92m   Compiling[0m bark-bitcoin-ext v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m sv1_api v6.0.0
[1m[92m   Compiling[0m channels_sv2 v8.0.0
[1m[92m   Compiling[0m miniscript v13.2.0
[1m[92m   Compiling[0m lightning-invoice v0.34.1
[1m[92m   Compiling[0m stratum-core v0.6.0
[1m[92m   Compiling[0m lightning v0.2.7
[1m[92m   Compiling[0m sqlx-macros-core v0.8.0
[1m[92m   Compiling[0m redis v0.27.6
[1m[92m   Compiling[0m sqlx-macros v0.8.0
[1m[92m   Compiling[0m stratum-apps v0.8.0
[1m[92m   Compiling[0m sqlx v0.8.0
[1m[92m   Compiling[0m ark-lib v0.7.1 (https://gitlab.com/ark-bitcoin/bark.git?rev=c419159fa54085572f8ca72d2ba1716556a9d3eb#c419159f)
[1m[92m   Compiling[0m spc-crypto v0.1.0 (/build/source/crates/spc-crypto)
[1m[92m   Compiling[0m spc-accounting v0.1.0 (/build/source/crates/spc-accounting)
[1m[92m   Compiling[0m spc-share-ext v0.1.0 (/build/source/crates/spc-share-ext)
[1m[92m   Compiling[0m spc-ext v0.1.0 (/build/source/crates/spc-ext)
[1m[92m   Compiling[0m spc-scan v0.1.0 (/build/source/crates/spc-scan)
[1m[92m   Compiling[0m spc-store v0.1.0 (/build/source/crates/spc-store)
[1m[92m   Compiling[0m spc-cpu-miner v0.1.0 (/build/source/bins/spc-cpu-miner)
[1m[92m   Compiling[0m spc-pool v0.1.0 (/build/source/bins/spc-pool)
[1m[92m   Compiling[0m spc-lab-txd v0.1.0 (/build/source/crates/spc-lab-txd)
[1m[92m   Compiling[0m spc-lab v0.1.0 (/build/source/crates/spc-lab)
[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 39.95s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m     Running[0m unittests src/lib.rs (target/debug/deps/spc_lab-7387fc0b4f3164a1)

running 4 tests
test wallet_bridge::native_render::run_token_tests::native_run_token_gate ... ignored, explicit same-source agent harness required; run startup-gate.py
test wallet_bridge::native_render::run_token_tests::run_token_hash_is_not_an_operator_profile_field ... ok
test wallet_bridge::native_render::tests::native_render_confines_declared_collector_and_keeps_only_public_binding ... ok
test wallet_bridge::native_render::run_token_tests::actual_engine_opaque_and_literal_markers_render_exact_digest ... ok

test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 114 filtered out; finished in 55.17s

[1m[92m    Finished[0m `test` profile [unoptimized + debuginfo] target(s) in 0.15s
[1m[33mwarning[0m: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
[1m[92mnote[0m: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
[1m[92m     Running[0m unittests src/lib.rs (target/debug/deps/spc_lab-7387fc0b4f3164a1)

running 1 test
native-startup-case {"agent_harness_sha256":"4d4adbd2806832f41da977bd8716e360eb105b50b8f6b9e2633301f46bf0d198","published_instance":{"collector_pubkey":"462779ad4aad39514614751a71085f2f10e1c7a593e4e030efb5b8721ce55b0b","config_path":"/build/native-token-f95c78a8e1457f46a78fd62f/private/wallet-id-alice/agent.toml","front_id":"front-a1","identity":"tsp1qqvdcf32k0vfxgsyet5ldt246q4jaw8scx3sysx0lnstlt6w4m5rc7qjdfdkdzdssxt9fh54wh8vsp2jdghv74kq2e9prxaxy2xnj2ng8vcl3n54a","identity_id":"id-alice","instance_id":[95,91,30,218,41,88,154,239,64,208,151,177,238,81,202,249],"pool_id":"pool-a","pool_pubkey":"531fe6068134503d2723133227c867ac8fa6c83c537e9a44c3c5bdbdcb1fe337","process":{"boot_id":"d92aebf7-86d1-4d8d-9d76-eda4768da2ce","config_sha256":[16,98,158,226,201,170,231,15,48,122,86,190,161,223,93,168,188,181,61,228,107,148,60,166,89,111,119,206,235,203,168,190],"executable_sha256":[77,74,219,210,128,104,50,244,29,169,119,189,135,22,227,96,235,16,91,80,184,246,185,226,99,51,1,244,107,240,209,152],"pid":16342,"run_token_sha256":[72,114,13,232,136,167,210,43,219,225,243,150,163,70,1,19,88,164,154,215,122,63,213,148,111,181,56,184,64,50,46,238],"start_time_ticks":83323472,"uid":1000},"run_id":"native-token-e273cb99c99681a5","schema":"spc.native-wallet-instance/1","socket_device":66306,"socket_inode":47846735,"socket_path":"/build/native-token-f95c78a8e1457f46a78fd62f/private/wallet-id-alice/state/native-audit.sock"},"scope":"actual prepared-engine renderer/Supervisor/loaded-config/native-service; fixture infrastructure","source_id":"git:98f0e6caa2482646e3c7073949f6388cd39f200b","traffic":true}
test wallet_bridge::native_render::run_token_tests::native_run_token_gate has been running for over 60 seconds
native-startup-case {"agent_harness_sha256":"4d4adbd2806832f41da977bd8716e360eb105b50b8f6b9e2633301f46bf0d198","published_instance":{"collector_pubkey":"462779ad4aad39514614751a71085f2f10e1c7a593e4e030efb5b8721ce55b0b","config_path":"/build/native-token-9f888847341746fa3173b6d4/private/wallet-id-alice/agent.toml","front_id":"front-a1","identity":"tsp1qqvdcf32k0vfxgsyet5ldt246q4jaw8scx3sysx0lnstlt6w4m5rc7qjdfdkdzdssxt9fh54wh8vsp2jdghv74kq2e9prxaxy2xnj2ng8vcl3n54a","identity_id":"id-alice","instance_id":[37,133,191,121,238,131,197,26,216,178,109,131,134,112,204,93],"pool_id":"pool-a","pool_pubkey":"531fe6068134503d2723133227c867ac8fa6c83c537e9a44c3c5bdbdcb1fe337","process":{"boot_id":"d92aebf7-86d1-4d8d-9d76-eda4768da2ce","config_sha256":[118,56,90,197,46,121,154,122,133,162,44,86,227,152,177,151,248,206,130,139,201,107,4,74,211,172,224,142,148,3,111,188],"executable_sha256":[77,74,219,210,128,104,50,244,29,169,119,189,135,22,227,96,235,16,91,80,184,246,185,226,99,51,1,244,107,240,209,152],"pid":16356,"run_token_sha256":[177,121,251,185,212,233,158,57,7,195,90,54,199,245,116,191,196,9,72,35,174,246,195,113,8,90,198,167,120,226,64,66],"start_time_ticks":83328079,"uid":1000},"run_id":"native-token-6b2c7c471d6d813c","schema":"spc.native-wallet-instance/1","socket_device":66306,"socket_inode":47846737,"socket_path":"/build/native-token-9f888847341746fa3173b6d4/private/wallet-id-alice/state/native-audit.sock"},"scope":"actual prepared-engine renderer/Supervisor/loaded-config/native-service; fixture infrastructure","source_id":"git:98f0e6caa2482646e3c7073949f6388cd39f200b","traffic":false}
test wallet_bridge::native_render::run_token_tests::native_run_token_gate ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 117 filtered out; finished in 119.71s

{"schema": "spc.native-startup-gate/1", "source_id": "git:98f0e6caa2482646e3c7073949f6388cd39f200b", "source_path": "/nix/store/6hwxw1jm24l2is2cd9mnnkxg0i0dsqf7-source", "joint_descriptor_sha256": "ab44c44bfd8613c5afc7107a8a253cc56f7731e1e154212f450386cd870e303c", "agent_harness_sha256": "4d4adbd2806832f41da977bd8716e360eb105b50b8f6b9e2633301f46bf0d198", "scope": "real engine token generation/render, Supervisor export, loaded agent config and native service publication; fixture infrastructure; no funded/full-controller acceptance"}
checkPhase completed in 3 minutes 40 seconds
Running phase: installPhase
@nix { "action": "setPhase", "phase": "installPhase" }
Running phase: fixupPhase
@nix { "action": "setPhase", "phase": "fixupPhase" }
shrinking RPATHs of ELF executables and libraries in /nix/store/d9cl1gi6gs0yv32hq3yxn9ihbkv17pl0-wallet-native-joint-0.1.0
checking for references to /build/ in /nix/store/d9cl1gi6gs0yv32hq3yxn9ihbkv17pl0-wallet-native-joint-0.1.0...
patching script interpreter paths in /nix/store/d9cl1gi6gs0yv32hq3yxn9ihbkv17pl0-wallet-native-joint-0.1.0
