fn main() {
    slint_build::compile("ui/main.slint").expect("Slint UI compilation failed");
    if cfg!(target_os = "windows") {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource.set("FileDescription", "Mickey RAR");
        resource.set("ProductName", "Mickey RAR");
        resource.set("CompanyName", "Michael Maged");
        resource.set("OriginalFilename", "mickey-rar.exe");
        if std::env::var("CARGO_BIN_NAME").as_deref() == Ok("mickey-rar-installer") {
            resource.set_manifest(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="requireAdministrator" uiAccess="false" /></requestedPrivileges></security>
  </trustInfo>
</assembly>"#);
        }
        resource.compile().expect("Windows icon resource compilation failed");
    }
}
