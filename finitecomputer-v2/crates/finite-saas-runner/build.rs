fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/ateapi.proto");
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&["proto/ateapi.proto"], &["proto"])?;
    Ok(())
}
