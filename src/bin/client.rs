//! Thin desktop entry point. Android uses the `cdylib` NativeActivity entry
//! exported by `xenomit_client::client`.

fn main() -> anyhow::Result<()> {
    xenomit_client::client::run_cli()
}
