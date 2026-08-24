use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use const_gen::{CompileConst, const_declaration};
use xz2::read::XzEncoder;

fn main() {
    println!("cargo:rerun-if-changed=vial.json");
    println!("cargo:rerun-if-changed=memory.x");

    generate_vial_config();

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rustc-link-arg=-Tdefmt.x");
}

fn generate_vial_config() {
    let mut content = String::new();
    File::open("vial.json")
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();

    let compact = json::stringify(json::parse(&content).unwrap());
    let mut compressed = Vec::new();
    XzEncoder::new(compact.as_bytes(), 6)
        .read_to_end(&mut compressed)
        .unwrap();

    let declarations = [
        const_declaration!(pub VIAL_KEYBOARD_DEF = compressed),
        const_declaration!(
            pub VIAL_KEYBOARD_ID = [0x52_u8, 0x65, 0x65, 0x4c, 0x52, 0x4d, 0x4b, 0x01]
        ),
    ]
    .map(|value| "#[allow(clippy::redundant_static_lifetimes)]\n".to_owned() + &value)
    .join("\n");

    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("config_generated.rs"),
        declarations,
    )
    .unwrap();
}
