#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(not(debug_assertions), not(feature = "custom-protocol"), not(test)))]
compile_error!(
    "UniLoader release builds require the `custom-protocol` feature; use `pnpm run build:app`, `pnpm run build`, or pass `--features custom-protocol`."
);

fn main() {
    uniloader_lib::run()
}
