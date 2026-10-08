# Type Foundry

Product code is this folder. Facts and the session log live in `../Agent/CONTEXT.md`. The command contract is `../documents/api.md`.

Mutations go through `foundry-api`. Do not add a second way to change a font.

Do not copy Shift source or another foundry's outlines into this repo. Local font authoring only. Do not upload fonts, outlines, reference images, or prompts.

Cargo writes build output to `App/target` unless your user Cargo config or `CARGO_TARGET_DIR` sets another place. Run the check before calling the engine done. On Windows: `powershell -ExecutionPolicy Bypass -File scripts/check.ps1`. The bypass is for machines whose execution policy rejects unsigned scripts. On macOS and Linux, run its three commands: `cargo fmt --all -- --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
