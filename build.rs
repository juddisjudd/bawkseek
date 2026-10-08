fn main() {
    println!("cargo:rerun-if-changed=assets/brand/bawkseek.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/brand/bawkseek.ico")
            .set("FileDescription", "bawkseek")
            .set("ProductName", "bawkseek");
        resource.compile().expect("compile Windows resources");
    }
}
