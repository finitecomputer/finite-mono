//! Disposable interoperability signer; no production CLI surface changes.
use std::io::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = finite_identity::IdentityPaths::resolve()?;
    let identity = finite_identity::FiniteIdentity::load_or_generate(&paths, "sites-vercel-poc")?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        println!(
            "{}",
            serde_json::json!({"pubkey": identity.public_key_hex(), "npub": identity.npub()})
        );
        return Ok(());
    }
    if args.len() != 2 {
        return Err("usage: sites_poc_sign [METHOD URL] (body on stdin)".into());
    }
    let mut body = Vec::new();
    io::stdin().take(3 * 1024 * 1024).read_to_end(&mut body)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let payload = if args[0] == "GET" {
        None
    } else {
        Some(body.as_slice())
    };
    let authorization = finitesites_proto::nip98::build_auth_header(
        &identity.expose_secret_bytes(),
        &args[1],
        &args[0],
        payload,
        now,
    )?;
    println!(
        "{}",
        serde_json::json!({"authorization":authorization,"body":String::from_utf8(body)?})
    );
    Ok(())
}
