fn main() {
    for asset in [
        "src/product-client.css",
        "src/product-client.html",
        "src/product-client.js",
        "src/smoke-ui.css",
        "src/smoke-ui.html",
        "src/smoke-ui.js",
    ] {
        println!("cargo:rerun-if-changed={asset}");
    }
}
