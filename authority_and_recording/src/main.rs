mod authority;
mod record;
mod server;
mod types;

use std::env;
use std::path::PathBuf;
use std::process;

fn main() {
    let project_root = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().expect("cannot get cwd"));

    if !project_root.is_dir() {
        eprintln!("[arcana] Error: {:?} is not a directory", project_root);
        process::exit(1);
    }

    eprintln!("[arcana] Authority & Record starting for {:?}", project_root);

    let mut srv = match server::Server::new(project_root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[arcana] Failed to start: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = srv.run() {
        eprintln!("[arcana] Server error: {}", e);
        process::exit(1);
    }
}
