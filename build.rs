use std::{env, path::PathBuf};

fn main() {
    let header = env::var("PORCUPINE_HEADER")
        .unwrap_or_else(|_| format!("{}/.local/include/pv_porcupine.h", env::var("HOME").unwrap()));

    println!("cargo:rerun-if-changed={}", header);
    println!("cargo:rustc-link-search=native={}/.local/lib", env::var("HOME").unwrap());
    println!("cargo:rustc-link-lib=dylib=pv_porcupine");

    let bindings = bindgen::Builder::default()
        .header(header)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate Porcupine bindings");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("porcupine_bindings.rs"))
        .expect("Couldn't write bindings");
}
