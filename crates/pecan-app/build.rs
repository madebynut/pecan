//! Embed Windows identity and the multi-resolution icon in the executable.
fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/pecan.ico");
    #[cfg(target_os = "windows")]
    {
        // GPUI supplies the PerMonitorV2 manifest; only add our icon and version resource.
        winresource::WindowsResource::new()
            .set_icon("../../assets/brand/pecan.ico")
            .set("ProductName", "Pecan")
            .set("FileDescription", "Pecan · 碧果")
            .set("OriginalFilename", "Pecan.exe")
            .set(
                "LegalCopyright",
                "Copyright © 2026 madebynut and Pecan contributors",
            )
            .compile()
            .expect("embed Pecan Windows icon and version information");
    }
}
