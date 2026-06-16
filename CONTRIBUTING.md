# Contributing

Thanks for your interest in the Touhou 6 macOS/Web reimplementation. This is a
clean-room fan engine — contributions of code, format research, and bug reports
are all welcome.

Please read the [Code of Conduct](CODE_OF_CONDUCT.md) first. The hard rules
there are not optional.

## The one rule that matters most

**Never commit, attach, or link copyrighted game data.** No `.DAT` files,
sprites, music, or executables. This repository is code only. Patches that add
copyrighted assets will be rejected outright. All testing assumes you supply
your own legally purchased copy of *the Embodiment of Scarlet Devil* (v1.02h).

## Getting set up

Install the Rust toolchain from [rustup.rs](https://rustup.rs), then:

```sh
git clone https://github.com/TheScreamer74/Touhou6-Mac.git
cd Touhou6-Mac
cargo run -p th06 -- --game-dir "/path/to/your/th06/folder"
```

Web (WebAssembly) build:

```sh
wasm-pack build crates/game --release --target web --out-dir ../../web/pkg
cd web && python3 -m http.server 8080   # http://localhost:8080
```

## Project layout

```
crates/
├── formats   PBG3 archives · ANM sprites · ECL · STD · MSG
├── engine    wgpu sprite renderer · fixed-timestep loop · input · rodio audio
└── game      ANM VM · ECL VM · title menu · stage logic · background · scenes
```

When in doubt about original game behavior, the references in the README
([happyhavoc/th06](https://github.com/happyhavoc/th06),
[thtk](https://github.com/thpatch/thtk),
[PyTouhou](https://pytouhou.linkmauve.fr)) are the ground truth.

## Before you open a pull request

Run these and make sure they pass:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

## Pull request guidelines

- Keep changes focused — one logical change per PR.
- Match the surrounding code style; do not reformat unrelated code.
- Reference the issue you are fixing, if any.
- For behavior that aims to match the original game, say how you verified it
  (compared against the decompilation, observed in-game, etc.).
- Describe what you changed and why in the PR description.

## Reporting bugs and requesting features

Use the issue templates. For gameplay-accuracy bugs, include the stage,
character/shot type, and what the original game does differently.

For security or copyright concerns, do **not** open a public issue — use a
[private security advisory](https://github.com/TheScreamer74/Touhou6-Mac/security/advisories/new)
instead.

## License

By contributing, you agree your contributions are licensed under the project's
[MIT License](LICENSE).
