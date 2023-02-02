fn main() {
    zigc::Build::new()
        .optimiziation(zigc::Opt::Fast)
        .as_static()
        .file("./src/main.zig")
        .finish();
}
