# Public native retention consumer

`native_retention.py` consumes final producer source `8259f003...`, DTO hash
`017fa69f...`, from `native-evidence-source-r1`. It uses the existing native
request/Approved codec and signature verifier. Producer compile/test admission
is separate; source-only DTO conformance does not admit a live binary.

Discovery derives only `<run>/private/native-evidence/<declared miner>` and
`process-<observed pid>-<observed start ticks>`. It walks directory descriptors
without following symlinks, enforces ownership/modes, rejects hardlinked or
nonregular records and checks directory/file identity through reads. Config,
manifest, event handle, request digest, payload SHA256, record hash, run,
process/token/config, peer generation, wallet instance and both epochs join.
Manifest/record/locator fields and versions are closed. The optional locator
parser recognizes only the final `spc.native-evidence-locator/1`; discovery
does not invent an observed locator from a manifest.

Records are bounded before JSON/hex decoding: 2,138,112 bytes per record,
16,384 records, 256 MiB per session and 32 sessions in this consumer. Larger
producer-configured capacities are explicitly unavailable to this consumer.
Pending files and missing/truncated/pruned records do not establish capture
completion. Receipt sequence holes may be failed, already charged attempts;
no missing record is synthesized to fill them.

The exact lowercase-hex UTF8 payloads exclude U32LE transport lengths. The
consumer restores those four framing bytes solely from the checked byte
length and uses the existing decoder. It verifies the outer/inner signatures
once at the retained verification time and checks the committed event time
against that candidate's original expiry and verification interval.

NativeCapture retains these originals and supplies verified exchange objects
to native_observer with `require_durable=True`. An arbitrary tuple/dictionary
or event hash cannot satisfy that live path. The actual SourceAuditCommitted
DurableHandle must match the read bytes; activation and each worker's first
hash follow in the bound controller event prefix. Retained exchanges without
activation are reported explicitly, including observed cancellation/Unknown.

After miner drain the native prefix and record census are sealed. Final audit
uses the pre-stop process/wallet/registry bindings and retained files, without
requiring dead processes to respond. Later miner events or record changes
block the final verdict. The controller can append unrelated events; these
are not relabelled as native EOF or permission to ignore a late miner event.
