fn main() {
    println!("cargo:rerun-if-changed=ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set("FileDescription", "Timeline — local table viewer");
        resource.set("ProductName", "Timeline");
        let mut icons: Vec<_> = std::fs::read_dir("ico")
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ico")))
            .collect();
        icons.sort();
        if let Some(icon) = icons.first() {
            resource.set_icon(icon.to_str().expect("Unicode icon path"));
        } else {
            println!("cargo:warning=No .ico file in ico/. Executable will use its default icon.");
        }
        resource
            .compile()
            .expect("Compile Windows executable resources");
    }
}
