fn main() {
    // generate_context! embeds the icons when the crate compiles, and
    // tauri_build does not list them as inputs. Without this line, a
    // change to an icon file leaves the old icon in the dev binary.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
