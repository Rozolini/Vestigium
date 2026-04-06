fn main() {
    // Define the custom entry point and Windows subsystem.
    println!("cargo:rustc-link-arg-bin=test_target=/ENTRY:vm_entry");
    println!("cargo:rustc-link-arg-bin=test_target=/SUBSYSTEM:CONSOLE");

    // Link essential MSVC runtime libraries.
    println!("cargo:rustc-link-arg-bin=test_target=vcruntime.lib");
    println!("cargo:rustc-link-arg-bin=test_target=ucrt.lib");

    // Force a fixed load address mapping to Guest Physical Address (GPA) 0x100000.
    // Disabling ASLR and relocations ensures a strictly deterministic memory layout.
    println!("cargo:rustc-link-arg-bin=test_target=/BASE:0x100000");
    println!("cargo:rustc-link-arg-bin=test_target=/DYNAMICBASE:NO");
    println!("cargo:rustc-link-arg-bin=test_target=/FIXED");
}
