fn main() {
    // Pass linker flags specifically for the MSVC target
    println!("cargo:rustc-link-arg=/ENTRY:_start");
    println!("cargo:rustc-link-arg=/DYNAMICBASE:NO");
    println!("cargo:rustc-link-arg=/FIXED");
    println!("cargo:rustc-link-arg=/NODEFAULTLIB");
    println!("cargo:rustc-link-arg=/SUBSYSTEM:CONSOLE");
    println!("cargo:rerun-if-changed=build.rs");
}
