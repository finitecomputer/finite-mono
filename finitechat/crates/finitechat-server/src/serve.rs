use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use crate::{HttpServerState, http_router};

type ServeResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone)]
pub struct ChatServeOptions {
    pub addr: String,
    pub sqlite_path: Option<String>,
}

pub async fn serve_chat(options: ChatServeOptions) -> ServeResult<()> {
    let addr = options.addr.parse::<SocketAddr>()?;
    let state = match options.sqlite_path {
        Some(path) => {
            create_sqlite_parent_dir(&path)?;
            HttpServerState::from_sqlite_path(path)?
        }
        None => HttpServerState::default(),
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("finitechat-server: listening on http://{addr}");
    axum::serve(listener, http_router(state)).await?;
    Ok(())
}

fn create_sqlite_parent_dir(path: &str) -> ServeResult<()> {
    let path = Path::new(path);
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::create_sqlite_parent_dir;

    #[test]
    fn sqlite_parent_dir_is_created_before_open() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp
            .path()
            .join(".state")
            .join("nested")
            .join("finitechat.sqlite3");

        create_sqlite_parent_dir(db_path.to_str().expect("utf8 path")).expect("create parent dir");

        assert!(db_path.parent().expect("parent").is_dir());
    }
}
