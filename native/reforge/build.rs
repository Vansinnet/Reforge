fn main() {
    println!("cargo:rerun-if-changed=reforge.rc");
    let version = env!("CARGO_PKG_VERSION");
    let mut parts: Vec<u32> = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(3)
        .map(|s| s.parse().unwrap())
        .collect();
    parts.resize(3, 0);
    let numeric = format!("{},{},{},0", parts[0], parts[1], parts[2]);
    let macros = [
        format!("REFORGE_VERSION_NUM={numeric}"),
        format!("REFORGE_VERSION_STR=\\\"{version}\\\""),
    ];
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("reforge.rc", &macros).manifest_optional().unwrap();
    }
}
