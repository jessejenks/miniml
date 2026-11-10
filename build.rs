fn main() {
    println!("cargo:rerun-if-changed=runtime/runtime.c");
    cc::Build::new()
        .file("runtime/runtime.c")
        .define("MINIML_TEST", None)
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .warnings_into_errors(true)
        .compile("miniml_runtime");
}
