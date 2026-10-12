# REA tooling repair — 2026-10-12

The scoped REA checkout remains pinned to
`morluto/rea@b3dec005bcb89b5afe6e0aa41e5251f8a3a31de7`.
The reusable [patch](rea-th06-limits.patch) changes local tooling only; it has
not been submitted to the upstream REA repository.

## WASM capacity

The full-game binary is 9,395,830 bytes; WABT expands it into 121,217,568 bytes
of WAT. REA's original 8 MiB binary and 32 MiB subprocess output limits rejected
these representations. The patch uses independent 16 MiB / 256 MiB budgets and
retains the 30-second subprocess deadline, cancellation, private snapshots,
digest verification and explicit failures on truncated output. The report now
derives its stated budget from the actual configuration.

The music-loop build also passes (9,394,203 binary bytes, 121,677,166 WAT bytes).
Both outputs match independent `wasm2wat` output byte for byte. A synthetic
12 MiB custom-section module exercises both formerly failing size boundaries.
The generated fixture contains no game assets. The 21 adapter checks and 42
real WABT CLI/MCP checks pass. [Receipts](rea-capacity-2026-10-12.json) preserve
artifact and WAT hashes, evidence IDs and selected-tool coverage.

Apply and rebuild in a separate checkout with REA's locked dependencies and
its supported Node toolchain:

```sh
git checkout b3dec005bcb89b5afe6e0aa41e5251f8a3a31de7
git apply /path/to/Touhou6/docs/rea-th06-limits.patch
npm ci
npm run build:cached
```

Use caller-supplied WABT 1.0.42, as required by REA. The portable regression
uses the public CLI and does not execute the selected WASM:

```sh
python3 scripts/verify_rea_capacity.py \
  --rea /path/to/rea/scripts/rea.mjs \
  --wabt-bin /path/to/wabt-1.0.42/bin \
  --wasm /path/to/th06_bg.wasm \
  --glue /path/to/th06.js \
  --report /tmp/rea-capacity.json
```

Full intermediate evidence is temporary; the saved receipt contains hashes,
sizes and IDs. Captured WAT remains a decoded representation. It does not
recover Rust variable names, resolve every JavaScript association or establish
Windows gameplay equivalence. Larger artifacts can still exceed the bounded
budgets.

## Final fidelity build

The final `fidelity-20261012-3ac820702f24` module also passes the repaired REA
adapter: 9,577,298 input bytes, all 13 sections and 124,898,828 bytes of complete
WAT, matching independent WABT output byte for byte. Its SHA-256 is
`3ac820702f24fdeca7dbdf17061f14d00392f10cc525334b42a2ec31aa3f95e6`.
The generated 12 MiB capacity fixture passes again. The macOS process-ownership
helper requires the system compiler outside the restricted sandbox; rerunning
there succeeded without changing the configured REA limits.
[Final receipt](rea-fidelity-capacity-2026-10-12.json).

## Ghidra verifier

The snapshot lifecycle verifier required an obsolete sentence in an error
message. The patch removes that wording assertion while retaining the stable
`evidence_integrity_mismatch` code, recovery action, no-write and active-session
checks. This repairs the verifier without weakening snapshot integrity checks.

The supplied Ghidra 12.1.4 distribution also lacked its macOS ARM64 GNU
demangler binary. It was compiled locally from the bundled C source using the
existing compiler, entirely inside the temporary Ghidra installation:

```sh
cc -arch arm64 -std=gnu17 -O2 -DHAVE_STDLIB_H -DHAVE_STRING_H \
  -I GPL/DemanglerGnu/src/demangler_gnu_v2_41/headers \
  GPL/DemanglerGnu/src/demangler_gnu_v2_41/c/*.c \
  -o GPL/DemanglerGnu/os/mac_arm_64/demangler_gnu_v2_41
```

Its smoke test demangles `_ZN3rea5scope5entryEi` as `rea::scope::entry(int)`.
The real host-native verification lane now passes all four fixture variants:
370 successful calls, 52 checked rejections, CLI/MCP parity, snapshot lifecycle,
namespace annotations, entry aliases, cancellation and complete cleanup. This
verifies Ghidra 12.1.4 on this Mac ARM64, not every supported architecture.
The [repair receipt](rea-ghidra-2026-10-12.json) records component and fixture hashes.
No personal REA/MCP configuration was changed and no Windows target was run.
